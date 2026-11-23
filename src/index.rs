//! Everything the finders need, built once: each file lexed, its definitions, and how often every name is mentioned.

use crate::defs::{self, Def};
use crate::lex;
use crate::walk::{self, FileEntry, Role};
use aho_corasick::AhoCorasick;
use anyhow::Result;
use rayon::prelude::*;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

pub struct Parsed {
    pub entry: FileEntry,
    /// Comments and strings blanked. Empty for text files.
    pub code: String,
    /// Comments blanked. Empty for text files: use refs().
    refs: String,
    pub defs: Vec<Def>,
    /// Byte offset of each line.
    line_at: Vec<usize>,
    /// The file looks names up at run time.
    pub dynamic: bool,
}

impl Parsed {
    /// The text a mention can be in.
    pub fn refs(&self) -> &str {
        if self.entry.role == Role::Source { &self.refs } else { &self.entry.text }
    }

    /// Lines start..=end of refs().
    pub fn lines(&self, start: usize, end: usize) -> &str {
        let from = self.line_at.get(start).copied().unwrap_or(0);
        let to = self.line_at.get(end + 1).copied().unwrap_or(self.refs().len());
        &self.refs()[from..to]
    }

    /// Lines start..=end with comments and strings blanked.
    pub fn lines_code(&self, start: usize, end: usize) -> &str {
        let from = self.line_at.get(start).copied().unwrap_or(0);
        let to = self.line_at.get(end + 1).copied().unwrap_or(self.code.len());
        &self.code[from..to]
    }

    pub fn line_count(&self) -> usize {
        self.entry.text.lines().count()
    }
}

/// Which languages are a library here: their public names are someone else's to call.
#[derive(Default, Clone)]
pub struct Lib {
    pub all: bool,
    pub langs: Vec<&'static str>,
}

impl Lib {
    pub fn is(&self, lang: &str) -> bool {
        self.all || self.langs.contains(&lang)
    }
}

/// Where a needle was found.
#[derive(Default, Clone)]
pub struct Hit {
    /// Up to two different files of code or config.
    pub files: Vec<u32>,
    /// Found in code or config that doesn't declare dependencies.
    pub outside_manifest: bool,
    /// Found in prose (a README).
    pub docs: bool,
}

pub struct Index {
    pub root: PathBuf,
    pub files: Vec<Parsed>,
    pub assets: Vec<walk::Asset>,
    /// How often each identifier appears outside comments, across the project.
    pub counts: HashMap<String, u32>,
    /// The part of counts that comes from files that declare dependencies: naming a package there isn't using it.
    pub manifest_counts: HashMap<String, u32>,
    pub prefixes: HashSet<String>,
    /// Folder names that appear in path-like strings: loaded by path, not by import.
    pub path_names: HashSet<String>,
    pub lib: Lib,
}

fn parse(entry: FileEntry) -> (Parsed, HashMap<String, u32>, Vec<String>, Vec<String>) {
    let mut counts = HashMap::new();
    let mut count = |text: &str| lex::idents(text, |t| *counts.entry(t.to_string()).or_insert(0) += 1);
    let Some(lang) = entry.lang else {
        if entry.role != Role::Doc {
            count(&entry.text);
        }
        let parsed = Parsed {
            entry,
            code: String::new(),
            refs: String::new(),
            defs: Vec::new(),
            line_at: Vec::new(),
            dynamic: false,
        };
        return (parsed, counts, Vec::new(), Vec::new());
    };
    let lexed = lex::lex(&entry.text, lang.spec);
    count(&lexed.refs);
    let raw: Vec<&str> = entry.text.split('\n').collect();
    let code: Vec<&str> = lexed.code.split('\n').collect();
    let defs = if entry.generated { Vec::new() } else { defs::find(lang, &raw, &code) };
    let mut line_at = vec![0];
    line_at.extend(entry.text.bytes().enumerate().filter(|(_, b)| *b == b'\n').map(|(i, _)| i + 1));
    let dynamic = lang.spec.dynamic.iter().any(|m| lexed.code.contains(m));
    let parsed = Parsed { entry, code: lexed.code, refs: lexed.refs, defs, line_at, dynamic };
    (parsed, counts, lexed.prefixes, lexed.path_parts)
}

fn detect_lib(root: &Path, files: &[Parsed]) -> Lib {
    let mut langs = Vec::new();
    let read = |name: &str| std::fs::read_to_string(root.join(name)).unwrap_or_default();
    // any package in the repo that others can install makes its exports someone else's to call
    let published = files.iter().filter(|f| f.entry.name() == "package.json").any(|f| {
        serde_json::from_str::<serde_json::Value>(&f.entry.text).is_ok_and(|pkg| {
            pkg["private"] != true && ["main", "exports", "module", "types"].iter().any(|k| !pkg[*k].is_null())
        })
    });
    if published {
        langs.extend(["JavaScript", "TypeScript"]);
    }
    if files.iter().any(|f| f.entry.name() == "lib.rs") || read("Cargo.toml").contains("[lib]") {
        langs.push("Rust");
    }
    let has = |lang: &str| files.iter().any(|f| f.entry.lang.is_some_and(|l| l.spec.name == lang));
    if has("Go") && !files.iter().any(|f| f.code.lines().any(|l| l.trim() == "package main")) {
        langs.push("Go");
    }
    let packaged = root.join("setup.py").is_file() || read("pyproject.toml").contains("[project]");
    let app = ["manage.py", "app.py", "main.py", "wsgi.py", "asgi.py", "Dockerfile", "Procfile"];
    if packaged && !files.iter().any(|f| app.contains(&f.entry.name())) {
        langs.push("Python");
    }
    Lib { all: false, langs }
}

impl Index {
    /// library: Some(true) treats every language as a library, Some(false) none, None guesses.
    pub fn build(root: &Path, library: Option<bool>) -> Result<Index> {
        let walked = walk::walk(root)?;
        let parsed: Vec<_> = walked.files.into_par_iter().map(parse).collect();
        let mut files = Vec::with_capacity(parsed.len());
        let mut counts: HashMap<String, u32> = HashMap::new();
        let mut manifest_counts: HashMap<String, u32> = HashMap::new();
        let mut prefixes = HashSet::new();
        let mut path_names = HashSet::new();
        for (file, file_counts, file_prefixes, parts) in parsed {
            let manifest = file.entry.role == Role::Text && file.entry.manifest();
            for (name, n) in file_counts {
                if manifest {
                    *manifest_counts.entry(name.clone()).or_insert(0) += n;
                }
                *counts.entry(name).or_insert(0) += n;
            }
            prefixes.extend(file_prefixes);
            path_names.extend(parts);
            files.push(file);
        }
        let lib = match library {
            Some(all) => Lib { all, langs: Vec::new() },
            None => detect_lib(root, &files),
        };
        Ok(Index {
            root: root.to_path_buf(),
            files,
            assets: walked.assets,
            counts,
            manifest_counts,
            prefixes,
            path_names,
            lib,
        })
    }

    /// Whether code (not a manifest) mentions this identifier.
    pub fn uses(&self, ident: &str) -> bool {
        self.counts.get(ident).copied().unwrap_or(0) > self.manifest_counts.get(ident).copied().unwrap_or(0)
    }

    /// Looks for each needle as a substring (ignoring case) in every file a mention can be in.
    pub fn search(&self, needles: &[String]) -> Result<Vec<Hit>> {
        let mut hits = vec![Hit::default(); needles.len()];
        if needles.is_empty() {
            return Ok(hits);
        }
        let ac = AhoCorasick::builder().ascii_case_insensitive(true).build(needles)?;
        let found: Vec<Vec<u32>> = self
            .files
            .par_iter()
            .map(|f| {
                let mut seen = vec![false; needles.len()];
                for m in ac.find_overlapping_iter(f.refs()) {
                    seen[m.pattern().as_usize()] = true;
                }
                seen.iter().enumerate().filter(|(_, s)| **s).map(|(i, _)| i as u32).collect()
            })
            .collect();
        for (fi, needles_found) in found.iter().enumerate() {
            let manifest = self.files[fi].entry.manifest();
            let doc = self.files[fi].entry.role == Role::Doc;
            for &n in needles_found {
                let hit = &mut hits[n as usize];
                if doc {
                    hit.docs = true;
                    continue;
                }
                if hit.files.len() < 2 {
                    hit.files.push(fi as u32);
                }
                hit.outside_manifest |= !manifest;
            }
        }
        Ok(hits)
    }
}
