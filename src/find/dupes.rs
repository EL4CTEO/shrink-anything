//! Duplicates: files and definitions that are the same code twice. Merging them takes judgment
//! (which copy stays, who imports what), so these go to the agent, not to the automatic cut.

use crate::index::Index;
use crate::lang::Kind;
use crate::util;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

const MIN_LINES: usize = 6;
const MIN_CHARS: usize = 160;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Copy {
    pub path: String,
    /// 1-based, inclusive. 0..0 for a whole file.
    pub start: usize,
    pub end: usize,
    pub name: String,
}

impl DupeGroup {
    /// Bytes that go if one copy stays.
    pub fn reclaim_bytes(&self) -> u64 {
        self.bytes * (self.copies.len() as u64).saturating_sub(1)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DupeGroup {
    /// "file" or "code".
    pub kind: String,
    /// Lines in one copy.
    pub lines: usize,
    /// Lines that go if one copy stays.
    pub reclaim: usize,
    /// Bytes in one copy, for assets.
    #[serde(default)]
    pub bytes: u64,
    pub copies: Vec<Copy>,
}

/// Code with whitespace squeezed out and its own name replaced, so two copies under different names match.
fn normalize(text: &str, name: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut word = String::new();
    let flush = |word: &mut String, out: &mut String| {
        if !word.is_empty() {
            out.push_str(if word.as_str() == name { "\u{1}" } else { word.as_str() });
            out.push(' ');
            word.clear();
        }
    };
    for c in text.chars() {
        if crate::lex::is_word(c) {
            word.push(c);
        } else {
            flush(&mut word, &mut out);
            if !c.is_whitespace() {
                out.push(c);
            }
        }
    }
    flush(&mut word, &mut out);
    out
}

pub fn find(ix: &Index) -> Vec<DupeGroup> {
    let mut groups: Vec<DupeGroup> = Vec::new();

    // whole files
    let mut by_text: HashMap<u64, Vec<usize>> = HashMap::new();
    for (fi, file) in ix.files.iter().enumerate() {
        let e = &file.entry;
        if e.lang.is_none() || e.generated || file.line_count() < MIN_LINES {
            continue;
        }
        by_text.entry(util::hash(normalize(file.refs(), "").as_bytes())).or_default().push(fi);
    }
    let mut whole: Vec<usize> = Vec::new();
    for files in by_text.into_values().filter(|f| f.len() > 1) {
        let lines = ix.files[files[0]].line_count();
        whole.extend(&files);
        groups.push(DupeGroup {
            kind: "file".into(),
            lines,
            reclaim: lines * (files.len() - 1),
            bytes: 0,
            copies: files
                .iter()
                .map(|fi| Copy { path: ix.files[*fi].entry.rel.clone(), start: 0, end: 0, name: String::new() })
                .collect(),
        });
    }

    // definitions
    let mut by_body: HashMap<u64, Vec<(usize, usize)>> = HashMap::new();
    for (fi, file) in ix.files.iter().enumerate() {
        if file.entry.generated || whole.contains(&fi) {
            continue;
        }
        for (di, d) in file.defs.iter().enumerate() {
            if !matches!(d.kind, Kind::Function | Kind::Method | Kind::Type) || d.end - d.line + 1 < MIN_LINES {
                continue;
            }
            // the name is only replaced on its own line: f() { g.f() } and h() { g.h() } are different code
            let text = file.lines(d.line, d.end);
            let (head, rest) = text.split_once('\n').unwrap_or((text, ""));
            let body = normalize(head, &d.name) + &normalize(rest, "");
            if body.len() >= MIN_CHARS {
                by_body.entry(util::hash(body.as_bytes())).or_default().push((fi, di));
            }
        }
    }
    let mut code: Vec<Vec<(usize, usize)>> = by_body.into_values().filter(|g| g.len() > 1).collect();
    // biggest first, so a duplicated class hides its duplicated methods
    code.sort_by_key(|g| std::cmp::Reverse(ix.files[g[0].0].defs[g[0].1].end - ix.files[g[0].0].defs[g[0].1].line));
    let mut covered: Vec<(usize, usize, usize)> = Vec::new();
    for group in code {
        let span = |&(fi, di): &(usize, usize)| {
            let d = &ix.files[fi].defs[di];
            (fi, d.line, d.end)
        };
        if group.iter().all(|m| {
            let (fi, s, e) = span(m);
            covered.iter().any(|&(cf, cs, ce)| cf == fi && cs <= s && e <= ce)
        }) {
            continue;
        }
        covered.extend(group.iter().map(span));
        let (_, s, e) = span(&group[0]);
        let lines = e - s + 1;
        groups.push(DupeGroup {
            kind: "code".into(),
            lines,
            reclaim: lines * (group.len() - 1),
            bytes: 0,
            copies: group
                .iter()
                .map(|&(fi, di)| {
                    let d = &ix.files[fi].defs[di];
                    Copy {
                        path: ix.files[fi].entry.rel.clone(),
                        start: d.line + 1,
                        end: d.end + 1,
                        name: d.name.clone(),
                    }
                })
                .collect(),
        });
    }
    for g in &mut groups {
        g.copies.sort_by(|a, b| (&a.path, a.start).cmp(&(&b.path, b.start)));
    }
    groups.sort_by(|a, b| b.reclaim.cmp(&a.reclaim).then_with(|| a.copies[0].path.cmp(&b.copies[0].path)));
    groups
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizing_ignores_layout_and_the_name() {
        let a = normalize("def area(w, h):\n    return w * h  \n", "area");
        let b = normalize("def size(w,h):\n\treturn w*h\n", "size");
        assert_eq!(a, b);
        assert_ne!(a, normalize("def size(w, h):\n    return w + h\n", "size"));
    }
}
