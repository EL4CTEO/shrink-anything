//! The cut: apply candidates, run the project's own check, keep what passes, put back what doesn't.
//!
//! Candidates go in as one batch. If the check fails the batch is split in two and each half is
//! tried again, down to the single cut that breaks it. Every state left on disk has passed the check.

use crate::candidate::{CandKind, Candidate, Eco, Edit, Tier};
use crate::check;
use crate::find::deps;
use crate::lang;
use crate::scan::{self, Metrics};
use crate::state::{Baseline, Ctx, CutRecord, Recorder, Run};
use crate::{apply, ui, util};
use anyhow::{Context, Result, bail, ensure};
use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

pub struct Opts {
    /// Also cut what something could reach without naming it (public members, exports of a library).
    pub aggressive: bool,
    /// Limit to these kinds. Empty: all.
    pub only: Vec<CandKind>,
    /// Stop trying after this many runs of the check.
    pub max_runs: usize,
    /// Cutting can orphan more code: scan and cut again this many times.
    pub rounds: usize,
    pub library: Option<bool>,
}

impl Default for Opts {
    fn default() -> Self {
        Opts { aggressive: false, only: Vec::new(), max_runs: 40, rounds: 4, library: None }
    }
}

pub struct PutBack {
    pub candidate: Candidate,
    pub reason: String,
}

pub struct Outcome {
    pub before: Metrics,
    pub after: Metrics,
    pub kept: Vec<Candidate>,
    pub put_back: Vec<PutBack>,
    /// Candidates the run budget didn't reach.
    pub untested: usize,
    pub check_runs: usize,
    pub tests: Option<u64>,
    pub size_before: Option<u64>,
    pub size_after: Option<u64>,
    /// Requirements go mod tidy dropped.
    pub tidied: usize,
    pub run: Option<u32>,
}

/// Runs the check on the project as it is. A check that fails now can't prove anything later.
pub fn baseline(ctx: &Ctx) -> Result<check::Outcome> {
    ensure!(
        !ctx.state.check.is_empty(),
        "no check command yet. Give one after --, for example: shrink-anything cut -- npm test"
    );
    ui::step(format!("checking the project as it is: {}", ui::bold(ctx.state.check.join(" "))));
    let out = check::run(&ctx.root, &ctx.state.check, Duration::from_secs(3600), &ctx.dir.join("check.log"))?;
    if !out.ok {
        bail!("the check fails before any change, so it can't prove anything:\n{}", util::tail(&out.output, 15));
    }
    match out.tests {
        Some(n) => ui::ok(format!("passes in {} ({n} tests)", util::secs(out.secs))),
        None => {
            ui::ok(format!("passes in {}", util::secs(out.secs)));
            ui::warn("the check prints no test count, so the proof is its exit code alone");
        }
    }
    Ok(out)
}

/// What a file holds at some point of the cut.
#[derive(Clone, PartialEq)]
enum Content {
    Text(String),
    /// Not text (an image, a font): it can be deleted and brought back, not edited.
    Binary,
    Gone,
}

/// The files a set of candidates touches, as they were when the round started, and as they are on disk now.
struct Workspace {
    root: PathBuf,
    original: BTreeMap<String, Content>,
    disk: BTreeMap<String, Content>,
}

impl Workspace {
    fn new(root: &Path, candidates: &[Candidate]) -> Result<Workspace> {
        let mut original = BTreeMap::new();
        for path in candidates.iter().flat_map(|c| c.edits.iter()).map(Edit::path) {
            if !original.contains_key(path) {
                let bytes = fs::read(root.join(path)).with_context(|| format!("couldn't read {path}"))?;
                original.insert(path.to_string(), String::from_utf8(bytes).map_or(Content::Binary, Content::Text));
            }
        }
        let disk = original.clone();
        Ok(Workspace { root: root.to_path_buf(), original, disk })
    }

    /// Every file as it should be with these candidates cut.
    fn render(&self, active: &[&Candidate]) -> Result<BTreeMap<String, Content>> {
        let mut out = BTreeMap::new();
        for (path, original) in &self.original {
            let mut deleted = false;
            let mut ranges = Vec::new();
            let mut dependencies: Vec<(Eco, String, String)> = Vec::new();
            for edit in active.iter().flat_map(|c| c.edits.iter()).filter(|e| e.path() == path) {
                match edit {
                    Edit::Delete { .. } => deleted = true,
                    Edit::Remove { start, end, .. } => ranges.push((*start, *end)),
                    Edit::Dep { eco, section, name, .. } => dependencies.push((*eco, section.clone(), name.clone())),
                }
            }
            let content = match original {
                _ if deleted => Content::Gone,
                Content::Text(text) => {
                    Content::Text(apply::render(text, lang::for_path(path), &ranges, &dependencies)?)
                }
                other if ranges.is_empty() && dependencies.is_empty() => other.clone(),
                _ => bail!("{path} isn't text"),
            };
            out.insert(path.clone(), content);
        }
        Ok(out)
    }

    /// Makes the disk match these candidates. Returns the paths it wrote or deleted.
    fn apply(&mut self, active: &[&Candidate], rec: &mut Recorder) -> Result<Vec<String>> {
        let mut changed = Vec::new();
        for (path, want) in self.render(active)? {
            if self.disk.get(&path) == Some(&want) {
                continue;
            }
            rec.backup(&path)?;
            let abs = self.root.join(&path);
            match &want {
                Content::Text(text) => fs::write(&abs, text).with_context(|| format!("couldn't write {path}"))?,
                // only ever deleted, so the copy the recorder took is the file
                Content::Binary => rec.restore(Some(&[path.as_str()]))?,
                Content::Gone => {
                    if abs.exists() {
                        fs::remove_file(&abs).with_context(|| format!("couldn't delete {path}"))?;
                    }
                }
            }
            self.disk.insert(path.clone(), want);
            changed.push(path);
        }
        Ok(changed)
    }
}

/// The check, with its budget and what it has to match.
struct Gate<'a> {
    ctx: &'a Ctx,
    timeout: Duration,
    base_tests: Option<u64>,
    runs: usize,
    max_runs: usize,
    failed: bool,
    untested: usize,
    last_tests: Option<u64>,
}

impl Gate<'_> {
    /// Why the project is broken right now. None: it's fine.
    fn broken(&mut self, changed: &[String]) -> Result<Option<String>> {
        let root = &self.ctx.root;
        for path in changed {
            // keep node_modules in step with package.json, so the check sees what a fresh install would
            if let Some((argv, dir)) = deps::sync_command(root, path) {
                let out = check::run(&dir, &argv, Duration::from_secs(900), &self.ctx.dir.join("install.log"))?;
                if !out.ok {
                    return Ok(Some(format!("{} failed: {}", argv.join(" "), out.last_line())));
                }
            }
        }
        for path in changed.iter().filter(|p| root.join(p).is_file()) {
            if let Some(error) = check::syntax_error(root, path) {
                let line = error.lines().last().unwrap_or_default().trim().to_string();
                return Ok(Some(format!("{path} no longer parses: {line}")));
            }
        }
        if let Some(fix) = &self.ctx.state.fix {
            let out = check::run(root, &[format!("{fix} ")], Duration::from_secs(900), &self.ctx.dir.join("fix.log"))?;
            if !out.ok {
                return Ok(Some(format!("the fix command failed: {}", out.last_line())));
            }
        }
        self.runs += 1;
        let out = check::run(root, &self.ctx.state.check, self.timeout, &self.ctx.dir.join("check.log"))?;
        let reason = if out.timed_out {
            Some("the check hung".to_string())
        } else if !out.ok {
            Some(format!("the check failed: {}", out.last_line()))
        } else {
            match (self.base_tests, out.tests) {
                (Some(base), now) if now.unwrap_or(0) < base => {
                    Some(format!("only {} of {base} tests still ran", now.unwrap_or(0)))
                }
                _ => None,
            }
        };
        self.failed |= reason.is_some();
        if reason.is_none() {
            self.last_tests = out.tests;
        }
        Ok(reason)
    }
}

/// Finds the biggest subset of batch that passes on top of kept, and leaves the disk at exactly that.
fn bisect(
    ws: &mut Workspace,
    candidates: &[Candidate],
    kept: &mut Vec<usize>,
    batch: Vec<usize>,
    gate: &mut Gate,
    rec: &mut Recorder,
    put_back: &mut Vec<PutBack>,
) -> Result<()> {
    let mut stack = vec![batch];
    let mut dirty = false;
    while let Some(batch) = stack.pop() {
        if gate.runs >= gate.max_runs {
            gate.untested += batch.len();
            continue;
        }
        let active: Vec<&Candidate> = kept.iter().chain(&batch).map(|i| &candidates[*i]).collect();
        let changed = match ws.apply(&active, rec) {
            Ok(changed) => changed,
            Err(e) if batch.len() == 1 => {
                put_back.push(PutBack { candidate: candidates[batch[0]].clone(), reason: format!("{e:#}") });
                continue;
            }
            Err(_) => {
                let (a, b) = batch.split_at(batch.len() / 2);
                stack.push(b.to_vec());
                stack.push(a.to_vec());
                continue;
            }
        };
        match gate.broken(&changed)? {
            None => {
                kept.extend(&batch);
                dirty = false;
            }
            Some(reason) => {
                dirty = true;
                if batch.len() == 1 {
                    put_back.push(PutBack { candidate: candidates[batch[0]].clone(), reason });
                } else {
                    let (a, b) = batch.split_at(batch.len() / 2);
                    stack.push(b.to_vec());
                    stack.push(a.to_vec());
                }
            }
        }
    }
    if dirty {
        let active: Vec<&Candidate> = kept.iter().map(|i| &candidates[*i]).collect();
        let changed = ws.apply(&active, rec)?;
        for path in &changed {
            if let Some((argv, dir)) = deps::sync_command(&ws.root, path) {
                check::run(&dir, &argv, Duration::from_secs(900), &gate.ctx.dir.join("install.log"))?;
            }
        }
    }
    Ok(())
}

/// Go ships its own unused-dependency remover. Returns how many requirements it dropped.
fn go_mod_tidy(gate: &mut Gate, rec: &mut Recorder) -> Result<usize> {
    let root = &gate.ctx.root;
    if !root.join("go.mod").is_file() || util::which("go").is_none() {
        return Ok(0);
    }
    let requires = |text: &str| text.lines().filter(|l| l.contains(" v") && !l.trim_start().starts_with("go ")).count();
    let before = fs::read_to_string(root.join("go.mod"))?;
    rec.backup("go.mod")?;
    rec.backup("go.sum")?;
    let argv = ["go".to_string(), "mod".to_string(), "tidy".to_string()];
    let tidy = check::run(root, &argv, Duration::from_secs(600), &gate.ctx.dir.join("install.log"))?;
    let after = fs::read_to_string(root.join("go.mod")).unwrap_or_default();
    if !tidy.ok || after == before {
        rec.restore(Some(&["go.mod", "go.sum"]))?;
        return Ok(0);
    }
    if gate.broken(&[])?.is_some() {
        rec.restore(Some(&["go.mod", "go.sum"]))?;
        return Ok(0);
    }
    Ok(requires(&before).saturating_sub(requires(&after)))
}

pub fn cut(ctx: &mut Ctx, opts: &Opts) -> Result<Outcome> {
    ctx.save()?;
    let base = baseline(ctx)?;
    let size_before = ctx.measure_size();
    let mut scanned = scan::scan(&ctx.root, opts.library)?;
    let before = scanned.metrics;
    let run_id = ctx.next_run();
    let mut rec = Recorder::new(ctx, run_id);
    let mut gate = Gate {
        ctx,
        timeout: Duration::from_secs_f64(base.secs * 5.0 + 60.0),
        base_tests: base.tests,
        runs: 0,
        max_runs: opts.max_runs,
        failed: false,
        untested: 0,
        last_tests: base.tests,
    };
    let wanted = |c: &Candidate| {
        (opts.aggressive || c.tier == Tier::Safe) && (opts.only.is_empty() || opts.only.contains(&c.kind))
    };
    let mut kept_all: Vec<Candidate> = Vec::new();
    let mut put_back: Vec<PutBack> = Vec::new();
    let mut rejected: HashSet<String> = HashSet::new();
    for round in 1..=opts.rounds {
        let candidates: Vec<Candidate> =
            scanned.candidates.iter().filter(|c| wanted(c) && !rejected.contains(&c.id)).cloned().collect();
        if candidates.is_empty() {
            break;
        }
        let lines: usize = candidates.iter().map(|c| c.lines).sum();
        let bytes: u64 = candidates.iter().map(|c| c.bytes).sum();
        let things = if candidates.len() == 1 { "1 thing".to_string() } else { format!("{} things", candidates.len()) };
        let mut what = format!("{} lines", util::thousands(lines as u64));
        if bytes > 0 {
            what.push_str(&format!(", {}", util::bytes(bytes)));
        }
        ui::step(format!("round {round}: cutting {things} ({what}) and running the check"));
        let mut ws = Workspace::new(&gate.ctx.root, &candidates)?;
        let mut kept: Vec<usize> = Vec::new();
        // dependencies first: each try may reinstall, so they get their own batch
        let (dependencies, rest): (Vec<usize>, Vec<usize>) =
            (0..candidates.len()).partition(|i| candidates[*i].kind == CandKind::Dep);
        let already = put_back.len();
        for batch in [dependencies, rest] {
            if !batch.is_empty() {
                bisect(&mut ws, &candidates, &mut kept, batch, &mut gate, &mut rec, &mut put_back)?;
            }
        }
        rejected.extend(put_back[already..].iter().map(|p| p.candidate.id.clone()));
        let failed = put_back.len() - already;
        if failed > 0 {
            ui::line(format!("  {} kept, {} put back", ui::green(kept.len()), ui::yellow(failed)));
        } else {
            ui::line(format!("  {} kept", ui::green(kept.len())));
        }
        if kept.is_empty() || gate.runs >= gate.max_runs {
            kept_all.extend(kept.iter().map(|i| candidates[*i].clone()));
            break;
        }
        kept_all.extend(kept.iter().map(|i| candidates[*i].clone()));
        scanned = scan::scan(&gate.ctx.root, opts.library)?;
    }
    let tidied =
        if opts.only.is_empty() || opts.only.contains(&CandKind::Dep) { go_mod_tidy(&mut gate, &mut rec)? } else { 0 };

    let (check_runs, untested, mut tests) = (gate.runs, gate.untested, gate.last_tests);
    let changed_anything = !kept_all.is_empty() || tidied > 0;
    // One last run on the final state when a failed try came after the last pass (a build output or a
    // cache may be stale), or when there's a build output to weigh.
    if changed_anything && (gate.failed || size_before.is_some()) {
        ui::step("checking the final state");
        if let Some(reason) = gate.broken(&[])? {
            rec.restore(None)?;
            bail!("the final state didn't pass ({reason}), so every change was put back");
        }
        tests = gate.last_tests;
    }
    if !changed_anything {
        rec.restore(None)?;
        return Ok(Outcome {
            before,
            after: before,
            kept: kept_all,
            put_back,
            untested,
            check_runs,
            tests,
            size_before,
            size_after: size_before,
            tidied,
            run: None,
        });
    }
    let after = scan::scan(&ctx.root, opts.library)?.metrics;
    let size_after = ctx.measure_size();
    let mut cuts: Vec<CutRecord> = kept_all
        .iter()
        .map(|c| CutRecord { kind: c.kind, name: c.name.clone(), path: c.path.clone(), lines: c.lines })
        .collect();
    if tidied > 0 {
        cuts.push(CutRecord {
            kind: CandKind::Dep,
            name: format!("go mod tidy ({tidied})"),
            path: "go.mod".into(),
            lines: tidied,
        });
    }
    if ctx.state.baseline.is_none() {
        ctx.state.baseline = Some(Baseline { metrics: before, tests: base.tests, size: size_before, at: util::now() });
    }
    ctx.state.runs.push(Run {
        id: run_id,
        at: util::now(),
        kind: "cut".into(),
        cuts,
        before,
        after,
        tests,
        size: size_after,
        files: rec.finish(),
        undone: false,
    });
    ctx.save()?;
    Ok(Outcome {
        before,
        after,
        kept: kept_all,
        put_back,
        untested,
        check_runs,
        tests,
        size_before,
        size_after,
        tidied,
        run: Some(run_id),
    })
}

pub struct Verified {
    pub ok: bool,
    pub tests: Option<u64>,
    pub base_tests: Option<u64>,
    pub secs: f64,
    /// The end of the check's output when it failed.
    pub output: String,
}

/// Runs the check on the project as it is now, against the numbers from before the first cut.
pub fn verify(ctx: &Ctx) -> Result<Verified> {
    ensure!(
        !ctx.state.check.is_empty(),
        "no check command yet. Give one after --, for example: shrink-anything verify -- npm test"
    );
    let out = check::run(&ctx.root, &ctx.state.check, Duration::from_secs(3600), &ctx.dir.join("check.log"))?;
    let base_tests = ctx.state.baseline.as_ref().and_then(|b| b.tests);
    let fewer = base_tests.is_some_and(|base| out.tests.unwrap_or(0) < base);
    Ok(Verified {
        ok: out.ok && !fewer,
        tests: out.tests,
        base_tests,
        secs: out.secs,
        output: if out.ok && !fewer { String::new() } else { util::tail(&out.output, 25) },
    })
}
