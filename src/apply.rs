//! Turns edits into new file contents: takes lines out, then tidies what the removal left behind
//! (imports nothing uses any more, a Python block with nothing in it).

use crate::candidate::Eco;
use crate::defs::indent_of;
use crate::find::deps;
use crate::lang::{Lang, Spec};
use crate::lex;
use anyhow::Result;
use regex::Regex;
use std::collections::HashMap;
use std::sync::OnceLock;

/// The text without lines start..=end of each range. The blank lines around a removed block are
/// squeezed so the neighbours end up as far apart as they were from it.
pub fn remove_lines(text: &str, ranges: &[(usize, usize)]) -> String {
    let lines: Vec<&str> = text.split_inclusive('\n').collect();
    let n = lines.len();
    let mut gone = vec![false; n];
    for &(start, end) in ranges {
        let end = end.min(n.saturating_sub(1));
        if start <= end {
            gone[start..=end].fill(true);
        }
    }
    let blank = |i: usize| lines[i].trim().is_empty();
    let mut i = 0;
    while i < n {
        if !gone[i] {
            i += 1;
            continue;
        }
        // one seam: removed blocks with nothing but blank lines between them
        let first = i;
        let mut last = i;
        let mut j = i + 1;
        while j < n && (gone[j] || blank(j)) {
            if gone[j] {
                last = j;
            }
            j += 1;
        }
        gone[first..=last].fill(true);
        let own = indent_of(lines[first]);
        let before = (0..first).rev().take_while(|&k| blank(k)).count();
        let after = (last + 1..n).take_while(|&k| blank(k)).count();
        let prev = first - before; // lines[prev - 1] is the code above, if any
        let next = last + 1 + after; // lines[next] is the code below, if any
        let (drop_before, drop_after) = if next >= n || prev == 0 {
            // nothing below or nothing above: the file ends, or starts, at the code that's left
            (before, after)
        } else if indent_of(lines[next]) < own || after >= before {
            // last in its block (the gap below belongs to the block), or the wider gap is below
            (before, 0)
        } else {
            (0, after)
        };
        gone[first - drop_before..first].fill(true);
        gone[last + 1..last + 1 + drop_after].fill(true);
        i = next.max(last + 1);
    }
    let out: String = lines.iter().zip(&gone).filter(|(_, g)| !**g).map(|(l, _)| *l).collect();
    if out.trim().is_empty() { String::new() } else { out }
}

/// One import line: what it brings in, and how to write it back with less.
struct Import {
    line: usize,
    /// Text before the first item.
    head: String,
    items: Vec<Item>,
    /// Text after the last item, line ending included.
    tail: String,
}

struct Item {
    /// As written: "a as b".
    text: String,
    /// The names the code would use. Any one in use keeps the item.
    names: Vec<String>,
    slot: Slot,
}

#[derive(Clone, Copy, PartialEq)]
enum Slot {
    /// The item is the whole line.
    Whole,
    Listed,
    JsDefault,
    JsNamed,
    JsNamespace,
}

/// The names a Go import path can go by in code: "gopkg.in/yaml.v3" is yaml, ".../go-chi/chi/v5" is chi.
fn go_names(path: &str) -> Vec<String> {
    let mut parts = path.rsplit('/');
    let mut base = parts.next().unwrap_or_default();
    if base.len() > 1 && base.starts_with('v') && base[1..].bytes().all(|b| b.is_ascii_digit()) {
        base = parts.next().unwrap_or(base);
    }
    let base = base.split('.').next().unwrap_or(base);
    let mut out = vec![base.to_string()];
    for alt in
        [base.trim_start_matches("go-").to_string(), base.replace('-', "_"), base.trim_end_matches("-go").to_string()]
    {
        if !out.contains(&alt) {
            out.push(alt);
        }
    }
    out
}

fn line_ending(line: &str) -> &str {
    if line.ends_with("\r\n") {
        "\r\n"
    } else if line.ends_with('\n') {
        "\n"
    } else {
        ""
    }
}

fn imports(spec: &Spec, raw: &[&str], code: &[&str]) -> Vec<Import> {
    static GO: OnceLock<Regex> = OnceLock::new();
    static JS: OnceLock<Regex> = OnceLock::new();
    let mut out = Vec::new();
    let whole = |line: usize, names: Vec<String>| Import {
        line,
        head: String::new(),
        items: vec![Item { text: raw[line].to_string(), names, slot: Slot::Whole }],
        tail: String::new(),
    };
    match spec.name {
        "Go" => {
            let re =
                GO.get_or_init(|| Regex::new(r#"^\s*(import\s+)?(?:([\w.]+)\s+)?"([^"]+)"\s*$"#).expect("pattern"));
            let mut block = false;
            for (i, line) in raw.iter().enumerate() {
                let t = code[i].trim();
                if t.starts_with("import (") || t == "import(" {
                    block = true;
                    continue;
                }
                if block && t.starts_with(')') {
                    block = false;
                    continue;
                }
                let Some(c) = re.captures(line.trim_end()) else { continue };
                if !block && c.get(1).is_none() {
                    continue;
                }
                match c.get(2).map(|m| m.as_str()) {
                    Some("_" | ".") => {}
                    Some(alias) => out.push(whole(i, vec![alias.to_string()])),
                    None => out.push(whole(i, go_names(&c[3]))),
                }
            }
        }
        "Python" => {
            for (i, line) in code.iter().enumerate() {
                let t = line.trim();
                let pad = &line[..line.len() - line.trim_start().len()];
                if let Some(rest) = t.strip_prefix("import ") {
                    if rest.contains(',') {
                        continue;
                    }
                    let name = match rest.split_once(" as ") {
                        Some((_, alias)) => alias.trim(),
                        None => rest.split('.').next().unwrap_or(rest).trim(),
                    };
                    out.push(whole(i, vec![name.to_string()]));
                } else if let Some((module, list)) = t.strip_prefix("from ").and_then(|r| r.split_once(" import ")) {
                    let list = list.trim().trim_start_matches('(').trim_end_matches(')');
                    if module.trim() == "__future__" || list.contains(['*', '(', '\\']) || list.trim().is_empty() {
                        continue;
                    }
                    let items = list
                        .split(',')
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .map(|text| {
                            let name = text.rsplit(" as ").next().unwrap_or(text).trim();
                            Item { text: text.to_string(), names: vec![name.to_string()], slot: Slot::Listed }
                        })
                        .collect();
                    out.push(Import {
                        line: i,
                        head: format!("{pad}from {} import ", module.trim()),
                        items,
                        tail: line_ending(raw[i]).to_string(),
                    });
                }
            }
        }
        "JavaScript" | "TypeScript" => {
            let re = JS.get_or_init(|| {
                Regex::new(r"^(\s*import\s+(?:type\s+)?)(?:([A-Za-z_]\w*)\s*,?\s*)?(?:\{([^}]*)\}|\*\s+as\s+([A-Za-z_]\w*))?\s*(from\b.*)$")
                    .expect("pattern")
            });
            for (i, line) in code.iter().enumerate() {
                let Some(c) = re.captures(line.trim_end()) else { continue };
                let mut items = Vec::new();
                if let Some(default) = c.get(2) {
                    items.push(Item {
                        text: default.as_str().to_string(),
                        names: vec![default.as_str().to_string()],
                        slot: Slot::JsDefault,
                    });
                }
                for text in c.get(3).map_or("", |m| m.as_str()).split(',').map(str::trim).filter(|s| !s.is_empty()) {
                    let name = text.rsplit(" as ").next().unwrap_or(text).trim().trim_start_matches("type ").trim();
                    items.push(Item { text: text.to_string(), names: vec![name.to_string()], slot: Slot::JsNamed });
                }
                if let Some(ns) = c.get(4) {
                    items.push(Item {
                        text: format!("* as {}", ns.as_str()),
                        names: vec![ns.as_str().to_string()],
                        slot: Slot::JsNamespace,
                    });
                }
                if items.is_empty() {
                    continue;
                }
                // the module name is a string, blanked in code: take the tail from the real line
                let from = c.get(5).map_or(0, |m| m.start());
                out.push(Import { line: i, head: c[1].to_string(), items, tail: raw[i][from..].to_string() });
            }
        }
        _ => {}
    }
    out
}

fn rewrite(import: &Import, keep: &[bool]) -> Option<String> {
    let kept = |slot: Slot| -> Vec<&str> {
        import
            .items
            .iter()
            .zip(keep)
            .filter(|(item, k)| **k && item.slot == slot)
            .map(|(item, _)| item.text.as_str())
            .collect()
    };
    if !keep.contains(&true) {
        return None;
    }
    let mut parts: Vec<String> = Vec::new();
    parts.extend(kept(Slot::Whole).iter().map(|s| s.to_string()));
    parts.extend(kept(Slot::Listed).iter().map(|s| s.to_string()));
    parts.extend(kept(Slot::JsDefault).iter().map(|s| s.to_string()));
    parts.extend(kept(Slot::JsNamespace).iter().map(|s| s.to_string()));
    let named = kept(Slot::JsNamed);
    if !named.is_empty() {
        parts.push(format!("{{ {} }}", named.join(", ")));
    }
    let js = import.items.iter().any(|i| matches!(i.slot, Slot::JsDefault | Slot::JsNamed | Slot::JsNamespace));
    Some(format!("{}{}{}{}", import.head, parts.join(", "), if js { " " } else { "" }, import.tail))
}

fn uses(spec: &Spec, text: &str) -> (Vec<Import>, HashMap<String, u32>, Vec<String>) {
    let code = lex::lex(text, spec).code;
    let raw: Vec<&str> = text.split_inclusive('\n').collect();
    let code_lines: Vec<&str> = code.split_inclusive('\n').collect();
    let found = imports(spec, &raw, &code_lines);
    let mut counts = HashMap::new();
    for (i, line) in code_lines.iter().enumerate() {
        if !found.iter().any(|imp| imp.line == i) {
            lex::idents(line, |t| *counts.entry(t.to_string()).or_insert(0) += 1);
        }
    }
    (found, counts, raw.iter().map(|l| l.to_string()).collect())
}

/// Drops imports that were in use before the removal and aren't after it. An import that was
/// already unused is left alone: it isn't ours to judge.
fn prune_imports(before: &str, after: &str, spec: &Spec) -> String {
    if !matches!(spec.name, "Go" | "Python" | "JavaScript" | "TypeScript") {
        return after.to_string();
    }
    let (_, used_before, _) = uses(spec, before);
    let (found, used_after, mut lines) = uses(spec, after);
    let mut dropped: Vec<(usize, usize)> = Vec::new();
    let mut rewritten = false;
    for import in &found {
        let keep: Vec<bool> = import
            .items
            .iter()
            .map(|item| {
                let was = item.names.iter().any(|n| used_before.contains_key(n));
                let is = item.names.iter().any(|n| used_after.contains_key(n));
                is || !was
            })
            .collect();
        if keep.iter().all(|k| *k) {
            continue;
        }
        match rewrite(import, &keep) {
            Some(line) => {
                lines[import.line] = line;
                rewritten = true;
            }
            None => dropped.push((import.line, import.line)),
        }
    }
    if dropped.is_empty() && !rewritten {
        return after.to_string();
    }
    // whole lines go the same way definitions do, so the spacing around them is kept
    let out = remove_lines(&lines.concat(), &dropped);
    if spec.name == "Go" {
        // every import in the block went: the block goes too
        static EMPTY: OnceLock<Regex> = OnceLock::new();
        let empty = EMPTY.get_or_init(|| Regex::new(r"(?m)^import \(\s*\)[ \t]*\r?\n(\r?\n)?").expect("pattern"));
        return empty.replace(&out, "").into_owned();
    }
    out
}

/// A class or def left with no body isn't valid Python: give it a pass.
fn fill_empty_python_blocks(text: &str, spec: &Spec) -> String {
    const OPENERS: &[&str] =
        &["class", "def", "async", "if", "elif", "else", "for", "while", "try", "except", "finally", "with"];
    let code = lex::lex(text, spec).code;
    let raw: Vec<&str> = text.split_inclusive('\n').collect();
    let code_lines: Vec<&str> = code.split_inclusive('\n').collect();
    let mut out = String::with_capacity(text.len());
    for (i, line) in raw.iter().enumerate() {
        out.push_str(line);
        let c = code_lines[i].trim();
        let first: String = c.chars().take_while(|ch| ch.is_alphanumeric() || *ch == '_').collect();
        if !c.ends_with(':') || !(OPENERS.contains(&first.as_str()) || c.starts_with(')')) {
            continue;
        }
        let own = indent_of(line);
        let body = (i + 1..raw.len()).find(|&j| {
            let r = raw[j].trim();
            !r.is_empty() && !(code_lines[j].trim().is_empty() && r.starts_with('#'))
        });
        if body.is_none_or(|j| indent_of(raw[j]) <= own) {
            let pad = &line[..line.len() - line.trim_start().len()];
            let ending = if line.ends_with("\r\n") { "\r\n" } else { "\n" };
            if !line.ends_with('\n') {
                out.push_str(ending);
            }
            out.push_str(&format!("{pad}    pass{ending}"));
        }
    }
    out
}

/// A file after its edits: lines out, then dependencies out.
pub fn render(
    original: &str,
    lang: Option<&'static Lang>,
    ranges: &[(usize, usize)],
    dependencies: &[(Eco, String, String)],
) -> Result<String> {
    let mut text = original.to_string();
    if !ranges.is_empty() {
        text = remove_lines(original, ranges);
        if let Some(lang) = lang {
            text = prune_imports(original, &text, lang.spec);
            if lang.spec.name == "Python" {
                text = fill_empty_python_blocks(&text, lang.spec);
            }
        }
    }
    for (eco, section, name) in dependencies {
        text = deps::remove(&text, *eco, section, name)?;
    }
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lang::for_path;

    fn render_as(file: &str, text: &str, ranges: &[(usize, usize)]) -> String {
        render(text, for_path(file), ranges, &[]).unwrap()
    }

    #[test]
    fn removal_keeps_the_spacing() {
        let py = "import os\n\n\ndef a():\n    pass\n\n\ndef b():\n    pass\n\n\ndef c():\n    pass\n";
        assert_eq!(remove_lines(py, &[(7, 8)]), "import os\n\n\ndef a():\n    pass\n\n\ndef c():\n    pass\n");
        assert_eq!(remove_lines(py, &[(11, 12)]), "import os\n\n\ndef a():\n    pass\n\n\ndef b():\n    pass\n");
        assert_eq!(remove_lines(py, &[(3, 4), (7, 8)]), "import os\n\n\ndef c():\n    pass\n");
        let js = "class K {\n  a() {}\n\n  b() {}\n}\n\nfunction f() {}\n";
        assert_eq!(remove_lines(js, &[(3, 3)]), "class K {\n  a() {}\n}\n\nfunction f() {}\n");
        assert_eq!(remove_lines("x\n", &[(0, 0)]), "");
        // a wider gap below than above is kept, and the end of the file doesn't collect blank lines
        let consts =
            "import os\n\nA = 1\nB = 2\n\n\nclass K:\n    pass\n\n\ndef f():\n    pass\n\n\ndef g():\n    pass\n";
        assert_eq!(remove_lines(consts, &[(2, 2), (3, 3), (10, 11), (14, 15)]), "import os\n\n\nclass K:\n    pass\n");
        assert_eq!(remove_lines("# top\n\nA = 1\n\nB = 2\n", &[(2, 2)]), "# top\n\nB = 2\n");
    }

    #[test]
    fn python_gets_a_pass_and_loses_dead_imports() {
        let py = "import os\nimport sys\nfrom json import dumps, loads as parse\n\n\nclass K:\n    def only(self):\n        return os.getcwd() + parse('1')\n\n\nprint(sys.argv, dumps({}))\n";
        let out = render_as("m.py", py, &[(6, 7)]);
        assert_eq!(out, "import sys\nfrom json import dumps\n\n\nclass K:\n    pass\n\n\nprint(sys.argv, dumps({}))\n");
    }

    #[test]
    fn go_imports_follow_the_code() {
        let go = "package a\n\nimport (\n\t\"fmt\"\n\t\"strings\"\n\tyaml \"gopkg.in/yaml.v3\"\n\t_ \"embed\"\n)\n\nfunc used() { fmt.Println() }\n\nfunc dead() string {\n\treturn strings.ToUpper(string(yaml.Node{}.Value))\n}\n";
        let out = render_as("a.go", go, &[(11, 13)]);
        assert_eq!(out, "package a\n\nimport (\n\t\"fmt\"\n\t_ \"embed\"\n)\n\nfunc used() { fmt.Println() }\n");
        let go = "package a\n\nimport (\n\t\"strings\"\n)\n\ntype T int\n\nfunc dead() string {\n\treturn strings.ToUpper(\"x\")\n}\n";
        assert_eq!(render_as("a.go", go, &[(8, 10)]), "package a\n\ntype T int\n");
    }

    #[test]
    fn js_imports_are_rewritten() {
        let js = "import def, { a, b as c } from './x';\nimport * as ns from \"y\";\nimport 'side-effect';\nimport unusedAlready from 'z';\n\nexport function keep() { return a(); }\n\nfunction dead() { return def(c, ns.q); }\n";
        let out = render_as("a.js", js, &[(7, 7)]);
        assert_eq!(
            out,
            "import { a } from './x';\nimport 'side-effect';\nimport unusedAlready from 'z';\n\nexport function keep() { return a(); }\n"
        );
    }
}
