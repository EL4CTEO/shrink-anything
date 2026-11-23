//! Dead code: a name whose every mention sits inside its own definition.
//!
//! This is deliberately a count of names, not a call graph. A count can't be fooled into thinking
//! something is dead: any other mention anywhere (another file, a string, a template, a config file)
//! keeps the name alive. It can only miss dead code, never invent it.

use crate::candidate::{CandKind, Candidate, Edit, Tier};
use crate::defs::{Decor, Def};
use crate::index::{Index, Parsed};
use crate::lang::{Kind, Private};
use crate::lex::count_word;
use regex::Regex;
use std::collections::{BTreeMap, HashMap};
use std::sync::OnceLock;

enum Verdict {
    Safe,
    Risky(&'static str),
    /// Something calls it without naming it. Not a candidate.
    Keep,
}

fn has_word(text: &str, word: &str) -> bool {
    count_word(text, word) > 0
}

/// A constant whose value runs code can't go just because its name is unused.
fn has_side_effects(code: &str) -> bool {
    static CALL: OnceLock<Regex> = OnceLock::new();
    let call = CALL.get_or_init(|| Regex::new(r"[\w\]\)]\s*\(").expect("pattern"));
    call.is_match(code) || has_word(code, "await") || has_word(code, "new")
}

fn inherits(d: &Def, patterns: &[&str]) -> bool {
    patterns.iter().any(|p| {
        if *p == "(" {
            // Python: class A(Base)
            d.header.split_once('(').is_some_and(|(_, rest)| {
                let base = rest.split(')').next().unwrap_or_default().trim();
                !base.is_empty() && base != "object"
            })
        } else {
            d.header.contains(p)
        }
    })
}

fn verdict(ix: &Index, file: &Parsed, d: &Def) -> Verdict {
    let Some(lang) = file.entry.lang else { return Verdict::Keep };
    let spec = lang.spec;
    let name = d.name.as_str();
    if file.entry.test || file.entry.generated {
        return Verdict::Keep;
    }
    if name.starts_with("__") || name.to_ascii_lowercase().starts_with("test") {
        return Verdict::Keep;
    }
    if spec.keep.contains(&name) || spec.keep_prefixes.iter().any(|p| name.starts_with(p)) {
        return Verdict::Keep;
    }
    if d.decor == Decor::Registered || has_word(&d.header, "override") {
        return Verdict::Keep;
    }
    if ix.prefixes.iter().any(|p| name.starts_with(p.as_str())) {
        return Verdict::Keep; // "handle_" + x somewhere builds names like this one
    }
    if ["Test", "Tests", "TestCase", "Spec"].iter().any(|s| name.ends_with(s)) || d.header.contains("export default") {
        return Verdict::Keep; // run by a test runner, or imported under whatever name the importer picks
    }
    let p = d.parent.as_str();
    let contract = ["trait", "interface", "protocol"].iter().any(|w| has_word(p, w))
        || (has_word(p, "impl") && has_word(p, "for"))
        || (has_word(p, "extension") && p.contains(':'))
        || has_word(p, "new");
    if contract {
        return Verdict::Keep; // the trait or interface decides what exists here
    }
    if d.kind == Kind::Const && (d.indent > 0 || has_side_effects(file.lines_code(d.line, d.end))) {
        return Verdict::Keep;
    }
    let lib = ix.lib.is(spec.name);
    let private = match spec.private {
        Private::Keyword(word) => has_word(&d.header, word) || d.header.contains(&format!("#{name}")),
        Private::Underscore => name.starts_with('_'),
        Private::Lowercase => name.starts_with(|c: char| c.is_lowercase() || c == '_'),
        Private::NotPub => !d.header.starts_with("pub") || d.header.starts_with("pub(") || !lib,
        Private::Never => false,
    };
    let member = d.kind == Kind::Method || d.indent > 0;
    // a member of a class with a base may be the base's hook: textwrap calls _wrap_chunks on whatever subclass it gets
    let overrides = member
        && spec.inherits.iter().any(|pat| {
            if *pat == "(" {
                p.split_once('(').is_some_and(|(_, rest)| {
                    let base = rest.split(')').next().unwrap_or_default().trim();
                    !base.is_empty() && base != "object"
                })
            } else {
                p.contains(pat)
            }
        });
    if overrides {
        return Verdict::Risky("a member of a class with a base type: the base may call it");
    }
    if member && !private {
        return Verdict::Risky("a public member: something may call it through an interface or by name");
    }
    if d.kind == Kind::Type && inherits(d, spec.inherits) {
        return Verdict::Risky("a type with a base that may register or call it");
    }
    let exported = match spec.name {
        "JavaScript" | "TypeScript" => d.header.starts_with("export"),
        "Rust" => d.header.starts_with("pub ") || d.header.starts_with("pub\t"),
        "Python" => !name.starts_with('_'),
        "Go" => name.starts_with(char::is_uppercase),
        _ => !private,
    };
    if lib && exported {
        return Verdict::Risky("exported from a library: its users aren't in this repo");
    }
    let dir = file.entry.dir();
    if dir.split('/').any(|part| crate::find::files::CONVENTION_DIRS.contains(&part)) {
        return Verdict::Risky("in a folder a framework loads by convention");
    }
    let stem = file.entry.name().split('.').next().unwrap_or_default().to_ascii_lowercase();
    let configuration = ["settings", "config", "conf", "defaults"]
        .iter()
        .any(|w| stem.contains(w) || dir.split('/').any(|part| part == *w));
    if d.kind == Kind::Const && configuration {
        return Verdict::Risky("a setting: frameworks read these by name");
    }
    if file.dynamic {
        return Verdict::Risky("its file looks names up at run time");
    }
    Verdict::Safe
}

fn merge(ranges: &mut Vec<(usize, usize)>) {
    ranges.sort_unstable();
    let mut out: Vec<(usize, usize)> = Vec::new();
    for &(s, e) in ranges.iter() {
        match out.last_mut() {
            Some(last) if s <= last.1 + 1 => last.1 = last.1.max(e),
            _ => out.push((s, e)),
        }
    }
    *ranges = out;
}

pub fn find(ix: &Index) -> Vec<Candidate> {
    let mut groups: HashMap<&str, Vec<(usize, &Def)>> = HashMap::new();
    for (fi, file) in ix.files.iter().enumerate() {
        for d in &file.defs {
            groups.entry(d.name.as_str()).or_default().push((fi, d));
        }
    }
    let mut out = Vec::new();
    for (name, members) in groups {
        if name.len() < 3 {
            continue;
        }
        let total = ix.counts.get(name).copied().unwrap_or(0);
        let mut by_file: BTreeMap<usize, Vec<(usize, usize)>> = BTreeMap::new();
        for (fi, d) in &members {
            by_file.entry(*fi).or_default().push((d.start, d.end));
        }
        let mut inside = 0;
        for (fi, ranges) in &mut by_file {
            merge(ranges);
            inside += ranges.iter().map(|&(s, e)| count_word(ix.files[*fi].lines(s, e), name)).sum::<u32>();
        }
        if inside < total {
            continue; // mentioned somewhere else: alive
        }
        let mut real: Vec<&(usize, &Def)> = members.iter().filter(|(_, d)| d.kind != Kind::Attached).collect();
        if real.is_empty() {
            continue;
        }
        let mut risk = None;
        let mut keep = false;
        for (fi, d) in &real {
            match verdict(ix, &ix.files[*fi], d) {
                Verdict::Keep => keep = true,
                Verdict::Risky(why) => risk = risk.or(Some(why)),
                Verdict::Safe => {}
            }
        }
        if keep {
            continue;
        }
        real.sort_by_key(|(fi, d)| (&ix.files[*fi].entry.rel, d.line));
        let (first_file, first) = real[0];
        let path = ix.files[*first_file].entry.rel.clone();
        let mut edits = Vec::new();
        let mut lines = 0;
        for (fi, ranges) in &by_file {
            for &(start, end) in ranges {
                lines += end - start + 1;
                edits.push(Edit::Remove { path: ix.files[*fi].entry.rel.clone(), start, end });
            }
        }
        let mentions = if total == 1 { "its only mention is".to_string() } else { format!("all {total} mentions are") };
        let why = match risk {
            Some(risk) => format!("{mentions} its own definition, but it is {risk}"),
            None => format!("{mentions} its own definition"),
        };
        out.push(Candidate {
            id: format!("code:{path}:{name}"),
            kind: CandKind::Symbol,
            tier: if risk.is_some() { Tier::Risky } else { Tier::Safe },
            name: name.to_string(),
            path,
            line: first.line + 1,
            lines,
            bytes: 0,
            why,
            edits,
        });
    }
    out
}
