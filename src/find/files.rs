//! Dead files: a source file whose name nothing else mentions, in a language where files are imported by name.

use crate::candidate::{CandKind, Candidate, Edit, Tier};
use crate::index::Index;
use anyhow::Result;
use std::collections::HashSet;

/// Names a runtime, a framework or a person runs directly.
const ENTRY_STEMS: &[&str] = &[
    "main",
    "page",
    "layout",
    "route",
    "loading",
    "error",
    "not-found",
    "template",
    "default",
    "robots",
    "sitemap",
    "manifest",
    "middleware",
    "proxy",
    "index",
    "app",
    "server",
    "cli",
    "setup",
    "conftest",
    "manage",
    "wsgi",
    "asgi",
    "__init__",
    "__main__",
    "mod",
    "lib",
    "init",
    "build",
    "gulpfile",
    "gruntfile",
    "noxfile",
    "fabfile",
    "tasks",
    "settings",
    "urls",
    "admin",
    "models",
    "views",
    "apps",
    "forms",
    "signals",
    "routes",
    "middleware",
    "config",
    "env",
    "seed",
    "seeds",
    "worker",
    "service-worker",
    "sw",
    "preload",
    "background",
    "content",
    "popup",
    "options",
    "plugin",
    "extension",
    "bootstrap",
    "autoload",
    "application",
    "schema",
    "types",
    "globals",
    "shims",
    "instrumentation",
    "sitecustomize",
];

/// Folders a framework or a tool loads by convention: what's in them is used without being named.
pub const CONVENTION_DIRS: &[&str] = &[
    "pages",
    "routes",
    "migrations",
    "versions",
    "alembic",
    "management",
    "commands",
    "tasks",
    "plugins",
    "middleware",
    "layouts",
    "stories",
    "theme",
    "themes",
    "cron",
    "jobs",
    "workers",
    "lambda",
    "lambdas",
    "functions",
    "seeds",
    "initializers",
    "controllers",
    "helpers",
    "mailers",
    "channels",
    "composables",
    "templatetags",
    "spiders",
    "ServerScriptService",
    "StarterPlayerScripts",
    "StarterGui",
    "ReplicatedFirst",
];

/// More folders whose files are run or shipped as they are, not imported.
const ENTRY_DIRS: &[&str] = &[
    "bin",
    "scripts",
    "script",
    "tools",
    "public",
    "static",
    "examples",
    "example",
    "docs",
    "benchmarks",
    "bench",
    "benches",
    "pkg",
    "packaging",
    "deploy",
    "deployment",
    "ci",
    "infra",
    "ops",
    "contrib",
    "extras",
    "misc",
    "hack",
    "installer",
    "homebrew",
    "brew",
    "debian",
    "docker",
];

/// A file with one of these loads other files by path.
pub const LOADERS: &[&str] = &[
    "pkgutil",
    "import_module",
    "importlib",
    "readdirSync",
    "readdir(",
    "glob",
    "listdir",
    "require.context",
    "import.meta.glob",
    "Dir[",
    "Dir.glob",
    "scandir",
    "walkdir",
    "GetDescendants",
    "GetChildren",
];

fn stem_of(name: &str) -> &str {
    name.rsplit_once('.').map_or(name, |(stem, _)| stem)
}

fn parent(dir: &str) -> &str {
    dir.rsplit_once('/').map_or("", |(up, _)| up)
}

pub fn find(ix: &Index) -> Result<Vec<Candidate>> {
    let loader_dirs: HashSet<&str> = ix
        .files
        .iter()
        .filter(|f| f.entry.lang.is_some() && LOADERS.iter().any(|m| f.refs().contains(m)))
        .map(|f| f.entry.dir())
        .collect();
    let mut wanted: Vec<(usize, String)> = Vec::new();
    for (fi, file) in ix.files.iter().enumerate() {
        let e = &file.entry;
        let Some(lang) = e.lang else { continue };
        if !lang.spec.file_refs || e.test || e.generated {
            continue;
        }
        let stem = stem_of(e.name());
        let lower = stem.to_ascii_lowercase();
        let entry_name = ENTRY_STEMS.contains(&lower.as_str())
            || stem.starts_with(['.', '+', '_', '$', '['])
            || [".config", ".setup", ".server", ".client", ".stories", ".worker", ".d"]
                .iter()
                .any(|s| lower.ends_with(s))
            || ["config", "rc"].iter().any(|s| lower.ends_with(s));
        let entry_dir = e.dir().split('/').any(|part| ENTRY_DIRS.contains(&part) || CONVENTION_DIRS.contains(&part));
        let script = e.text.starts_with("#!") || e.text.contains("__name__");
        if stem.len() < 3 || entry_name || entry_dir || script {
            continue;
        }
        wanted.push((fi, stem.to_string()));
    }
    let needles: Vec<String> = wanted.iter().map(|(_, stem)| stem.clone()).collect();
    let hits = ix.search(&needles)?;
    let mut out = Vec::new();
    for ((fi, stem), hit) in wanted.iter().zip(&hits) {
        if hit.files.iter().any(|f| *f as usize != *fi) {
            continue; // another file names it
        }
        let file = &ix.files[*fi];
        let e = &file.entry;
        let dir = e.dir();
        let dir_name = dir.rsplit('/').next().unwrap_or_default();
        let risk = if dir.is_empty() {
            Some("at the top of the repo, where files are often run by hand")
        } else if ix.path_names.contains(dir_name) {
            Some("in a folder that code reaches by path")
        } else if loader_dirs.contains(dir) || loader_dirs.contains(parent(dir)) {
            Some("next to code that loads files by path")
        } else {
            None
        };
        let why = match risk {
            Some(risk) => format!("no other file mentions {stem}, but it is {risk}"),
            None => format!("no other file mentions {stem}"),
        };
        out.push(Candidate {
            id: format!("file:{}", e.rel),
            kind: CandKind::File,
            tier: if risk.is_some() { Tier::Risky } else { Tier::Safe },
            name: e.name().to_string(),
            path: e.rel.clone(),
            line: 0,
            lines: file.line_count(),
            bytes: 0,
            why,
            edits: vec![Edit::Delete { path: e.rel.clone() }],
        });
    }
    Ok(out)
}
