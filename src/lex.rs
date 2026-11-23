//! One scanner for every language: finds comments and strings, so the rest only ever looks at code.
//!
//! It returns two copies of the text, both the same length as the original with line breaks kept:
//! code, where comments and strings are blanked (for finding definitions and matching brackets), and
//! refs, where only comments are blanked (for counting mentions: a name in a string still counts).

use crate::lang::Spec;

pub struct Lexed {
    pub code: String,
    pub refs: String,
    /// Starts of strings that end in _, like "handle_": names built at run time.
    pub prefixes: Vec<String>,
    /// Folder and file names that appear in path-like strings.
    pub path_parts: Vec<String>,
}

pub fn is_ident(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

/// Calls f with every identifier in text.
pub fn idents(text: &str, mut f: impl FnMut(&str)) {
    let b = text.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i].is_ascii_alphabetic() || b[i] == b'_' {
            let start = i;
            while i < b.len() && is_ident(b[i]) {
                i += 1;
            }
            f(&text[start..i]);
        } else if b[i].is_ascii_digit() {
            // a number, with any letters glued to it
            while i < b.len() && is_ident(b[i]) {
                i += 1;
            }
        } else {
            i += 1;
        }
    }
}

/// How many times word appears in text as a whole identifier.
pub fn count_word(text: &str, word: &str) -> u32 {
    let mut n = 0;
    idents(text, |t| n += u32::from(t == word));
    n
}

fn blank(buf: &mut [u8], from: usize, to: usize) {
    for b in &mut buf[from..to] {
        if *b != b'\n' && *b != b'\r' {
            *b = b' ';
        }
    }
}

fn find(b: &[u8], from: usize, needle: &[u8]) -> Option<usize> {
    if from >= b.len() {
        return None;
    }
    b[from..].windows(needle.len()).position(|w| w == needle).map(|p| p + from)
}

fn line_end(b: &[u8], from: usize) -> usize {
    b[from..].iter().position(|&c| c == b'\n').map_or(b.len(), |p| p + from)
}

/// The end of a Lua long bracket ([[ ... ]] or [==[ ... ]==]) opening at at.
fn lua_long(b: &[u8], at: usize) -> Option<usize> {
    if b.get(at) != Some(&b'[') {
        return None;
    }
    let mut j = at + 1;
    while b.get(j) == Some(&b'=') {
        j += 1;
    }
    if b.get(j) != Some(&b'[') {
        return None;
    }
    let mut close = vec![b']'];
    close.extend(std::iter::repeat_n(b'=', j - at - 1));
    close.push(b']');
    Some(find(b, j + 1, &close).map_or(b.len(), |p| p + close.len()))
}

/// The end of a Rust raw string (r"..." or r#"..."#) starting at the r.
fn rust_raw(b: &[u8], i: usize) -> Option<usize> {
    let mut j = i + 1;
    while b.get(j) == Some(&b'#') {
        j += 1;
    }
    if b.get(j) != Some(&b'"') {
        return None;
    }
    let mut close = vec![b'"'];
    close.extend(std::iter::repeat_n(b'#', j - i - 1));
    Some(find(b, j + 1, &close).map_or(b.len(), |p| p + close.len()))
}

/// The end of a Rust char literal at the quote, or None for a lifetime.
fn rust_char(b: &[u8], i: usize) -> Option<usize> {
    let next = *b.get(i + 1)?;
    if next == b'\\' {
        return b[i + 2..].iter().take(12).position(|&c| c == b'\'').map(|p| i + 2 + p + 1);
    }
    let len = match next {
        0x00..0x80 => 1,
        0xC0..0xE0 => 2,
        0xE0..0xF0 => 3,
        _ => 4,
    };
    (b.get(i + 1 + len) == Some(&b'\'')).then_some(i + 2 + len)
}

/// The end of a JavaScript regex literal at the slash, or None for a division.
fn js_regex(b: &[u8], i: usize) -> Option<usize> {
    let mut k = i;
    while k > 0 && matches!(b[k - 1], b' ' | b'\t') {
        k -= 1;
    }
    let starts_expression = if k == 0 || b[k - 1] == b'\n' || b"(,=:[!&|?{};+-*%>~^".contains(&b[k - 1]) {
        true
    } else {
        let mut w = k;
        while w > 0 && is_ident(b[w - 1]) {
            w -= 1;
        }
        matches!(&b[w..k], b"return" | b"typeof" | b"case" | b"in" | b"of" | b"void" | b"delete")
    };
    if !starts_expression {
        return None;
    }
    let mut j = i + 1;
    let mut class = false;
    while j < b.len() && b[j] != b'\n' {
        match b[j] {
            b'\\' => j += 1,
            b'[' => class = true,
            b']' => class = false,
            b'/' if !class => {
                j += 1;
                while j < b.len() && b[j].is_ascii_alphabetic() {
                    j += 1;
                }
                return Some(j);
            }
            _ => {}
        }
        j += 1;
    }
    None
}

/// (end, length of each delimiter) of the string whose opening quote is at i.
fn string_end(b: &[u8], i: usize, spec: &Spec) -> (usize, usize) {
    let q = b[i];
    let n = b.len();
    let raw = q == b'\x60' && spec.name == "Go";
    if spec.triple && b.get(i + 1) == Some(&q) && b.get(i + 2) == Some(&q) {
        let mut j = i + 3;
        while j < n {
            if b[j] == b'\\' {
                j += 2;
                continue;
            }
            if b[j] == q && b.get(j + 1) == Some(&q) && b.get(j + 2) == Some(&q) {
                return (j + 3, 3);
            }
            j += 1;
        }
        return (n, 3);
    }
    let mut j = i + 1;
    while j < n {
        let c = b[j];
        if c == b'\\' && !raw {
            j += 2;
            continue;
        }
        if c == q {
            return (j + 1, 1);
        }
        if c == b'\n' && !spec.multiline && q != b'\x60' {
            return (j, 1);
        }
        j += 1;
    }
    (n, 1)
}

fn record(content: &str, prefixes: &mut Vec<String>, path_parts: &mut Vec<String>) {
    if content.is_empty() || content.len() > 200 {
        return;
    }
    let head = content.split(['{', '$', '%', '#']).next().unwrap_or_default();
    if head.len() >= 3 && head.ends_with('_') && head.bytes().all(is_ident) {
        prefixes.push(head.to_string());
    }
    // A folder reached by path: every folder of a glob ("plugins/*.py"), or a path that ends at a
    // folder ("./commands"). "../src/cart.ts" is an import of one file and says nothing about src.
    if content.contains(['/', '*']) || content.starts_with('.') {
        let glob = content.contains('*');
        let parts: Vec<&str> = content.split(['/', '\\']).filter(|p| !p.is_empty() && !p.starts_with('.')).collect();
        for (i, part) in parts.iter().enumerate() {
            let folder = glob || (i + 1 == parts.len() && !part.contains('.'));
            let part = part.split('.').next().unwrap_or_default();
            if folder && part.len() >= 3 && part.bytes().all(|c| is_ident(c) || c == b'-') {
                path_parts.push(part.to_string());
                if glob {
                    // a pattern reads every file in the folder; a plain path may only copy it
                    path_parts.push(format!("*{part}"));
                }
            }
        }
    }
}

pub fn lex(text: &str, spec: &Spec) -> Lexed {
    let b = text.as_bytes();
    let n = b.len();
    let mut code = b.to_vec();
    let mut refs = b.to_vec();
    let mut prefixes = Vec::new();
    let mut path_parts = Vec::new();
    let js = matches!(spec.name, "JavaScript" | "TypeScript");
    let rust = spec.name == "Rust";
    let lua = matches!(spec.name, "Lua" | "Luau");
    let php = spec.name == "PHP";
    let mut i = 0;
    while i < n {
        let c = b[i];
        if lua {
            let comment = b[i..].starts_with(b"--[");
            if (comment || c == b'[')
                && let Some(end) = lua_long(b, if comment { i + 2 } else { i })
            {
                blank(&mut code, i, end);
                if comment {
                    blank(&mut refs, i, end);
                }
                i = end;
                continue;
            }
        }
        if let Some((open, close)) = spec.block_comment
            && b[i..].starts_with(open.as_bytes())
        {
            let end = find(b, i + open.len(), close.as_bytes()).map_or(n, |p| p + close.len());
            blank(&mut code, i, end);
            blank(&mut refs, i, end);
            i = end;
            continue;
        }
        if let Some(lc) = spec.line_comments.iter().find(|lc| b[i..].starts_with(lc.as_bytes())) {
            // A # only opens a comment at the start of a word: $# and #[attr] are code.
            let hash = *lc == "#";
            let ok = !hash || ((i == 0 || b[i - 1].is_ascii_whitespace()) && !(php && b.get(i + 1) == Some(&b'[')));
            if ok {
                let end = line_end(b, i);
                blank(&mut code, i, end);
                blank(&mut refs, i, end);
                i = end;
                continue;
            }
        }
        if rust
            && c == b'r'
            && (i == 0 || !is_ident(b[i - 1]) || (b[i - 1] == b'b' && (i < 2 || !is_ident(b[i - 2]))))
            && let Some(end) = rust_raw(b, i)
        {
            blank(&mut code, i, end);
            i = end;
            continue;
        }
        if spec.quotes.as_bytes().contains(&c) {
            if rust && c == b'\'' {
                match rust_char(b, i) {
                    Some(end) => {
                        blank(&mut code, i, end.min(n));
                        i = end.min(n);
                    }
                    None => i += 1,
                }
                continue;
            }
            let (end, q) = string_end(b, i, spec);
            if end >= i + 2 * q
                && let Ok(content) = std::str::from_utf8(&b[i + q..end - q])
            {
                record(content, &mut prefixes, &mut path_parts);
            }
            blank(&mut code, i, end);
            i = end.max(i + 1);
            continue;
        }
        if js
            && c == b'/'
            && let Some(end) = js_regex(b, i)
        {
            blank(&mut code, i, end);
            i = end;
            continue;
        }
        i += 1;
    }
    Lexed {
        code: String::from_utf8_lossy(&code).into_owned(),
        refs: String::from_utf8_lossy(&refs).into_owned(),
        prefixes,
        path_parts,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lang::for_path;

    fn code(file: &str, text: &str) -> String {
        lex(text, for_path(file).unwrap().spec).code
    }

    #[test]
    fn blanks_comments_and_strings_but_keeps_the_shape() {
        let src = "x = \"a # b\"  # note\ny = 'z'\n";
        let out = lex(src, for_path("a.py").unwrap().spec);
        assert_eq!(out.code.len(), src.len());
        assert_eq!(out.code, "x =                \ny =    \n");
        assert_eq!(out.refs, "x = \"a # b\"        \ny = 'z'\n");
    }

    #[test]
    fn python_triple_quotes_span_lines() {
        let out = code("a.py", "def f():\n    \"\"\"doc\nmore { text\n    \"\"\"\n    return 1\n");
        assert_eq!(out.lines().nth(2).unwrap().trim(), "");
        assert!(out.contains("return 1"));
    }

    #[test]
    fn rust_lifetimes_chars_and_raw_strings() {
        let out = code("a.rs", "fn f<'a>(x: &'a str) -> char { let s = r#\"a \" {\"#; '{' }\n");
        assert!(out.contains("fn f<'a>(x: &'a str)"));
        assert_eq!(out.matches('{').count(), 1, "{out}");
    }

    #[test]
    fn js_regex_and_templates() {
        let out =
            code("a.js", "const u = s.replace(/\\/\\//g, 'x'); keep();\nconst t = \x60a $\x7bb} //c\x60; keep2();\n");
        assert!(out.contains("keep()") && out.contains("keep2()"), "{out}");
        assert_eq!(code("a.js", "const half = a / 2; // c\n").trim_end(), "const half = a / 2;");
    }

    #[test]
    fn lua_long_brackets() {
        let out = lex("local s = [[ if { ]] --[[ gone\n still gone ]] x()\n-- c\n", for_path("a.lua").unwrap().spec);
        assert!(out.code.contains("x()") && !out.code.contains("gone") && !out.code.contains("if"));
        assert!(out.refs.contains("if {") && !out.refs.contains("gone"));
    }

    #[test]
    fn hash_inside_a_word_is_not_a_comment() {
        assert!(code("a.sh", "n=$# ; go\n").contains("go"));
        assert!(code("a.php", "#[Route('/x')]\nfunction f() {}\n").contains("#["));
    }

    #[test]
    fn records_dynamic_prefixes_and_paths() {
        let out = lex(
            "getattr(self, 'handle_' + k); glob('./plugins/*.py'); open('../src/cart.ts'); ls('./jobs')\n",
            for_path("a.py").unwrap().spec,
        );
        assert_eq!(out.prefixes, ["handle_"]);
        assert_eq!(out.path_parts, ["plugins", "*plugins", "jobs"]);
    }

    #[test]
    fn counts_whole_words() {
        assert_eq!(count_word("foo foo_bar afoo foo() 1foo", "foo"), 2);
    }
}
