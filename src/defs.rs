//! Finds definitions and the lines each one covers, from the blanked code.

use crate::lang::{Block, Kind, Lang};
use serde::{Deserialize, Serialize};

/// A span longer than this is a matching mistake, not a definition.
const MAX_SPAN: usize = 1500;

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug, Serialize, Deserialize)]
pub enum Decor {
    None,
    /// Decorators that don't call or register anything (#[inline], @staticmethod).
    Benign,
    /// A decorator that may register the definition somewhere (@app.route, #[test]).
    Registered,
}

#[derive(Clone, Debug)]
pub struct Def {
    pub name: String,
    pub kind: Kind,
    /// The line with the name (0-based).
    pub line: usize,
    /// First line to remove: decorators and doc comments above count.
    pub start: usize,
    /// Last line to remove.
    pub end: usize,
    pub indent: usize,
    pub decor: Decor,
    /// The definition line, trimmed, comments and strings blanked.
    pub header: String,
    /// The line that opens the block this definition sits in.
    pub parent: String,
}

/// Words that start a statement, never a definition.
const STATEMENT: &[&str] = &[
    "return", "await", "yield", "throw", "new", "else", "case", "if", "for", "while", "switch", "in", "is", "as", "do",
    "try", "catch", "finally", "delete", "typeof", "assert", "raise", "print", "echo", "go", "defer", "break",
    "continue", "goto", "import", "from", "package", "using", "elif", "except", "with", "until", "unless",
];

/// Words a pattern can mistake for a name.
const RESERVED: &[&str] = &[
    "if",
    "for",
    "while",
    "switch",
    "catch",
    "function",
    "return",
    "else",
    "do",
    "try",
    "with",
    "new",
    "func",
    "fun",
    "def",
    "class",
    "struct",
    "enum",
    "interface",
    "type",
    "const",
    "let",
    "var",
    "static",
    "public",
    "private",
    "async",
    "await",
    "in",
    "of",
    "super",
    "this",
    "self",
    "impl",
    "fn",
    "mod",
    "use",
    "match",
    "loop",
    "where",
    "end",
    "then",
    "local",
    "not",
    "and",
    "or",
    "nil",
    "null",
    "true",
    "false",
];

const BENIGN: &[&str] = &[
    "staticmethod",
    "classmethod",
    "property",
    "cache",
    "lru_cache",
    "cached_property",
    "dataclass",
    "final",
    "inline",
    "derive",
    "allow",
    "expect",
    "warn",
    "must_use",
    "doc",
    "repr",
    "non_exhaustive",
    "track_caller",
    "cold",
    "automatically_derived",
    "Deprecated",
    "deprecated",
    "SuppressWarnings",
    "Suppress",
    "Nullable",
    "NonNull",
    "NotNull",
    "JvmStatic",
    "Obsolete",
    "spec",
    "moduledoc",
    "nodiscard",
    "maybe_unused",
    "template",
    "discardableResult",
    "inlinable",
];

pub fn indent_of(line: &str) -> usize {
    let mut n = 0;
    for c in line.chars() {
        match c {
            ' ' => n += 1,
            '\t' => n += 4,
            _ => break,
        }
    }
    n
}

fn continues(t: &str) -> bool {
    t.ends_with(['=', '+', '-', '*', '/', ',', '?', ':', '.', '|', '&', '\\'])
        || t.ends_with("=>")
        || t.ends_with("->")
        || [" where", " extends", " implements", " throws"].iter().any(|w| t.ends_with(w))
}

fn next_continues(code: &[&str], i: usize) -> bool {
    let Some(next) = code[i + 1..].iter().map(|l| l.trim()).find(|l| !l.is_empty()) else { return false };
    next.starts_with(['{', '.', '|', '&', '?', ':', '+', '-', '*', '/', '=', ',', ')', ']'])
        || ["where", "extends", "implements", "throws"].iter().any(|w| next.starts_with(w))
}

/// The last line of a statement that starts at start, and whether it had a block of its own.
fn brace_span(code: &[&str], start: usize) -> Option<(usize, bool)> {
    let mut depth = 0i32;
    let mut block = false;
    for i in start..code.len().min(start + MAX_SPAN) {
        for ch in code[i].bytes() {
            match ch {
                b'{' => {
                    block |= depth == 0;
                    depth += 1;
                }
                b'(' | b'[' => depth += 1,
                b')' | b']' | b'}' => {
                    depth -= 1;
                    if depth < 0 {
                        return None;
                    }
                }
                _ => {}
            }
        }
        if depth > 0 {
            continue;
        }
        if block {
            return Some((i, true));
        }
        let t = code[i].trim();
        if t.is_empty() || continues(t) || next_continues(code, i) {
            continue;
        }
        return Some((i, false));
    }
    None
}

fn indent_span(code: &[&str], raw: &[&str], start: usize) -> Option<usize> {
    let own = indent_of(code[start]);
    // the header may run over several lines: def f(\n a,\n) -> int:
    let mut depth = 0i32;
    let mut i = start;
    loop {
        for ch in code[i].bytes() {
            match ch {
                b'(' | b'[' | b'{' => depth += 1,
                b')' | b']' | b'}' => depth -= 1,
                _ => {}
            }
        }
        if depth <= 0 {
            break;
        }
        i += 1;
        if i >= code.len() || i > start + 80 {
            return None;
        }
    }
    let mut end = i;
    for j in i + 1..code.len().min(start + MAX_SPAN) {
        if raw[j].trim().is_empty() {
            continue;
        }
        if code[j].trim().is_empty() {
            // a comment, or the inside of a string
            if indent_of(raw[j]) <= own && raw[j].trim_start().starts_with('#') {
                break;
            }
            end = j;
            continue;
        }
        if indent_of(code[j]) <= own {
            break;
        }
        end = j;
    }
    Some(end)
}

fn starts_with_word(t: &str, word: &str) -> bool {
    t.strip_prefix(word).is_some_and(|rest| !rest.starts_with(|c: char| c.is_alphanumeric() || c == '_'))
}

fn end_span(code: &[&str], start: usize) -> Option<usize> {
    let t = code[start].trim();
    let one_line = starts_with_word(t.rsplit(' ').next().unwrap_or_default(), "end")
        || t.contains(", do:")
        || (t.starts_with("def ") && (t.contains(") =") || t.split_whitespace().nth(2) == Some("=")));
    if one_line {
        return Some(start);
    }
    let own = indent_of(code[start]);
    for (j, line) in code.iter().enumerate().take(start + MAX_SPAN).skip(start + 1) {
        let t = line.trim();
        if t.is_empty() {
            continue;
        }
        let indent = indent_of(line);
        if indent < own {
            return None;
        }
        if indent == own {
            if starts_with_word(t, "end") {
                return Some(j);
            }
            if !["rescue", "ensure", "else", "elsif", "when", "after", "catch"].iter().any(|w| starts_with_word(t, w)) {
                return None;
            }
        }
    }
    None
}

fn decorator(raw: &str) -> Option<Decor> {
    let t = raw.trim();
    let body = if let Some(rest) = t.strip_prefix('@') {
        rest
    } else if let Some(rest) = t.strip_prefix("#![").or_else(|| t.strip_prefix("#[")) {
        rest
    } else if let Some(rest) = t.strip_prefix("[[") {
        rest
    } else if t.starts_with('[') && t.ends_with(']') {
        &t[1..]
    } else if t.starts_with("template") {
        "template"
    } else {
        return None;
    };
    let path: String = body.chars().take_while(|c| c.is_alphanumeric() || matches!(c, '_' | '.' | ':')).collect();
    let name = path.rsplit(['.', ':']).next().unwrap_or_default();
    Some(if BENIGN.contains(&name) { Decor::Benign } else { Decor::Registered })
}

/// The first line of a decorator that ends on line at, when it runs over several lines.
fn decorator_start(raw: &[&str], code: &[&str], at: usize) -> Option<usize> {
    let mut depth = 0i32;
    for k in (at.saturating_sub(40)..=at).rev() {
        for ch in code[k].bytes() {
            match ch {
                b')' | b']' => depth += 1,
                b'(' | b'[' => depth -= 1,
                _ => {}
            }
        }
        if depth <= 0 {
            return decorator(raw[k]).map(|_| k);
        }
    }
    None
}

/// Walks up over the decorators and comments that belong to the definition on line.
fn extend_up(raw: &[&str], code: &[&str], line: usize) -> (usize, Decor) {
    let mut start = line;
    let mut decor = Decor::None;
    while start > 0 {
        let prev = start - 1;
        let r = raw[prev].trim();
        if r.is_empty() {
            break;
        }
        if let Some(d) = decorator(r) {
            decor = decor.max(d);
            start = prev;
            continue;
        }
        let c = code[prev].trim();
        if c.is_empty() {
            // a comment line: part of the definition, and Go puts directives there
            if r.starts_with("//go:") || r.starts_with("//export") || r.contains("#pragma") {
                decor = Decor::Registered;
            }
            start = prev;
            continue;
        }
        if (c.ends_with([')', ']']) || c.starts_with(')'))
            && let Some(first) = decorator_start(raw, code, prev)
        {
            decor = Decor::Registered;
            start = first;
            continue;
        }
        break;
    }
    (start, decor)
}

fn parent_of(code: &[&str], line: usize, indent: usize) -> String {
    if indent == 0 {
        return String::new();
    }
    code[..line]
        .iter()
        .rev()
        .find(|l| {
            let t = l.trim();
            !t.is_empty() && t != "{" && indent_of(l) < indent
        })
        .map(|l| l.trim().to_string())
        .unwrap_or_default()
}

/// Every definition in a file. raw and code are the same lines, before and after blanking.
pub fn find(lang: &Lang, raw: &[&str], code: &[&str]) -> Vec<Def> {
    let spec = lang.spec;
    let c_family = matches!(spec.name, "C" | "C++");
    let js = matches!(spec.name, "JavaScript" | "TypeScript");
    let mut defs: Vec<Def> = Vec::new();
    for (i, line) in code.iter().enumerate() {
        let t = line.trim();
        if t.is_empty() {
            continue;
        }
        let first: String = t.chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
        if STATEMENT.contains(&first.as_str()) {
            continue;
        }
        let mut seen: Vec<&str> = Vec::new();
        for (re, kind) in &lang.defs {
            let Some(name) = re.captures(line).and_then(|c| c.name("name")) else { continue };
            let name = name.as_str();
            if RESERVED.contains(&name) || seen.contains(&name) {
                continue;
            }
            if js && *kind == Kind::Method && (t.contains("function") || t.contains("=>")) {
                continue; // a call that takes a callback, not a method
            }
            let mut kind = *kind;
            let end = match (spec.block, kind) {
                (Block::Brace, _) | (_, Kind::Const) => brace_span(code, i).map(|(end, block)| {
                    if c_family && kind == Kind::Function && !block {
                        kind = Kind::Attached; // a prototype
                    }
                    end
                }),
                (Block::Indent, _) => indent_span(code, raw, i),
                (Block::End, _) => end_span(code, i),
            };
            let Some(end) = end else { continue };
            seen.push(name);
            let indent = indent_of(line);
            let (start, decor) = extend_up(raw, code, i);
            defs.push(Def {
                name: name.trim_start_matches('#').to_string(),
                kind,
                line: i,
                start,
                end,
                indent,
                decor,
                header: t.to_string(),
                parent: parent_of(code, i, indent),
            });
        }
    }
    // A span that runs into a sibling's definition matched the wrong closing bracket.
    let overran: Vec<bool> = defs
        .iter()
        .enumerate()
        .map(|(k, a)| {
            defs[k + 1..].iter().take_while(|b| b.line <= a.end).any(|b| b.line > a.line && b.indent <= a.indent)
        })
        .collect();
    let mut keep = overran.iter().map(|o| !o);
    defs.retain(|_| keep.next().unwrap_or(false));
    defs
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lang::for_path;
    use crate::lex::lex;

    fn defs(file: &str, text: &str) -> Vec<Def> {
        let lang = for_path(file).unwrap();
        let code = lex(text, lang.spec).code;
        find(lang, &text.split('\n').collect::<Vec<_>>(), &code.split('\n').collect::<Vec<_>>())
    }

    fn spans(file: &str, text: &str) -> Vec<(String, usize, usize)> {
        defs(file, text).into_iter().map(|d| (d.name, d.start, d.end)).collect()
    }

    #[test]
    fn python_blocks_end_where_the_indent_does() {
        let src = "import os\n\n# helper\n@staticmethod\ndef a(x,\n      y):\n    \"\"\"doc\nflush left\n    \"\"\"\n    return x\n\n\ndef b(): return 1\nX_MAX = [\n    1,\n]\n";
        assert_eq!(
            spans("m.py", src),
            [("a".to_string(), 2, 9), ("b".to_string(), 12, 12), ("X_MAX".to_string(), 13, 15)]
        );
    }

    #[test]
    fn decorators_that_register_are_noticed_even_over_several_lines() {
        let d = defs("m.py", "@app.route(\n    '/x',\n)\ndef view():\n    pass\n\n@property\ndef p(self):\n    pass\n");
        assert_eq!((d[0].start, d[0].decor), (0, Decor::Registered));
        assert_eq!(d[1].decor, Decor::Benign);
        let d = defs("a.rs", "#[inline]\nfn a() {}\n\n#[test]\nfn b() {}\n");
        assert_eq!((d[0].decor, d[1].decor), (Decor::Benign, Decor::Registered));
    }

    #[test]
    fn brace_languages() {
        let js = "export function a(x) {\n  return { y: x };\n}\nconst b = (x) =>\n  x + 1\nconst c = async (x) => {\n  await x;\n};\nclass K {\n  m(a) {\n    it('x', function() {\n    });\n  }\n}\n";
        assert_eq!(
            spans("a.js", js),
            [
                ("a".to_string(), 0, 2),
                ("b".to_string(), 3, 4),
                ("c".to_string(), 5, 7),
                ("K".to_string(), 8, 13),
                ("m".to_string(), 9, 12)
            ]
        );
        let rs = "/// doc\npub fn a()\n    -> u8\n{\n    1\n}\nstruct S;\nimpl S {\n    fn m(&self) {}\n}\n";
        let d = defs("a.rs", rs);
        assert_eq!((d[0].start, d[0].end), (0, 5));
        assert_eq!((d[1].name.as_str(), d[1].end), ("S", 6));
        assert_eq!((d[2].kind, d[2].name.as_str(), d[2].end), (Kind::Attached, "S", 9));
        assert_eq!((d[3].name.as_str(), d[3].parent.as_str()), ("m", "impl S {"));
        let go = "func (s *Srv) Run(a int) error {\n\treturn nil\n}\n\ntype Srv struct {\n\tx int\n}\n";
        let d = defs("a.go", go);
        assert_eq!(
            d.iter().map(|d| (d.name.as_str(), d.kind)).collect::<Vec<_>>(),
            [("Run", Kind::Method), ("Srv", Kind::Attached), ("Srv", Kind::Type)]
        );
    }

    #[test]
    fn c_prototypes_attach_to_the_function() {
        let d = defs("a.c", "int add(int a, int b);\n\nint add(int a, int b)\n{\n    return a + b;\n}\n");
        assert_eq!((d[0].kind, d[0].end), (Kind::Attached, 0));
        assert_eq!((d[1].kind, d[1].start, d[1].end), (Kind::Function, 2, 5));
    }

    #[test]
    fn end_languages() {
        let lua = "local function a(x)\n  if x then\n    return 1\n  end\nend\n\nfunction M.b() return 2 end\nM.c = function()\nend\n";
        assert_eq!(spans("a.lua", lua), [("a".to_string(), 0, 4), ("b".to_string(), 6, 6), ("c".to_string(), 7, 8)]);
        let rb = "class A\n  def x\n    1\n  rescue\n    2\n  end\n  def y = 3\nend\n";
        assert_eq!(spans("a.rb", rb), [("A".to_string(), 0, 7), ("x".to_string(), 1, 5), ("y".to_string(), 6, 6)]);
    }

    #[test]
    fn a_span_that_swallows_a_sibling_is_dropped() {
        // the stray brace in the regex-free language makes a() look unclosed
        let d = defs("a.go", "func a() {\n\tx := '{'\n\t_ = x\n}\n\nfunc b() {\n}\n");
        assert_eq!(d.iter().map(|d| d.name.as_str()).collect::<Vec<_>>(), ["a", "b"]);
        let d = defs("a.kt", "fun a() {\n  val r = x /{/ y\n}\nfun b() {\n}\n");
        assert_eq!(d.iter().map(|d| d.name.as_str()).collect::<Vec<_>>(), ["b"]);
    }
}
