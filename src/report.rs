//! The before and after, as markdown for a pull request and an SVG card to share.

use crate::candidate::CandKind;
use crate::scan::{self, Metrics};
use crate::state::Ctx;
use crate::util;
use anyhow::{Context, Result};
use serde::Serialize;
use std::fmt::Write as _;
use std::fs;
use std::path::PathBuf;

#[derive(Serialize)]
pub struct Report {
    pub before: Metrics,
    pub after: Metrics,
    pub tests_before: Option<u64>,
    pub tests_after: Option<u64>,
    pub size_before: Option<u64>,
    pub size_after: Option<u64>,
    pub code_cut: usize,
    pub files_cut: usize,
    pub deps_cut: usize,
    pub assets_cut: usize,
    pub agent_runs: usize,
    pub check: String,
    pub md: PathBuf,
    pub svg: PathBuf,
    pub markdown: String,
}

struct Row {
    label: &'static str,
    before: u64,
    after: u64,
    show: fn(u64) -> String,
}

fn rows(r: &Report) -> Vec<Row> {
    let mut rows = vec![
        Row { label: "lines", before: r.before.lines, after: r.after.lines, show: util::thousands },
        Row { label: "files", before: r.before.files, after: r.after.files, show: util::thousands },
        Row { label: "dependencies", before: r.before.deps, after: r.after.deps, show: util::thousands },
        Row {
            label: "tokens for an agent",
            before: r.before.tokens(),
            after: r.after.tokens(),
            show: |n| format!("~{}", util::compact(n)),
        },
    ];
    if r.before.asset_bytes > 0 {
        rows.push(Row { label: "assets", before: r.before.asset_bytes, after: r.after.asset_bytes, show: util::bytes });
    }
    if let (Some(before), Some(after)) = (r.size_before, r.size_after) {
        rows.push(Row { label: "build size", before, after, show: util::bytes });
    }
    // nothing to compare where there was nothing
    rows.retain(|row| row.before > 0 || row.after > 0);
    rows
}

fn tests_line(r: &Report) -> String {
    match (r.tests_before, r.tests_after) {
        (Some(before), Some(after)) => format!("{before} tests ran before, {after} after"),
        _ => "the check passed before and after".to_string(),
    }
}

fn markdown(r: &Report) -> String {
    let mut md = String::from("## shrink-anything report\n\n| | before | after | |\n|---|---:|---:|---:|\n");
    for row in rows(r) {
        let delta = util::delta(row.before, row.after);
        let delta = if delta.is_empty() { String::new() } else { format!("**{delta}**") };
        let _ = writeln!(md, "| {} | {} | {} | {delta} |", row.label, (row.show)(row.before), (row.show)(row.after));
    }
    let mut cut = Vec::new();
    for (n, one, many) in [
        (r.code_cut, "piece of dead code", "pieces of dead code"),
        (r.files_cut, "dead file", "dead files"),
        (r.deps_cut, "unused dependency", "unused dependencies"),
        (r.assets_cut, "unused asset", "unused assets"),
    ] {
        if n > 0 {
            cut.push(format!("{n} {}", if n == 1 { one } else { many }));
        }
    }
    if r.agent_runs > 0 {
        cut.push("duplicates and leftovers merged by a coding agent".to_string());
    }
    let _ = writeln!(md, "\nRemoved: {}.", if cut.is_empty() { "nothing yet".to_string() } else { cut.join(", ") });
    let _ = writeln!(md, "Checked with `{}`: {}.", r.check, tests_line(r));
    let _ = writeln!(
        md,
        "\nMade with [shrink-anything](https://github.com/EL4CTEO/shrink-anything). Undo: `shrink-anything undo --all`."
    );
    md
}

fn xml(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

fn svg(r: &Report) -> String {
    const FONT: &str = "-apple-system,'Segoe UI',Helvetica,Arial,sans-serif";
    const MONO: &str = "ui-monospace,'Cascadia Code',Consolas,Menlo,monospace";
    let rows = rows(r);
    let minus = |s: String| s.replace('-', "\u{2212}");
    // the headline is whatever shrank the most
    let drop = |row: &Row| if row.before == 0 { 0.0 } else { 1.0 - row.after as f64 / row.before as f64 };
    let best = rows.iter().max_by(|a, b| drop(a).total_cmp(&drop(b))).unwrap_or(&rows[0]);
    let headline = match util::delta(best.before, best.after) {
        d if d.is_empty() => "0%".to_string(),
        d => minus(d),
    };
    let what = if best.label == "lines" { "lines of code" } else { best.label };
    let mut s = String::new();
    let _ = write!(
        s,
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="1200" height="630" viewBox="0 0 1200 630" font-family="{FONT}">
<defs><linearGradient id="bg" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="#11161d"/><stop offset="1" stop-color="#090c10"/></linearGradient>
<linearGradient id="mint" x1="0" y1="0" x2="1" y2="0"><stop offset="0" stop-color="#2fbf87"/><stop offset="1" stop-color="#5eead4"/></linearGradient></defs>
<rect width="1200" height="630" fill="url(#bg)"/>
<rect x="24" y="24" width="1152" height="582" rx="28" fill="#161b22" stroke="#30363d" stroke-width="2"/>
<g fill="none" stroke="url(#mint)" stroke-width="9" stroke-linecap="round" stroke-linejoin="round"><path d="M72 76l20 20-20 20"/><path d="M138 76l-20 20 20 20"/></g>
<rect x="99" y="82" width="12" height="28" rx="4" fill="#5eead4"/>
<text x="160" y="108" font-size="34" font-weight="700" fill="#e6edf3">shrink<tspan fill="#5eead4">-anything</tspan></text>
<text x="72" y="330" font-size="180" font-weight="800" fill="url(#mint)" letter-spacing="-6">{headline}</text>
<text x="76" y="392" font-size="38" font-weight="600" fill="#e6edf3">{}</text>
<text x="76" y="440" font-size="28" fill="#8b949e" font-family="{MONO}">{} → {}</text>
"##,
        xml(what),
        xml(&(best.show)(best.before)),
        xml(&(best.show)(best.after)),
    );
    let (x, width) = (650.0, 470.0);
    let step = (360.0 / rows.len() as f64).min(84.0);
    let top = 330.0 - step * (rows.len() as f64 - 1.0) / 2.0 - 14.0;
    for (i, row) in rows.iter().enumerate() {
        let y = top + i as f64 * step;
        let share = if row.before == 0 { 1.0 } else { (row.after as f64 / row.before as f64).clamp(0.02, 1.0) };
        let delta = minus(util::delta(row.before, row.after));
        let _ = write!(
            s,
            r##"<text x="{x}" y="{y}" font-size="21" fill="#8b949e">{}</text>
<text x="{}" y="{y}" font-size="18" fill="#e6edf3" text-anchor="end" font-family="{MONO}">{} → {}</text>
<text x="{}" y="{y}" font-size="21" font-weight="700" fill="#5eead4" text-anchor="end">{delta}</text>
<rect x="{x}" y="{}" width="{width}" height="10" rx="5" fill="#30363d"/>
<rect x="{x}" y="{}" width="{:.1}" height="10" rx="5" fill="url(#mint)"/>
"##,
            row.label,
            x + width - 78.0,
            xml(&(row.show)(row.before)),
            xml(&(row.show)(row.after)),
            x + width,
            y + 12.0,
            y + 12.0,
            width * share,
        );
    }
    let check: String = r.check.chars().take(60).collect();
    let _ = write!(
        s,
        r##"<line x1="72" y1="520" x2="1128" y2="520" stroke="#30363d" stroke-width="2"/>
<text x="72" y="566" font-size="26" fill="#e6edf3"><tspan fill="#3fb950" font-weight="700">✓</tspan> {}</text>
<text x="1128" y="566" font-size="22" fill="#8b949e" text-anchor="end" font-family="{MONO}">{}</text>
</svg>
"##,
        xml(&tests_line(r)),
        xml(&check)
    );
    s
}

pub fn report(ctx: &Ctx, library: Option<bool>) -> Result<Report> {
    let base = ctx
        .state
        .baseline
        .as_ref()
        .context("nothing has been cut yet: run shrink-anything cut -- <your test command> first")?;
    let after = scan::scan(&ctx.root, library)?.metrics;
    let count = |kind: CandKind| ctx.live_runs().flat_map(|r| r.cuts.iter()).filter(|c| c.kind == kind).count();
    let last = ctx.live_runs().last();
    let mut r = Report {
        before: base.metrics,
        after,
        tests_before: base.tests,
        tests_after: last.and_then(|r| r.tests),
        size_before: base.size,
        size_after: last.and_then(|r| r.size),
        code_cut: count(CandKind::Symbol),
        files_cut: count(CandKind::File),
        deps_cut: count(CandKind::Dep),
        assets_cut: count(CandKind::Asset),
        agent_runs: ctx.live_runs().filter(|r| r.kind == "agent").count(),
        check: ctx.state.check.join(" "),
        md: ctx.dir.join("report.md"),
        svg: ctx.dir.join("report.svg"),
        markdown: String::new(),
    };
    r.markdown = markdown(&r);
    fs::create_dir_all(&ctx.dir)?;
    fs::write(&r.md, &r.markdown)?;
    fs::write(&r.svg, svg(&r))?;
    Ok(r)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Report {
        Report {
            before: Metrics {
                files: 40,
                lines: 12_480,
                code_lines: 10_000,
                bytes: 400_000,
                deps: 31,
                assets: 0,
                asset_bytes: 0,
            },
            after: Metrics {
                files: 36,
                lines: 9_870,
                code_lines: 8_000,
                bytes: 310_000,
                deps: 27,
                assets: 0,
                asset_bytes: 0,
            },
            tests_before: Some(132),
            tests_after: Some(132),
            size_before: Some(4_400_000),
            size_after: Some(3_100_000),
            code_cut: 23,
            files_cut: 4,
            deps_cut: 1,
            assets_cut: 0,
            agent_runs: 0,
            check: "npm test && echo \"<ok>\"".into(),
            md: PathBuf::new(),
            svg: PathBuf::new(),
            markdown: String::new(),
        }
    }

    #[test]
    fn markdown_has_the_numbers() {
        let md = markdown(&sample());
        assert!(md.contains("| lines | 12,480 | 9,870 | **-21%** |"), "{md}");
        assert!(md.contains("| dependencies | 31 | 27 | **-13%** |"));
        assert!(md.contains("23 pieces of dead code, 4 dead files, 1 unused dependency."));
        assert!(md.contains("132 tests ran before, 132 after"));
    }

    #[test]
    fn svg_is_well_formed_enough() {
        let s = svg(&sample());
        assert!(s.starts_with("<svg") && s.trim_end().ends_with("</svg>"));
        assert!(s.contains("\u{2212}21%") && s.contains("&lt;ok&gt;") && !s.contains("<ok>"));
        assert_eq!(s.matches("<text").count(), s.matches("</text>").count());
    }
}
