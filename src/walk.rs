//! Reads the project: which files are code, which are config that can mention code, which to leave alone.

use crate::lang::{self, Lang};
use anyhow::Result;
use ignore::WalkBuilder;
use rayon::prelude::*;
use std::path::{Path, PathBuf};

/// Bigger files are data or bundles.
const MAX_BYTES: u64 = 2 * 1024 * 1024;

const SKIP_DIRS: &[&str] = &[
    ".git",
    ".shrink",
    ".hg",
    ".svn",
    "node_modules",
    "target",
    "dist",
    "build",
    "out",
    "vendor",
    ".venv",
    "venv",
    "env",
    "__pycache__",
    ".next",
    ".nuxt",
    ".svelte-kit",
    "coverage",
    ".tox",
    ".mypy_cache",
    ".pytest_cache",
    ".ruff_cache",
    ".gradle",
    "obj",
    "Pods",
    ".dart_tool",
    ".idea",
    ".vscode",
    "third_party",
    ".fast",
    ".cache",
    "bower_components",
    ".terraform",
    ".turbo",
    ".parcel-cache",
    "DerivedData",
];

const LOCKFILES: &[&str] = &[
    "package-lock.json",
    "yarn.lock",
    "pnpm-lock.yaml",
    "bun.lock",
    "bun.lockb",
    "Cargo.lock",
    "poetry.lock",
    "uv.lock",
    "go.sum",
    "composer.lock",
    "Gemfile.lock",
    "Pipfile.lock",
    "pubspec.lock",
    "flake.lock",
    "npm-shrinkwrap.json",
    "mix.lock",
];

/// Files that aren't code we understand, but can name a function, a file or a package.
const TEXT_EXTS: &[&str] = &[
    "json",
    "jsonc",
    "json5",
    "yaml",
    "yml",
    "toml",
    "ini",
    "cfg",
    "conf",
    "xml",
    "html",
    "htm",
    "vue",
    "svelte",
    "astro",
    "css",
    "scss",
    "sass",
    "less",
    "ps1",
    "bat",
    "cmd",
    "mk",
    "cmake",
    "gradle",
    "properties",
    "tpl",
    "tmpl",
    "j2",
    "jinja",
    "jinja2",
    "erb",
    "ejs",
    "hbs",
    "mustache",
    "twig",
    "pug",
    "haml",
    "slim",
    "graphql",
    "gql",
    "proto",
    "sql",
    "tf",
    "nix",
    "plist",
    "csproj",
    "fsproj",
    "vbproj",
    "sln",
    "vcxproj",
    "props",
    "targets",
    "unity",
    "prefab",
    "asset",
    "ipynb",
    "mdx",
    "razor",
    "cshtml",
    "xaml",
    "storyboard",
    "xib",
    "pro",
    "cabal",
    "podspec",
    "bazel",
    "bzl",
    "service",
    "desktop",
    "in",
    "r",
    "jl",
    "scala",
    "clj",
    "hs",
    "ml",
    "zig",
    "nim",
    "pl",
    "pm",
    "groovy",
    "vb",
    "fs",
    "m",
    "mm",
    "sol",
    "erl",
    "cr",
    "d",
    "v",
    "gd",
    "gdshader",
    "tscn",
    "tres",
    "rbxlx",
    "rbxmx",
    "project",
];

const TEXT_NAMES: &[&str] = &[
    "Dockerfile",
    "Makefile",
    "makefile",
    "GNUmakefile",
    "CMakeLists.txt",
    "Procfile",
    "Justfile",
    "justfile",
    "Jenkinsfile",
    "Vagrantfile",
    "Pipfile",
    "MANIFEST.in",
    "BUILD",
    "WORKSPACE",
    "Brewfile",
    "Caddyfile",
    "Tiltfile",
    "go.mod",
    ".babelrc",
    ".eslintrc",
    ".prettierrc",
    ".npmrc",
    ".env.example",
];

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Role {
    /// A language from the table: we find definitions in it.
    Source,
    /// Anything else that can mention a name.
    Text,
    /// Prose. It can name an image, but mentioning a function there doesn't make the function used.
    Doc,
}

/// A file that's shipped as it is: an image, a font, a sound. Never read, only weighed.
pub struct Asset {
    pub rel: String,
    pub bytes: u64,
}

pub struct Walked {
    pub files: Vec<FileEntry>,
    pub assets: Vec<Asset>,
}

const ASSET_EXTS: &[&str] = &[
    "png", "jpg", "jpeg", "gif", "webp", "avif", "bmp", "ico", "svg", "tif", "tiff", "psd", "heic", "mp3", "wav",
    "ogg", "flac", "m4a", "aac", "opus", "mp4", "webm", "mov", "avi", "mkv", "ttf", "otf", "woff", "woff2", "eot",
    "pdf", "zip", "gz", "7z", "rar", "glb", "gltf", "fbx", "obj", "blend", "ktx", "ktx2", "dds", "tga", "riv",
    "lottie", "atlas",
];

const DOC_EXTS: &[&str] = &["md", "markdown", "rst", "txt", "adoc"];

pub fn is_asset(rel: &str) -> bool {
    rel.rsplit_once('.').is_some_and(|(_, ext)| ASSET_EXTS.contains(&ext.to_ascii_lowercase().as_str()))
}

pub struct FileEntry {
    /// Path from the project root, with forward slashes.
    pub rel: String,
    pub lang: Option<&'static Lang>,
    pub role: Role,
    pub text: String,
    pub test: bool,
    pub generated: bool,
}

impl FileEntry {
    pub fn name(&self) -> &str {
        self.rel.rsplit('/').next().unwrap_or(&self.rel)
    }

    pub fn dir(&self) -> &str {
        self.rel.rsplit_once('/').map_or("", |(dir, _)| dir)
    }

    /// Declares dependencies: a mention in here isn't a use.
    pub fn manifest(&self) -> bool {
        let name = self.name();
        matches!(
            name,
            "package.json" | "Cargo.toml" | "pyproject.toml" | "setup.py" | "setup.cfg" | "Pipfile" | "go.mod"
        ) || is_requirements(name)
    }
}

pub fn is_requirements(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.ends_with(".txt") && (lower.starts_with("requirements") || lower.starts_with("constraints"))
        || lower.ends_with(".txt") && lower.contains("requirements")
}

pub fn is_test(rel: &str) -> bool {
    const DIRS: &[&str] = &[
        "test",
        "tests",
        "__tests__",
        "spec",
        "specs",
        "testing",
        "e2e",
        "fixtures",
        "testdata",
        "mocks",
        "__mocks__",
        "androidtest",
        "unittest",
        "unittests",
        "integration_tests",
        "cypress",
        "playwright",
    ];
    let (dir, name) = rel.rsplit_once('/').unwrap_or(("", rel));
    if dir.split('/').any(|part| DIRS.contains(&part.to_ascii_lowercase().as_str())) {
        return true;
    }
    let stem = name.rsplit_once('.').map_or(name, |(stem, _)| stem);
    let lower = stem.to_ascii_lowercase();
    lower.starts_with("test_")
        || lower == "conftest"
        || matches!(stem, "tests" | "test")
        || ["_test", "_tests", "_spec", ".test", ".spec", "-test", "-spec"].iter().any(|s| lower.ends_with(s))
        || ["Test", "Tests", "Spec", "IT"].iter().any(|s| stem.len() > s.len() && stem.ends_with(s))
}

fn is_generated(rel: &str, text: &str) -> bool {
    const SUFFIXES: &[&str] = &[
        ".d.ts",
        ".min.js",
        ".min.css",
        ".pb.go",
        "_pb2.py",
        "_pb2_grpc.py",
        ".g.dart",
        ".freezed.dart",
        ".designer.cs",
        ".g.cs",
    ];
    if SUFFIXES.iter().any(|s| rel.ends_with(s)) || rel.contains(".generated.") || rel.contains("/generated/") {
        return true;
    }
    let head: String = text.chars().take(600).collect::<String>().to_ascii_lowercase();
    ["do not edit", "@generated", "auto-generated", "autogenerated", "code generated", "automatically generated"]
        .iter()
        .any(|m| head.contains(m))
        || text.lines().take(20).any(|l| l.len() > 3000)
}

fn role_of(rel: &str) -> Option<(Role, Option<&'static Lang>)> {
    let name = rel.rsplit('/').next().unwrap_or(rel);
    if LOCKFILES.contains(&name) {
        return None;
    }
    if let Some(lang) = lang::for_path(rel) {
        return Some((Role::Source, Some(lang)));
    }
    let ext = name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default();
    let text = TEXT_EXTS.contains(&ext.as_str())
        || TEXT_NAMES.iter().any(|n| name == *n || name.starts_with(&format!("{n}.")))
        || is_requirements(name)
        || (name.starts_with('.') && (name.ends_with("rc") || name.ends_with("ignore")));
    if text {
        return Some((Role::Text, None));
    }
    DOC_EXTS.contains(&ext.as_str()).then_some((Role::Doc, None))
}

pub fn walk(root: &Path) -> Result<Walked> {
    let mut assets: Vec<Asset> = Vec::new();
    let mut paths: Vec<(PathBuf, String, Role, Option<&'static Lang>)> = Vec::new();
    // Inside a git repo, ignore files apply up to the repo root. Outside one, only the project's own:
    // a stray ~/.gitignore must not hide the whole project.
    let in_repo = root.ancestors().any(|dir| dir.join(".git").exists());
    let walker = WalkBuilder::new(root)
        .hidden(false)
        .require_git(in_repo)
        .parents(in_repo)
        .filter_entry(|e| {
            !(e.file_type().is_some_and(|t| t.is_dir())
                && SKIP_DIRS.contains(&e.file_name().to_string_lossy().as_ref()))
        })
        .build();
    for entry in walker.flatten() {
        if !entry.file_type().is_some_and(|t| t.is_file()) {
            continue;
        }
        let Ok(rel) = entry.path().strip_prefix(root) else { continue };
        let rel = crate::util::forward_slashes(rel);
        let bytes = entry.metadata().map_or(u64::MAX, |m| m.len());
        if is_asset(&rel) {
            assets.push(Asset { rel, bytes });
            continue;
        }
        let Some((role, lang)) = role_of(&rel) else { continue };
        if bytes > MAX_BYTES {
            continue;
        }
        paths.push((entry.into_path(), rel, role, lang));
    }
    let mut files: Vec<FileEntry> = paths
        .into_par_iter()
        .filter_map(|(abs, rel, role, lang)| {
            let text = String::from_utf8(std::fs::read(&abs).ok()?).ok()?;
            let test = is_test(&rel);
            let generated = is_generated(&rel, &text);
            Some(FileEntry { rel, lang, role, text, test, generated })
        })
        .collect();
    files.sort_by(|a, b| a.rel.cmp(&b.rel));
    assets.sort_by(|a, b| a.rel.cmp(&b.rel));
    Ok(Walked { files, assets })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn knows_tests() {
        for rel in [
            "tests/a.py",
            "src/test_x.py",
            "a/b_test.go",
            "x.spec.ts",
            "src/__tests__/a.js",
            "FooTest.java",
            "conftest.py",
        ] {
            assert!(is_test(rel), "{rel}");
        }
        for rel in ["src/a.py", "latest.py", "contest.js", "Test.java", "src/attest/x.rs"] {
            assert!(!is_test(rel), "{rel}");
        }
    }

    #[test]
    fn knows_roles() {
        assert_eq!(role_of("src/a.py").unwrap().0, Role::Source);
        assert_eq!(role_of("package.json").unwrap().0, Role::Text);
        assert_eq!(role_of("Dockerfile.prod").unwrap().0, Role::Text);
        assert_eq!(role_of("requirements-dev.txt").unwrap().0, Role::Text);
        assert!(role_of("package-lock.json").is_none());
        assert_eq!(role_of("README.md").unwrap().0, Role::Doc);
        assert!(role_of("data.bin").is_none());
        assert!(is_asset("img/Logo.PNG") && !is_asset("src/a.py"));
    }

    #[test]
    fn knows_generated() {
        assert!(is_generated("a.d.ts", ""));
        assert!(is_generated("a.go", "// Code generated by protoc. DO NOT EDIT.\npackage a\n"));
        assert!(!is_generated("a.go", "package a\n"));
    }
}
