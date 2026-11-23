//! The part that needs judgment: writes the task for a coding agent, runs the agent, and keeps
//! its work only if the check still passes and no test was touched.

use crate::candidate::Tier;
use crate::scan::{self, Metrics, Scan};
use crate::state::{Baseline, Ctx, Recorder, Run};
use crate::walk;
use crate::{check, cut, ui, util};
use anyhow::{Context, Result, bail, ensure};
use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::fs::{self, File};
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::Duration;

/// Presets for agents we know; anything else is run as given, with the prompt on stdin.
pub fn argv(spec: &str) -> Vec<String> {
    let preset: &[&str] = match spec {
        "claude" => &[
            "claude",
            "-p",
            "--permission-mode",
            "acceptEdits",
            "--allowedTools",
            "Read Edit Write Glob Grep Bash(shrink-anything verify:*) Bash(git diff:*) Bash(git status:*)",
        ],
        "codex" => &["codex", "exec", "--full-auto", "-"],
        _ => &[],
    };
    if preset.is_empty() {
        spec.split_whitespace().map(String::from).collect()
    } else {
        preset.iter().map(|s| s.to_string()).collect()
    }
}

fn spawn(argv: &[String], dir: &Path, log: &File) -> std::io::Result<Child> {
    let mut cmd = if cfg!(windows) && util::which(&argv[0]).is_some_and(|p| p.extension().is_some_and(|e| e != "exe")) {
        // npm-installed agents are .cmd shims on Windows
        let mut c = Command::new("cmd");
        c.arg("/C").args(argv);
        c
    } else {
        let mut c = Command::new(&argv[0]);
        c.args(&argv[1..]);
        c
    };
    cmd.current_dir(dir)
        .env_remove("CLAUDECODE")
        .stdin(Stdio::piped())
        .stdout(log.try_clone()?)
        .stderr(log.try_clone()?)
        .spawn()
}

fn run(spec: &str, dir: &Path, prompt: &str, log_path: &Path) -> Result<()> {
    let argv = argv(spec);
    ensure!(!argv.is_empty(), "empty --agent");
    let log = File::create(log_path)?;
    let mut child = spawn(&argv, dir, &log)
        .with_context(|| format!("couldn't start the agent ({}); is it installed and on PATH?", argv[0]))?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(prompt.as_bytes())?;
    }
    let status = child.wait()?;
    let text = fs::read_to_string(log_path).unwrap_or_default();
    ensure!(status.success(), "the agent exited with {status}. Log: {}\n{}", log_path.display(), util::tail(&text, 15));
    Ok(())
}

/// The task, as markdown. None when there's nothing left that needs judgment.
pub fn task(scan: &Scan, check: &[String]) -> Option<String> {
    let risky: Vec<_> = scan.tier(Tier::Risky).collect();
    if scan.duplicates.is_empty() && risky.is_empty() {
        return None;
    }
    let verify = if check.is_empty() {
        "`shrink-anything verify -- <the test command>`".to_string()
    } else {
        "`shrink-anything verify`".to_string()
    };
    let mut md = String::new();
    let _ = writeln!(md, "# Shrink task\n");
    let _ = writeln!(
        md,
        "shrink-anything has already removed what it could prove dead on its own. What is left needs judgment."
    );
    let _ = writeln!(md, "Make this codebase smaller without changing what it does.\n");
    let _ = writeln!(md, "## Rules\n");
    let _ = writeln!(md, "- Never edit, delete or skip a test. The check fails if fewer tests run than before.");
    let _ = writeln!(md, "- Behaviour stays the same: no new features, no rewrites for style.");
    let _ = writeln!(md, "- Delete rather than rewrite. The smallest diff that removes the code is the right one.");
    let _ = writeln!(md, "- After each group of changes run {verify}. If it fails, fix the change or put it back.");
    let _ = writeln!(md, "- Leave anything you aren't sure about, and say so.\n");
    if !scan.duplicates.is_empty() {
        let _ = writeln!(
            md,
            "## Duplicates ({} groups, about {} lines to win)\n",
            scan.duplicates.len(),
            scan.duplicate_lines()
        );
        let _ = writeln!(md, "The same code in more than one place. Keep one copy and make the others use it.\n");
        for group in scan.duplicates.iter().take(40) {
            let copies: Vec<String> = group
                .copies
                .iter()
                .map(|c| {
                    if c.start == 0 {
                        format!("`{}`", c.path)
                    } else {
                        format!("`{}:{}-{}` ({})", c.path, c.start, c.end, c.name)
                    }
                })
                .collect();
            if group.kind == "asset" {
                let _ = writeln!(
                    md,
                    "- the same file, {} × {}: {}",
                    util::bytes(group.bytes),
                    group.copies.len(),
                    copies.join(", ")
                );
            } else {
                let _ = writeln!(md, "- {} lines × {}: {}", group.lines, group.copies.len(), copies.join(", "));
            }
        }
        md.push('\n');
    }
    if !risky.is_empty() {
        let lines: usize = risky.iter().map(|c| c.lines).sum();
        let _ = writeln!(md, "## Probably dead ({}, about {lines} lines)\n", risky.len());
        let _ = writeln!(
            md,
            "Nothing mentions these, but something could reach them without naming them. For each one, look for"
        );
        let _ = writeln!(
            md,
            "that way in: a framework convention, reflection, a public API that users of this project call."
        );
        let _ = writeln!(md, "Delete it only if there is none.\n");
        for c in risky.iter().take(80) {
            let at = if c.line == 0 { c.path.clone() } else { format!("{}:{}", c.path, c.line) };
            let _ = writeln!(md, "- `{}` at `{at}` ({} lines): {}", c.name, c.lines, c.why);
        }
        if risky.len() > 80 {
            let _ = writeln!(md, "- and {} more: `shrink-anything scan --all`", risky.len() - 80);
        }
        md.push('\n');
    }
    let _ = writeln!(md, "When you are done, reply with one line per change: what went, and why that was safe.");
    Some(md)
}

/// Writes .shrink/TASK.md. None when there's nothing to do.
pub fn write_task(ctx: &Ctx, scan: &Scan) -> Result<Option<PathBuf>> {
    let Some(md) = task(scan, &ctx.state.check) else { return Ok(None) };
    ctx.save()?;
    let path = ctx.dir.join("TASK.md");
    fs::write(&path, md)?;
    Ok(Some(path))
}

pub struct AgentOutcome {
    pub before: Metrics,
    pub after: Metrics,
    /// Files the agent changed, added or deleted.
    pub files: usize,
    pub tests: Option<u64>,
    /// Why the agent's work was thrown away. None: it was kept (or it changed nothing).
    pub rejected: Option<String>,
    pub run: Option<u32>,
}

type Snapshot = BTreeMap<String, (String, bool)>;

fn snapshot(root: &Path) -> Result<Snapshot> {
    Ok(walk::walk(root)?.files.into_iter().map(|f| (f.rel, (f.text, f.test))).collect())
}

/// What the agent did: (path, original text if the file existed, is a test).
fn changes(before: &Snapshot, after: &Snapshot) -> Vec<(String, Option<String>, bool)> {
    let mut out = Vec::new();
    for (path, (text, test)) in before {
        if after.get(path).map(|(t, _)| t) != Some(text) {
            out.push((path.clone(), Some(text.clone()), *test));
        }
    }
    for (path, (_, test)) in after {
        if !before.contains_key(path) {
            out.push((path.clone(), None, *test));
        }
    }
    out
}

fn put_back(root: &Path, changed: &[(String, Option<String>, bool)]) -> Result<()> {
    for (path, original, _) in changed {
        let abs = root.join(path);
        match original {
            Some(text) => {
                if let Some(parent) = abs.parent() {
                    fs::create_dir_all(parent)?;
                }
                fs::write(&abs, text)?;
            }
            None => {
                let _ = fs::remove_file(&abs);
            }
        }
    }
    Ok(())
}

/// Hands the task to an agent and keeps its work only if the check agrees.
pub fn work(ctx: &mut Ctx, agent: &str, attempts: usize, library: Option<bool>) -> Result<AgentOutcome> {
    let scanned = scan::scan(&ctx.root, library)?;
    let before = scanned.metrics;
    if write_task(ctx, &scanned)?.is_none() {
        ui::ok("nothing left that needs an agent");
        return Ok(AgentOutcome { before, after: before, files: 0, tests: None, rejected: None, run: None });
    }
    let base = cut::baseline(ctx)?;
    let size_before = ctx.measure_size();
    let original = snapshot(&ctx.root)?;
    let mut feedback = String::new();
    let mut rejected = None;
    for attempt in 1..=attempts.max(1) {
        ui::step(format!(
            "{agent} is working on .shrink/TASK.md{}",
            if attempt > 1 { format!(" (attempt {attempt})") } else { String::new() }
        ));
        let prompt = format!("Read .shrink/TASK.md in this folder and do what it says.{feedback}");
        if let Err(e) = run(agent, &ctx.root, &prompt, &ctx.dir.join("agent.log")) {
            put_back(&ctx.root, &changes(&original, &snapshot(&ctx.root)?))?;
            return Err(e);
        }
        let changed = changes(&original, &snapshot(&ctx.root)?);
        if changed.is_empty() {
            ui::ok("the agent found nothing it could safely remove");
            return Ok(AgentOutcome { before, after: before, files: 0, tests: base.tests, rejected: None, run: None });
        }
        let tests: Vec<&str> = changed.iter().filter(|c| c.2).map(|c| c.0.as_str()).collect();
        let problem = if !tests.is_empty() {
            Some(format!("tests were changed, which isn't allowed: {}", tests.join(", ")))
        } else if let Some((path, error)) = changed
            .iter()
            .filter(|c| ctx.root.join(&c.0).is_file())
            .find_map(|c| check::syntax_error(&ctx.root, &c.0).map(|e| (c.0.clone(), e)))
        {
            Some(format!("{path} no longer parses:\n{error}"))
        } else {
            let out = check::run(
                &ctx.root,
                &ctx.state.check,
                Duration::from_secs_f64(base.secs * 5.0 + 60.0),
                &ctx.dir.join("check.log"),
            )?;
            if out.timed_out {
                Some("the check hung".to_string())
            } else if !out.ok {
                Some(format!("the check failed:\n{}", util::tail(&out.output, 25)))
            } else if base.tests.is_some_and(|b| out.tests.unwrap_or(0) < b) {
                Some(format!("only {} of {} tests still ran", out.tests.unwrap_or(0), base.tests.unwrap_or(0)))
            } else {
                None
            }
        };
        let Some(problem) = problem else {
            let run_id = ctx.next_run();
            let mut rec = Recorder::new(ctx, run_id);
            for (path, text, _) in &changed {
                rec.backup_text(path, text.as_deref())?;
            }
            let after = scan::scan(&ctx.root, library)?.metrics;
            if ctx.state.baseline.is_none() {
                ctx.state.baseline =
                    Some(Baseline { metrics: before, tests: base.tests, size: size_before, at: util::now() });
            }
            let size = ctx.measure_size();
            ctx.state.runs.push(Run {
                id: run_id,
                at: util::now(),
                kind: "agent".into(),
                cuts: Vec::new(),
                before,
                after,
                tests: base.tests,
                size,
                files: rec.finish(),
                undone: false,
            });
            ctx.save()?;
            return Ok(AgentOutcome {
                before,
                after,
                files: changed.len(),
                tests: base.tests,
                rejected: None,
                run: Some(run_id),
            });
        };
        ui::warn(problem.lines().next().unwrap_or_default());
        feedback = format!(
            "\n\nYour last attempt was not accepted, and the files are as you left them. Fix this, or put the change back:\n{problem}"
        );
        rejected = Some(problem);
    }
    put_back(&ctx.root, &changes(&original, &snapshot(&ctx.root)?))?;
    if rejected.is_none() {
        bail!("the agent didn't run");
    }
    Ok(AgentOutcome { before, after: before, files: 0, tests: base.tests, rejected, run: None })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presets_and_custom() {
        assert_eq!(argv("claude")[0], "claude");
        assert_eq!(argv("codex"), ["codex", "exec", "--full-auto", "-"]);
        assert_eq!(argv("aider --yes"), ["aider", "--yes"]);
    }

    #[test]
    fn sees_what_changed() {
        let mut before = Snapshot::new();
        before.insert("a.py".into(), ("1".into(), false));
        before.insert("b.py".into(), ("2".into(), false));
        before.insert("tests/t.py".into(), ("3".into(), true));
        let mut after = before.clone();
        after.insert("a.py".into(), ("changed".into(), false));
        after.remove("b.py");
        after.insert("c.py".into(), ("new".into(), false));
        let got = changes(&before, &after);
        assert_eq!(
            got.iter().map(|c| (c.0.as_str(), c.1.is_some())).collect::<Vec<_>>(),
            [("a.py", true), ("b.py", true), ("c.py", false)]
        );
    }
}
