//! What shrink-anything remembers between commands, in .shrink/: the check command, the numbers
//! before the first cut, every run, and the original of every file a run touched.

use crate::candidate::CandKind;
use crate::scan::Metrics;
use crate::util;
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Baseline {
    pub metrics: Metrics,
    /// The test count the check printed, if it prints one.
    pub tests: Option<u64>,
    /// Bytes on disk of the --size paths.
    pub size: Option<u64>,
    pub at: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CutRecord {
    pub kind: CandKind,
    pub name: String,
    pub path: String,
    pub lines: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Touched {
    pub path: String,
    /// The file was there before the run.
    pub existed: bool,
    /// Hash of the file after the run. None: the run deleted it.
    pub after: Option<u64>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Run {
    pub id: u32,
    pub at: u64,
    /// "cut" or "agent".
    pub kind: String,
    pub cuts: Vec<CutRecord>,
    pub before: Metrics,
    pub after: Metrics,
    pub tests: Option<u64>,
    pub size: Option<u64>,
    pub files: Vec<Touched>,
    #[serde(default)]
    pub undone: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct State {
    /// The command that proves the project still works.
    #[serde(default)]
    pub check: Vec<String>,
    /// A shell line run after every edit, before the check (a formatter, an import fixer).
    #[serde(default)]
    pub fix: Option<String>,
    /// Build outputs to weigh before and after.
    #[serde(default)]
    pub size: Vec<String>,
    #[serde(default)]
    pub baseline: Option<Baseline>,
    #[serde(default)]
    pub runs: Vec<Run>,
}

pub struct Ctx {
    pub root: PathBuf,
    pub dir: PathBuf,
    pub state: State,
}

impl Ctx {
    pub fn open(root: &Path) -> Result<Ctx> {
        if !root.is_dir() {
            bail!("{} isn't a folder", root.display());
        }
        let dir = root.join(".shrink");
        let path = dir.join("state.json");
        let state = match fs::read_to_string(&path) {
            Ok(text) => serde_json::from_str(&text)
                .with_context(|| format!("{} is damaged; delete it to start over", path.display()))?,
            Err(_) => State::default(),
        };
        Ok(Ctx { root: root.to_path_buf(), dir, state })
    }

    pub fn save(&self) -> Result<()> {
        fs::create_dir_all(&self.dir)?;
        let ignore = self.dir.join(".gitignore");
        if !ignore.exists() {
            fs::write(ignore, "*\n")?;
        }
        fs::write(self.dir.join("state.json"), serde_json::to_string_pretty(&self.state)? + "\n")?;
        Ok(())
    }

    pub fn next_run(&self) -> u32 {
        self.state.runs.iter().map(|r| r.id).max().unwrap_or(0) + 1
    }

    fn undo_dir(&self, run: u32) -> PathBuf {
        self.dir.join("undo").join(run.to_string())
    }

    /// Bytes on disk of the --size paths, if any are set.
    pub fn measure_size(&self) -> Option<u64> {
        (!self.state.size.is_empty()).then(|| self.state.size.iter().map(|p| util::disk_size(&self.root.join(p))).sum())
    }

    /// The runs that are still in effect, oldest first.
    pub fn live_runs(&self) -> impl Iterator<Item = &Run> {
        self.state.runs.iter().filter(|r| !r.undone)
    }
}

/// Keeps the original of every file a run changes, so the run can be taken back.
pub struct Recorder {
    root: PathBuf,
    dir: PathBuf,
    touched: BTreeMap<String, bool>,
}

impl Recorder {
    pub fn new(ctx: &Ctx, run: u32) -> Recorder {
        Recorder { root: ctx.root.clone(), dir: ctx.undo_dir(run), touched: BTreeMap::new() }
    }

    /// Call before the first change to a file.
    pub fn backup(&mut self, rel: &str) -> Result<()> {
        if self.touched.contains_key(rel) {
            return Ok(());
        }
        let abs = self.root.join(rel);
        let existed = abs.is_file();
        if existed {
            let to = self.dir.join(rel);
            fs::create_dir_all(to.parent().context("backup path")?)?;
            fs::copy(&abs, &to).with_context(|| format!("couldn't back up {rel}"))?;
        }
        self.touched.insert(rel.to_string(), existed);
        Ok(())
    }

    /// Records a file whose original text is already in hand (the agent changed it before we could copy it).
    pub fn backup_text(&mut self, rel: &str, original: Option<&str>) -> Result<()> {
        if self.touched.contains_key(rel) {
            return Ok(());
        }
        if let Some(text) = original {
            let to = self.dir.join(rel);
            fs::create_dir_all(to.parent().context("backup path")?)?;
            fs::write(to, text)?;
        }
        self.touched.insert(rel.to_string(), original.is_some());
        Ok(())
    }

    /// Puts files back as they were before the run.
    pub fn restore(&self, only: Option<&[&str]>) -> Result<()> {
        for (rel, existed) in &self.touched {
            if only.is_some_and(|only| !only.contains(&rel.as_str())) {
                continue;
            }
            restore_file(&self.root, &self.dir, rel, *existed)?;
        }
        Ok(())
    }

    /// The files that really changed, with what they hash to now.
    pub fn finish(self) -> Vec<Touched> {
        let mut out = Vec::new();
        for (rel, existed) in &self.touched {
            let now = fs::read(self.root.join(rel)).ok();
            let before = fs::read(self.dir.join(rel)).ok();
            if now == before {
                continue;
            }
            out.push(Touched { path: rel.clone(), existed: *existed, after: now.map(|b| util::hash(&b)) });
        }
        out
    }
}

fn restore_file(root: &Path, undo: &Path, rel: &str, existed: bool) -> Result<()> {
    let abs = root.join(rel);
    if existed {
        if let Some(parent) = abs.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::copy(undo.join(rel), &abs).with_context(|| format!("couldn't restore {rel}"))?;
    } else if abs.is_file() {
        fs::remove_file(&abs)?;
    }
    Ok(())
}

/// Takes back the last run (or every run). Refuses if a file changed since, unless forced.
pub fn undo(ctx: &mut Ctx, all: bool, force: bool) -> Result<Vec<Run>> {
    let mut ids: Vec<u32> = ctx.live_runs().map(|r| r.id).collect();
    ids.reverse();
    if !all {
        ids.truncate(1);
    }
    let mut undone = Vec::new();
    for id in ids {
        let run = ctx.state.runs.iter().find(|r| r.id == id).context("run")?.clone();
        let changed: Vec<&str> = run
            .files
            .iter()
            .filter(|t| fs::read(ctx.root.join(&t.path)).ok().map(|b| util::hash(&b)) != t.after)
            .map(|t| t.path.as_str())
            .collect();
        if !changed.is_empty() && !force {
            bail!(
                "these files changed after run {id}, so undoing would lose that work: {}\nPass --force to restore them anyway.",
                changed.join(", ")
            );
        }
        let dir = ctx.undo_dir(id);
        for t in &run.files {
            restore_file(&ctx.root, &dir, &t.path, t.existed)?;
        }
        if let Some(r) = ctx.state.runs.iter_mut().find(|r| r.id == id) {
            r.undone = true;
        }
        undone.push(run);
    }
    if ctx.live_runs().next().is_none() {
        ctx.state.baseline = None;
    }
    ctx.save()?;
    Ok(undone)
}

#[cfg(test)]
pub mod tests {
    use super::*;

    /// A fresh folder under the system temp dir, removed when dropped.
    pub struct TempDir(pub PathBuf);

    impl TempDir {
        pub fn new(tag: &str) -> TempDir {
            let dir =
                std::env::temp_dir().join(format!("shrink-anything-{tag}-{}-{}", std::process::id(), util::now()));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).unwrap();
            TempDir(dir)
        }

        pub fn write(&self, rel: &str, text: &str) {
            let p = self.0.join(rel);
            fs::create_dir_all(p.parent().unwrap()).unwrap();
            fs::write(p, text).unwrap();
        }

        pub fn read(&self, rel: &str) -> String {
            fs::read_to_string(self.0.join(rel)).unwrap_or_default()
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn a_run_can_be_taken_back() {
        let tmp = TempDir::new("undo");
        tmp.write("a.py", "one\n");
        tmp.write("b.py", "two\n");
        let mut ctx = Ctx::open(&tmp.0).unwrap();
        let mut rec = Recorder::new(&ctx, 1);
        rec.backup("a.py").unwrap();
        rec.backup("b.py").unwrap();
        rec.backup("new.py").unwrap();
        tmp.write("a.py", "changed\n");
        fs::remove_file(tmp.0.join("b.py")).unwrap();
        tmp.write("new.py", "made by the run\n");
        let files = rec.finish();
        assert_eq!(files.len(), 3);
        let m = Metrics::default();
        ctx.state.runs.push(Run {
            id: 1,
            at: 0,
            kind: "cut".into(),
            cuts: vec![],
            before: m,
            after: m,
            tests: None,
            size: None,
            files,
            undone: false,
        });

        tmp.write("a.py", "edited by hand\n");
        assert!(undo(&mut ctx, false, false).is_err(), "a hand edit blocks undo");
        tmp.write("a.py", "changed\n");
        assert_eq!(undo(&mut ctx, false, false).unwrap().len(), 1);
        assert_eq!(tmp.read("a.py"), "one\n");
        assert_eq!(tmp.read("b.py"), "two\n");
        assert!(!tmp.0.join("new.py").exists());
        assert!(undo(&mut ctx, false, false).unwrap().is_empty(), "nothing left to undo");
    }
}
