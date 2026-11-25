//! Runs the real binary on a real (tiny) project. Needs python on PATH, as CI has.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const BIN: &str = env!("CARGO_BIN_EXE_shrink-anything");

const LIB: &str = "import os\n\n\ndef used():\n    return 1\n\n\ndef dead_one():\n    return os.getcwd()\n\n\ndef secretly_used():\n    return 3\n\n\ndef dead_two():\n    return dead_one()\n";

/// One test reaches secretly_used through a name it reads from a data file: nothing in the code mentions it.
const TESTS: &str = "import unittest\n\nimport lib\n\n\nclass T(unittest.TestCase):\n    def test_used(self):\n        self.assertEqual(lib.used(), 1)\n\n    def test_by_name(self):\n        name = open('names.dat').read().strip()\n        self.assertEqual(getattr(lib, name)(), 3)\n";

struct Project(PathBuf);

impl Project {
    fn new(tag: &str) -> Project {
        let dir = std::env::temp_dir().join(format!("shrink-anything-e2e-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("lib.py"), LIB).unwrap();
        fs::write(dir.join("test_lib.py"), TESTS).unwrap();
        fs::write(dir.join("names.dat"), "secretly_used\n").unwrap();
        Project(dir)
    }

    fn run(&self, args: &[&str]) -> (bool, String) {
        let out = Command::new(BIN).arg("-C").arg(&self.0).args(args).env("NO_COLOR", "1").output().unwrap();
        (
            out.status.success(),
            format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)),
        )
    }

    fn read(&self, rel: &str) -> String {
        fs::read_to_string(self.0.join(rel)).unwrap()
    }
}

impl Drop for Project {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn python() -> bool {
    Command::new("python")
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

#[test]
fn cut_keeps_what_passes_puts_back_what_breaks_and_undo_restores() {
    if !python() {
        eprintln!("skipped: no python");
        return;
    }
    let p = Project::new("cut");

    let (ok, json) = p.run(&["scan", "--json"]);
    assert!(ok, "{json}");
    let scan: serde_json::Value = serde_json::from_str(&json).unwrap();
    let names: Vec<&str> = scan["candidates"].as_array().unwrap().iter().map(|c| c["name"].as_str().unwrap()).collect();
    assert!(names.contains(&"dead_two") && names.contains(&"secretly_used"), "{names:?}");
    assert!(
        !names.contains(&"used") && !names.contains(&"dead_one"),
        "dead_one is still called by dead_two: {names:?}"
    );

    let (ok, out) = p.run(&["cut", "--", "python", "-m", "unittest"]);
    assert!(ok, "{out}");
    let lib = p.read("lib.py");
    assert!(!lib.contains("dead_two"), "{lib}");
    assert!(!lib.contains("dead_one"), "orphaned by the first round, cut in the second:\n{lib}");
    assert!(lib.contains("def secretly_used"), "the test that calls it by name put it back:\n{lib}");
    assert!(!lib.contains("import os"), "its only user went:\n{lib}");
    assert!(lib.contains("def used"));
    assert!(out.contains("put back") && out.contains("2 tests still run and pass"), "{out}");
    assert_eq!(p.read("test_lib.py"), TESTS, "tests are never touched");

    let (ok, out) = p.run(&["verify"]);
    assert!(ok, "{out}");
    let (ok, out) = p.run(&["report"]);
    assert!(ok && out.contains("| lines | 29 | 18 | **-38%** |"), "{out}");
    assert!(p.read(".shrink/report.svg").starts_with("<svg"));

    let (ok, out) = p.run(&["undo"]);
    assert!(ok, "{out}");
    assert_eq!(p.read("lib.py"), LIB, "undo is exact");
}

#[test]
fn a_check_that_already_fails_proves_nothing() {
    if !python() {
        return;
    }
    let p = Project::new("red");
    fs::write(p.0.join("names.dat"), "no_such_function\n").unwrap();
    let (ok, out) = p.run(&["cut", "--", "python", "-m", "unittest"]);
    assert!(!ok && out.contains("fails before any change"), "{out}");
    assert_eq!(p.read("lib.py"), LIB);
}

#[test]
fn verify_fails_when_tests_stop_running() {
    if !python() {
        return;
    }
    let p = Project::new("fewer");
    let (ok, out) = p.run(&["cut", "--", "python", "-m", "unittest"]);
    assert!(ok, "{out}");
    // someone "fixes" a failure by deleting a test
    fs::write(p.0.join("test_lib.py"), TESTS.split("    def test_by_name").next().unwrap()).unwrap();
    let (ok, out) = p.run(&["verify"]);
    assert!(!ok && out.contains("1 tests, 2 before"), "{out}");
}

/// Issue 1: a called function whose name has a non-ASCII letter was reported dead and cut.
#[test]
fn names_with_non_ascii_letters_are_not_cut_while_called() {
    let p = Project::new("unicode");
    // the а in dаta_format is Cyrillic U+0430
    fs::write(
        p.0.join("app.py"),
        "def dаta_format(record):\n    return {\"id\": record}\n\n\ndef build(record):\n    return dаta_format(record)\n\n\ndef ünused():\n    return 0\n",
    )
    .unwrap();
    fs::write(
        p.0.join("names.py"),
        "def résumé_title(name):\n    return name.title()\n\n\ndef show(name):\n    return résumé_title(name)\n",
    )
    .unwrap();
    let (ok, json) = p.run(&["scan", "--all", "--json"]);
    assert!(ok, "{json}");
    let scan: serde_json::Value = serde_json::from_str(&json).unwrap();
    let names: Vec<&str> = scan["candidates"].as_array().unwrap().iter().map(|c| c["name"].as_str().unwrap()).collect();
    assert!(!names.contains(&"dаta_format") && !names.contains(&"résumé_title"), "still called: {names:?}");
    assert!(names.contains(&"build") && names.contains(&"show"), "nothing calls these: {names:?}");
}

fn mcp(dir: &Path, messages: &[serde_json::Value]) -> Vec<serde_json::Value> {
    let mut child =
        Command::new(BIN).arg("mcp").current_dir(dir).stdin(Stdio::piped()).stdout(Stdio::piped()).spawn().unwrap();
    {
        let mut stdin = child.stdin.take().unwrap();
        for m in messages {
            writeln!(stdin, "{m}").unwrap();
        }
    }
    let out = child.wait_with_output().unwrap();
    String::from_utf8_lossy(&out.stdout).lines().map(|l| serde_json::from_str(l).unwrap()).collect()
}

#[test]
fn mcp_scans_and_cuts() {
    if !python() {
        return;
    }
    let p = Project::new("mcp");
    let call = |id: u32, name: &str, args: serde_json::Value| serde_json::json!({ "jsonrpc": "2.0", "id": id, "method": "tools/call", "params": { "name": name, "arguments": args } });
    let replies = mcp(
        &p.0,
        &[
            serde_json::json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {} }),
            call(2, "shrink_scan", serde_json::json!({})),
            call(3, "shrink_cut", serde_json::json!({ "check": ["python", "-m", "unittest"] })),
            call(4, "shrink_verify", serde_json::json!({})),
        ],
    );
    assert_eq!(replies.len(), 4, "only protocol messages on stdout: {replies:?}");
    assert_eq!(replies[0]["result"]["serverInfo"]["name"], "shrink-anything");
    let text = |i: usize| -> serde_json::Value {
        serde_json::from_str(replies[i]["result"]["content"][0]["text"].as_str().unwrap()).unwrap()
    };
    assert!(text(1)["candidates"].as_array().unwrap().len() >= 2);
    let cut = text(2);
    assert_eq!(cut["changed"], true, "{cut}");
    assert_eq!(cut["put_back"][0]["id"], "code:lib.py:secretly_used");
    assert_eq!(text(3)["ok"], true);
    assert!(!p.read("lib.py").contains("dead_two"));
}
