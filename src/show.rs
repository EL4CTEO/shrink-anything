//! What each command prints.

use crate::agent::AgentOutcome;
use crate::candidate::{CandKind, Candidate, Tier};
use crate::cut::{Outcome, Verified};
use crate::report::Report;
use crate::scan::{Metrics, Scan};
use crate::state::{Ctx, Run};
use crate::weigh::Weight;
use crate::{ui, util};

/// Rows shown per section without --all.
const SHOWN: usize = 12;

fn fit(s: &str, width: usize) -> String {
    if s.chars().count() <= width {
        format!("{s:<width$}")
    } else {
        let head: String = s.chars().take(width.saturating_sub(1)).collect();
        format!("{head}…")
    }
}

fn plural(n: u64, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

pub fn next(command: &str) {
    ui::line(format!("  {} {}", ui::dim("next:"), ui::bold(command)));
}

fn metrics_line(m: &Metrics) -> String {
    let mut line = format!(
        "{} lines · {} files · {} · ~{} tokens for an agent to read",
        util::thousands(m.lines),
        util::thousands(m.files),
        plural(m.deps, "dependency", "dependencies"),
        util::compact(m.tokens())
    );
    if m.asset_bytes > 0 {
        line.push_str(&format!(" · {} of assets", util::bytes(m.asset_bytes)));
    }
    line
}

fn change(label: &str, before: u64, after: u64, show: fn(u64) -> String) -> Option<String> {
    (before != after).then(|| {
        format!("{} → {} {label} ({})", show(before), ui::bold(show(after)), ui::green(util::delta(before, after)))
    })
}

fn section(title: &str, mark: &str, candidates: &[&Candidate], all: bool) {
    if candidates.is_empty() {
        return;
    }
    ui::line("");
    ui::line(format!("  {}{}", ui::bold(fit(title, 74)), ui::dim("  saves")));
    for c in candidates.iter().take(if all { usize::MAX } else { SHOWN }) {
        let at = if c.line == 0 { c.path.clone() } else { format!("{}:{}", c.path, c.line) };
        let saves = match c.kind {
            CandKind::Asset => util::bytes(c.bytes),
            CandKind::Dep => "1 dep".to_string(),
            _ => format!("{} lines", c.lines),
        };
        ui::line(format!(
            "  {mark} {} {} {} {:>9}",
            ui::dim(fit(c.kind.label(), 5)),
            fit(&c.name, 28),
            ui::dim(fit(&at, 34)),
            saves
        ));
        if all {
            ui::line(format!("          {}", ui::dim(&c.why)));
        }
    }
    if !all && candidates.len() > SHOWN {
        ui::line(ui::dim(format!("    … and {} more (--all)", candidates.len() - SHOWN)));
    }
}

/// "412 lines, 2 dependencies and 1.2 MB of assets"
fn savings(lines: u64, deps: u64, bytes: u64) -> String {
    let mut parts = Vec::new();
    if lines > 0 || (deps == 0 && bytes == 0) {
        parts.push(format!("{} lines", util::thousands(lines)));
    }
    if deps > 0 {
        parts.push(plural(deps, "dependency", "dependencies"));
    }
    if bytes > 0 {
        parts.push(format!("{} of assets", util::bytes(bytes)));
    }
    match parts.len() {
        1 => parts.remove(0),
        n => format!("{} and {}", parts[..n - 1].join(", "), parts[n - 1]),
    }
}

pub fn scan(s: &Scan, all: bool, has_check: bool) {
    let langs: Vec<String> = s.languages.iter().take(6).map(|(name, n)| format!("{name} {n}")).collect();
    ui::step(format!(
        "read {} files: {}",
        s.metrics.files,
        if langs.is_empty() { "no language I know".into() } else { langs.join(", ") }
    ));
    ui::line(format!("  {}", metrics_line(&s.metrics)));
    if !s.library.is_empty() {
        ui::line(ui::dim(format!(
            "  treated as a library, so public names are kept: {} (--app to cut them too)",
            s.library.join(", ")
        )));
    }
    let safe: Vec<&Candidate> = s.tier(Tier::Safe).collect();
    let risky: Vec<&Candidate> = s.tier(Tier::Risky).collect();
    section("can go, and your check will prove it", &ui::mint("✂"), &safe, all);
    section("needs a look first (your agent, or --aggressive)", &ui::yellow("?"), &risky, all);
    if !s.duplicates.is_empty() {
        ui::line("");
        ui::line(format!("  {}", ui::bold("the same thing twice (for your agent)")));
        for d in s.duplicates.iter().take(if all { usize::MAX } else { 6 }) {
            let places: Vec<String> = d
                .copies
                .iter()
                .map(|c| if c.start == 0 { c.path.clone() } else { format!("{}:{}", c.path, c.start) })
                .collect();
            let size = if d.kind == "asset" { util::bytes(d.bytes) } else { format!("{} lines", d.lines) };
            ui::line(format!(
                "  {} {:>10} × {}  {}",
                ui::mint("≡"),
                size,
                d.copies.len(),
                ui::dim(fit(&places.join("  "), 64))
            ));
        }
        if !all && s.duplicates.len() > 6 {
            ui::line(ui::dim(format!("    … and {} more (--all)", s.duplicates.len() - 6)));
        }
    }
    ui::line("");
    if safe.is_empty() && risky.is_empty() && s.duplicates.is_empty() {
        ui::ok("nothing to cut: no dead code, dead files, unused dependencies, unused assets or duplicates found");
        return;
    }
    let deps = safe.iter().filter(|c| c.kind == CandKind::Dep).count() as u64;
    let mut line = format!("  {} can go now", ui::bold(savings(s.lines(Tier::Safe) as u64, deps, s.bytes(Tier::Safe))));
    if !risky.is_empty() {
        line.push_str(&format!(
            " · {} more with a look",
            savings(s.lines(Tier::Risky) as u64, 0, s.bytes(Tier::Risky))
        ));
    }
    if !s.duplicates.is_empty() {
        line.push_str(&format!(" · {} in duplicates", savings(s.duplicate_lines() as u64, 0, s.duplicate_bytes())));
    }
    ui::line(line);
    next(if has_check { "shrink-anything cut" } else { "shrink-anything cut -- <your test command>" });
}

fn totals(before: &Metrics, after: &Metrics) -> String {
    let parts: Vec<String> = [
        change("lines", before.lines, after.lines, util::thousands),
        change("files", before.files, after.files, util::thousands),
        change("dependencies", before.deps, after.deps, util::thousands),
        change("of assets", before.asset_bytes, after.asset_bytes, util::bytes),
    ]
    .into_iter()
    .flatten()
    .collect();
    if parts.is_empty() { "the numbers didn't move".to_string() } else { parts.join(" · ") }
}

pub fn cut(o: &Outcome) {
    for p in o.put_back.iter().take(8) {
        ui::line(format!("  {} {} {}", ui::yellow("↩"), fit(&p.candidate.name, 30), ui::dim(&p.reason)));
    }
    if o.put_back.len() > 8 {
        ui::line(ui::dim(format!("    … and {} more put back", o.put_back.len() - 8)));
    }
    if o.untested > 0 {
        ui::warn(format!("{} candidates weren't tried: the budget of check runs ran out (--max-runs)", o.untested));
    }
    if o.run.is_none() {
        ui::ok(if o.put_back.is_empty() {
            "nothing to cut"
        } else {
            "nothing could go without breaking the check, so nothing changed"
        });
        return;
    }
    ui::ok(totals(&o.before, &o.after));
    let tests = match o.tests {
        Some(1) => "1 test still runs and passes".to_string(),
        Some(n) => format!("{n} tests still run and pass"),
        None => "the check still passes".to_string(),
    };
    ui::line(format!("  {tests} · the check ran {} times", o.check_runs + 1));
    if let (Some(before), Some(after)) = (o.size_before, o.size_after) {
        ui::line(format!(
            "  build size {} → {} ({})",
            util::bytes(before),
            ui::bold(util::bytes(after)),
            ui::green(util::delta(before, after))
        ));
    }
    ui::line(ui::dim("  take it back with: shrink-anything undo"));
}

pub fn agent(o: &AgentOutcome) {
    match (&o.rejected, o.run) {
        (Some(reason), _) => {
            ui::warn(format!("the agent's changes were put back: {}", reason.lines().next().unwrap_or_default()));
        }
        (None, Some(_)) => {
            let tests = o.tests.map_or(String::new(), |n| format!(", {n} tests still pass"));
            ui::ok(format!("agent: {} ({} files{tests})", totals(&o.before, &o.after), o.files));
        }
        (None, None) => {}
    }
}

pub fn verify(v: &Verified) {
    let tests = match (v.tests, v.base_tests) {
        (Some(now), Some(base)) => format!(" ({now} tests, {base} before the first cut)"),
        (Some(now), None) => format!(" ({now} tests)"),
        _ => String::new(),
    };
    if v.ok {
        ui::ok(format!("the check passes in {}{tests}", util::secs(v.secs)));
    } else {
        ui::line(ui::dim(&v.output));
        ui::error(format!("the check doesn't pass{tests}"));
    }
}

pub fn undo(runs: &[Run]) {
    if runs.is_empty() {
        ui::ok("nothing to undo");
    }
    for run in runs {
        ui::ok(format!("run {} taken back: {} restored", run.id, plural(run.files.len() as u64, "file", "files")));
    }
    if runs.iter().any(|r| r.files.iter().any(|f| f.path.ends_with("package.json"))) {
        ui::line(ui::dim("  package.json changed back: reinstall to get the packages back"));
    }
}

pub fn report(r: &Report) {
    ui::line("");
    ui::line(&r.markdown);
    ui::ok(format!("wrote {} and {}", r.md.display(), r.svg.display()));
}

pub fn weigh(w: &Weight) {
    ui::step(format!(
        "weighed {}: {} in {}",
        w.paths.join(", "),
        ui::bold(util::bytes(w.bytes)),
        plural(w.files, "file", "files")
    ));
    let share = |bytes: u64| if w.bytes == 0 { 0.0 } else { bytes as f64 / w.bytes as f64 };
    ui::line("");
    ui::line(format!("  {}", ui::bold("by kind")));
    for (ext, bytes, files) in w.kinds.iter().take(8) {
        let bar = "█".repeat((share(*bytes) * 30.0).round() as usize);
        ui::line(format!(
            "  {} {:>10} {:>4.0}%  {} {}",
            fit(ext, 10),
            util::bytes(*bytes),
            share(*bytes) * 100.0,
            ui::mint(fit(&bar, 30)),
            ui::dim(plural(*files, "file", "files"))
        ));
    }
    ui::line("");
    ui::line(format!("  {}", ui::bold("heaviest files")));
    for (path, bytes) in &w.heaviest {
        ui::line(format!("  {:>10} {:>4.0}%  {}", util::bytes(*bytes), share(*bytes) * 100.0, ui::dim(path)));
    }
    if w.duplicated > 0 || w.source_maps > 0 {
        ui::line("");
        ui::line(format!("  {}", ui::bold("easy wins")));
    }
    if w.source_maps > 0 {
        ui::line(format!(
            "  {} {:>10}  source maps: fine for debugging, usually not meant to ship",
            ui::yellow("!"),
            util::bytes(w.source_maps)
        ));
    }
    if w.duplicated > 0 {
        ui::line(format!(
            "  {} {:>10}  spent on files that are in there more than once",
            ui::yellow("!"),
            util::bytes(w.duplicated)
        ));
        for (bytes, paths) in w.duplicates.iter().take(5) {
            ui::line(ui::dim(format!(
                "               {} × {}: {}",
                util::bytes(*bytes),
                paths.len(),
                fit(&paths.join("  "), 70)
            )));
        }
    }
    ui::line("");
    next(&format!("shrink-anything cut --size {} -- <your build and test command>", w.paths[0]));
}

pub fn status(ctx: &Ctx) {
    if ctx.state.check.is_empty() {
        ui::line("  no check command yet");
    } else {
        ui::line(format!("  check: {}", ui::bold(ctx.state.check.join(" "))));
    }
    let Some(base) = &ctx.state.baseline else {
        ui::line("  nothing has been cut yet");
        next("shrink-anything scan");
        return;
    };
    ui::line(format!("  before the first cut: {}", metrics_line(&base.metrics)));
    for run in ctx.live_runs() {
        let what = if run.kind == "agent" { "agent".to_string() } else { plural(run.cuts.len() as u64, "cut", "cuts") };
        ui::line(format!("  run {} ({what}): {}", run.id, totals(&run.before, &run.after)));
    }
    next("shrink-anything report");
}
