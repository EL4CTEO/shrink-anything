//! Runs the project's own check (tests, a build, anything that exits non-zero when it's broken)
//! and reads what it can from the output.

use crate::lang;
use crate::util;
use anyhow::{Context, Result};
use regex::Regex;
use std::collections::HashMap;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::process::{Child, Stdio};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

pub struct Outcome {
    pub ok: bool,
    pub timed_out: bool,
    pub secs: f64,
    /// How many tests the output says passed. None when it doesn't say.
    pub tests: Option<u64>,
    pub output: String,
}

impl Outcome {
    /// The last line that says something, for a one-line reason.
    pub fn last_line(&self) -> String {
        let line = self.output.lines().rev().map(str::trim).find(|l| !l.is_empty()).unwrap_or("no output");
        line.chars().take(110).collect()
    }
}

fn strip_ansi(text: &str) -> String {
    static ANSI: OnceLock<Regex> = OnceLock::new();
    ANSI.get_or_init(|| Regex::new(r"\x1b\[[0-9;?]*[A-Za-z]").expect("pattern")).replace_all(text, "").into_owned()
}

/// Adds up every "N passed"-style number in the output. The same formula runs before and after a
/// cut, so what matters is that the number doesn't drop: a test that stops running would hide a break.
pub fn count_tests(output: &str) -> Option<u64> {
    static PATTERNS: OnceLock<Vec<Regex>> = OnceLock::new();
    let patterns = PATTERNS.get_or_init(|| {
        [
            r"(?i)\b(\d+)\s+(?:tests?\s+)?(?:passed|passing)\b", // pytest, cargo, jest, vitest, mocha
            r"(?i)\bpass(?:ed)?\s*[:=]?\s+(\d+)\b",              // node --test, dotnet
            r"\bRan (\d+) tests?\b",                             // unittest
            r"\bTests run:\s*(\d+)",                             // maven, gradle
            r"\b(\d+) examples?, 0 failures",                    // rspec
            r"\bOK \((\d+) tests?",                              // phpunit
            r"\b(\d+) runs, \d+ assertions",                     // minitest
            r"(?m)^ok\s+\S+\s+(?:\(cached\)|[\d.]+m?s)()",       // go test: one per package
            r"(?m)^--- PASS:()",                                 // go test -v
        ]
        .iter()
        .map(|p| Regex::new(p).expect("pattern"))
        .collect()
    });
    let text = strip_ansi(output);
    let mut total = 0u64;
    let mut any = false;
    for re in patterns {
        for c in re.captures_iter(&text) {
            any = true;
            total += c.get(1).and_then(|m| m.as_str().parse::<u64>().ok()).unwrap_or(1);
        }
    }
    any.then_some(total)
}

fn kill(child: &mut Child) {
    if cfg!(windows) {
        // the check is usually a shell that started the real work: take the whole tree down
        let _ = std::process::Command::new("taskkill")
            .args(["/PID", &child.id().to_string(), "/T", "/F"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
    let _ = child.kill();
    let _ = child.wait();
}

/// Runs argv in dir, with stdout and stderr going to log.
pub fn run(dir: &Path, argv: &[String], timeout: Duration, log: &Path) -> Result<Outcome> {
    if let Some(parent) = log.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let file = File::create(log)?;
    let start = Instant::now();
    let mut child = util::command(argv)
        .current_dir(dir)
        .env("CI", "true")
        .env_remove("CLAUDECODE")
        .stdin(Stdio::null())
        .stdout(file.try_clone()?)
        .stderr(file)
        .spawn()
        .with_context(|| format!("couldn't start {}; is it installed and on PATH?", argv[0]))?;
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break Some(status);
        }
        if start.elapsed() > timeout {
            kill(&mut child);
            break None;
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    let output = String::from_utf8_lossy(&std::fs::read(log).unwrap_or_default()).into_owned();
    Ok(Outcome {
        ok: status.is_some_and(|s| s.success()),
        timed_out: status.is_none(),
        secs: start.elapsed().as_secs_f64(),
        tests: count_tests(&output),
        output,
    })
}

fn program(alternatives: &str) -> Option<PathBuf> {
    static FOUND: OnceLock<Mutex<HashMap<String, Option<PathBuf>>>> = OnceLock::new();
    let mut found = FOUND.get_or_init(Default::default).lock().unwrap_or_else(|e| e.into_inner());
    found.entry(alternatives.to_string()).or_insert_with(|| alternatives.split('|').find_map(util::which)).clone()
}

/// The language's own parser for one file, when it's installed.
pub fn syntax_tool(rel: &str) -> Option<(PathBuf, &'static [&'static str])> {
    let spec = lang::for_path(rel)?.spec;
    let (first, rest) = spec.syntax.split_first()?;
    if spec.name == "JavaScript" && rel.ends_with(".jsx") {
        return None; // node doesn't parse JSX
    }
    Some((program(first)?, rest))
}

/// What the parser says is wrong with a file. None: it parses, or there's no parser to ask.
pub fn syntax_error(root: &Path, rel: &str) -> Option<String> {
    let (program, args) = syntax_tool(rel)?;
    let out =
        std::process::Command::new(program).args(args).arg(rel).current_dir(root).stdin(Stdio::null()).output().ok()?;
    if out.status.success() {
        return None;
    }
    let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    Some(util::tail(&text, 3))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_tests_in_every_dialect() {
        assert_eq!(count_tests("===== 12 passed, 1 skipped in 0.3s ====="), Some(12));
        assert_eq!(
            count_tests("test result: ok. 26 passed; 0 failed\n\ntest result: ok. 4 passed; 0 failed"),
            Some(30)
        );
        assert_eq!(count_tests("Tests:       5 passed, 5 total"), Some(5));
        assert_eq!(count_tests("\x1b[32m  14 passing\x1b[0m (9ms)"), Some(14));
        assert_eq!(count_tests("# tests 7\n# pass 7\n# fail 0"), Some(7));
        assert_eq!(count_tests("Ran 9 tests in 0.01s\n\nOK"), Some(9));
        assert_eq!(count_tests("ok  \texample.com/a\t0.21s\nok  \texample.com/b\t(cached)"), Some(2));
        assert_eq!(count_tests("OK (3 tests, 5 assertions)"), Some(3));
        assert_eq!(count_tests("8 examples, 0 failures"), Some(8));
        assert_eq!(count_tests("Compiling...\nFinished"), None);
    }

    #[test]
    fn runs_and_times_out() {
        let dir = std::env::temp_dir();
        let log = dir.join(format!("shrink-anything-check-{}.log", std::process::id()));
        let ok = run(&dir, &["echo 3 passed".to_string()], Duration::from_secs(20), &log).unwrap();
        assert!(ok.ok && ok.tests == Some(3), "{}", ok.output);
        let bad = run(&dir, &["exit 3".to_string()], Duration::from_secs(20), &log).unwrap();
        assert!(!bad.ok && !bad.timed_out);
        let sleep = if cfg!(windows) { "ping -n 20 127.0.0.1" } else { "sleep 20" };
        let slow = run(&dir, &[sleep.to_string()], Duration::from_millis(300), &log).unwrap();
        assert!(slow.timed_out && !slow.ok && slow.secs < 10.0);
        let _ = std::fs::remove_file(log);
    }
}
