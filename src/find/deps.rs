//! Unused dependencies: declared in a manifest, mentioned nowhere else.
//!
//! npm (and pnpm, yarn, bun), Cargo, and Python (requirements files, pyproject). Go has its own
//! tool for this, go mod tidy, which the cut runs for you.

use crate::candidate::{CandKind, Candidate, Eco, Edit, Tier};
use crate::index::Index;
use crate::walk;
use anyhow::{Context, Result, bail};
use serde_json::Value;
use std::path::{Path, PathBuf};
use toml_edit::DocumentMut;

/// optionalDependencies are left alone: they hold platform binaries nothing imports by name.
const NPM_SECTIONS: &[&str] = &["dependencies", "devDependencies"];
const CARGO_SECTIONS: &[&str] = &["dependencies", "dev-dependencies", "build-dependencies"];

/// PyPI names whose import name is something else.
const PY_IMPORTS: &[(&str, &str)] = &[
    ("pillow", "PIL"),
    ("pyyaml", "yaml"),
    ("beautifulsoup4", "bs4"),
    ("scikit-learn", "sklearn"),
    ("opencv-python", "cv2"),
    ("opencv-python-headless", "cv2"),
    ("python-dateutil", "dateutil"),
    ("python-dotenv", "dotenv"),
    ("pyjwt", "jwt"),
    ("attrs", "attr"),
    ("protobuf", "google"),
    ("psycopg2-binary", "psycopg2"),
    ("mysqlclient", "MySQLdb"),
    ("pyserial", "serial"),
    ("pyzmq", "zmq"),
    ("python-multipart", "multipart"),
    ("msgpack-python", "msgpack"),
    ("scikit-image", "skimage"),
    ("pycryptodome", "Crypto"),
    ("python-jose", "jose"),
    ("websocket-client", "websocket"),
    ("pymupdf", "fitz"),
    ("discord.py", "discord"),
    ("python-slugify", "slugify"),
    ("pytz", "pytz"),
    ("setuptools", "pkg_resources"),
];

struct Declared {
    manifest: String,
    eco: Eco,
    section: String,
    name: String,
    /// Any of these as a substring of another file means it's used.
    needles: Vec<String>,
    /// Any of these as an identifier anywhere means it's used.
    idents: Vec<String>,
    /// Used in a way only the manifest shows (a script, a feature), or a tool we don't judge.
    keep: bool,
}

/// Packages that are configured, not imported.
fn npm_tooling(name: &str) -> bool {
    const PREFIXES: &[&str] = &[
        "@types/",
        "eslint",
        "prettier",
        "@typescript-eslint/",
        "typescript",
        "@babel/",
        "babel-",
        "postcss",
        "autoprefixer",
        "tailwindcss",
        "@tailwindcss/",
        "webpack",
        "husky",
        "lint-staged",
        "@vitejs/",
        "vite-plugin",
        "rollup-plugin",
        "@rollup/",
        "ts-node",
        "tsx",
        "jest",
        "ts-jest",
        "@jest/",
        "vitest",
        "@vitest/",
        "nodemon",
        "stylelint",
        "@commitlint/",
        "@swc/",
        "esbuild",
        "sass",
        "less",
        "@biomejs/",
        "@nuxt/",
        "@sveltejs/",
        "@next/",
        "@angular/",
        "@angular-devkit/",
        "@storybook/",
        "storybook",
        "karma",
        "mocha",
        "chai",
        "nyc",
        "c8",
        "@testing-library/",
        "jsdom",
        "happy-dom",
        "cross-env",
        "dotenv",
        "tslib",
        "core-js",
        "regenerator-runtime",
        "@playwright/",
        "playwright",
        "cypress",
        "@nx/",
        "nx",
        "lerna",
        "turbo",
        "@changesets/",
        "@swc-node/",
        "@tsconfig/",
        "@capacitor/",
        "@electron/",
        "electron",
        "react-native",
        "expo",
    ];
    PREFIXES.iter().any(|p| name.starts_with(p))
        || ["@docusaurus/", "@mdx-js/", "@astrojs/", "@remix-run/", "gatsby"].iter().any(|p| name.starts_with(p))
        || ["preset", "plugin", "theme"].iter().any(|s| name.contains(s))
        || ["-loader", "-plugin", "-preset", "-config", "-transformer", "-adapter", "-polyfill"]
            .iter()
            .any(|s| name.contains(s))
}

fn contains_word(hay: &str, word: &str) -> bool {
    hay.match_indices(word).any(|(i, _)| {
        let ok =
            |c: Option<char>| c.is_none_or(|c| !(c.is_alphanumeric() || c == '_' || c == '-' || c == '/' || c == '@'));
        ok(hay[..i].chars().next_back()) && ok(hay[i + word.len()..].chars().next())
    })
}

/// The commands a package installs: tsc for typescript.
fn npm_bins(dir: &Path, name: &str) -> Vec<String> {
    let Ok(text) = std::fs::read_to_string(dir.join("node_modules").join(name).join("package.json")) else {
        return Vec::new();
    };
    match serde_json::from_str::<Value>(&text).map(|v| v["bin"].clone()) {
        Ok(Value::String(_)) => vec![name.rsplit('/').next().unwrap_or(name).to_string()],
        Ok(Value::Object(map)) => map.keys().cloned().collect(),
        _ => Vec::new(),
    }
}

fn npm(root: &Path, manifest: &str, text: &str, out: &mut Vec<Declared>) {
    let Ok(pkg) = serde_json::from_str::<Value>(text) else { return };
    let mut rest = pkg.clone();
    if let Some(obj) = rest.as_object_mut() {
        for section in NPM_SECTIONS {
            obj.remove(*section);
        }
    }
    let rest = rest.to_string();
    let dir = root.join(manifest).parent().map(Path::to_path_buf).unwrap_or_default();
    for section in NPM_SECTIONS {
        let Some(deps) = pkg[*section].as_object() else { continue };
        for name in deps.keys() {
            let bins = npm_bins(&dir, name);
            let keep = npm_tooling(name) || contains_word(&rest, name) || bins.iter().any(|b| contains_word(&rest, b));
            let mut needles = vec![name.clone()];
            needles.extend(bins.into_iter().filter(|b| b.len() >= 3));
            out.push(Declared {
                manifest: manifest.into(),
                eco: Eco::Npm,
                section: (*section).into(),
                name: name.clone(),
                needles,
                idents: Vec::new(),
                keep,
            });
        }
    }
}

fn cargo(manifest: &str, text: &str, out: &mut Vec<Declared>) {
    let Ok(doc) = text.parse::<DocumentMut>() else { return };
    for section in CARGO_SECTIONS {
        let Some(table) = doc.get(section).and_then(|i| i.as_table_like()) else { continue };
        for (key, item) in table.iter() {
            let optional =
                item.as_table_like().and_then(|t| t.get("optional")).and_then(|v| v.as_bool()).unwrap_or(false);
            // named again in the manifest: a feature turns it on, or another table needs it
            let keep = optional || text.matches(key).count() > 1;
            out.push(Declared {
                manifest: manifest.into(),
                eco: Eco::Cargo,
                section: (*section).into(),
                name: key.to_string(),
                needles: Vec::new(),
                idents: vec![key.replace('-', "_")],
                keep,
            });
        }
    }
}

/// "requests" from "requests[socks]>=2.0 ; python_version > '3'".
fn requirement_name(spec: &str) -> Option<&str> {
    let spec = spec.trim();
    if spec.is_empty() || spec.starts_with(['-', '#']) || spec.contains("://") || spec.contains(" @ ") {
        return None;
    }
    let end = spec.find(|c: char| !(c.is_alphanumeric() || matches!(c, '-' | '_' | '.'))).unwrap_or(spec.len());
    (end > 0).then(|| &spec[..end])
}

fn python_idents(name: &str) -> Vec<String> {
    let lower = name.to_ascii_lowercase();
    let mut out: Vec<String> =
        PY_IMPORTS.iter().filter(|(pkg, _)| *pkg == lower).map(|(_, import)| import.to_string()).collect();
    let snake = lower.replace(['-', '.'], "_");
    for candidate in [
        snake.clone(),
        snake.strip_prefix("python_").unwrap_or(&snake).to_string(),
        snake.strip_suffix("_python").unwrap_or(&snake).to_string(),
        snake.strip_prefix("py").unwrap_or(&snake).to_string(),
        snake.split('_').next().unwrap_or(&snake).to_string(),
    ] {
        if candidate.len() >= 2 && !out.contains(&candidate) {
            out.push(candidate);
        }
    }
    out
}

fn python(manifest: &str, eco: Eco, section: &str, name: &str, out: &mut Vec<Declared>) {
    out.push(Declared {
        manifest: manifest.into(),
        eco,
        section: section.into(),
        name: name.to_string(),
        needles: if name.len() >= 3 { vec![name.to_string()] } else { Vec::new() },
        idents: python_idents(name),
        keep: false,
    });
}

fn pyproject(manifest: &str, text: &str, out: &mut Vec<Declared>) {
    let Ok(doc) = text.parse::<DocumentMut>() else { return };
    if let Some(list) = doc.get("project").and_then(|p| p.get("dependencies")).and_then(|d| d.as_array()) {
        for name in list.iter().filter_map(|v| v.as_str()).filter_map(requirement_name) {
            python(manifest, Eco::Pyproject, "project", name, out);
        }
    }
    let poetry = doc.get("tool").and_then(|t| t.get("poetry")).and_then(|p| p.get("dependencies"));
    if let Some(table) = poetry.and_then(|d| d.as_table_like()) {
        for (key, _) in table.iter().filter(|(key, _)| *key != "python") {
            python(manifest, Eco::Pyproject, "poetry", key, out);
        }
    }
}

fn declared(ix: &Index) -> Vec<Declared> {
    let mut out = Vec::new();
    for file in &ix.files {
        let e = &file.entry;
        match e.name() {
            "package.json" => npm(&ix.root, &e.rel, &e.text, &mut out),
            "Cargo.toml" => cargo(&e.rel, &e.text, &mut out),
            "pyproject.toml" => pyproject(&e.rel, &e.text, &mut out),
            name if walk::is_requirements(name) => {
                for name in e.text.lines().filter_map(requirement_name) {
                    python(&e.rel, Eco::Requirements, "", name, &mut out);
                }
            }
            _ => {}
        }
    }
    out
}

/// (how many dependencies are declared, which ones nothing uses)
pub fn find(ix: &Index) -> Result<(u64, Vec<Candidate>)> {
    let all = declared(ix);
    let go_requires = ix
        .files
        .iter()
        .filter(|f| f.entry.name() == "go.mod")
        .map(|f| {
            f.entry
                .text
                .lines()
                .filter(|l| l.contains(" v") && !l.contains("// indirect") && !l.starts_with("go "))
                .count()
        })
        .sum::<usize>();
    let total = (all.len() + go_requires) as u64;
    let mut needles: Vec<String> = Vec::new();
    let mut owner: Vec<usize> = Vec::new();
    for (di, dep) in all.iter().enumerate().filter(|(_, d)| !d.keep) {
        for needle in &dep.needles {
            needles.push(needle.clone());
            owner.push(di);
        }
    }
    let hits = ix.search(&needles)?;
    let mut used = vec![false; all.len()];
    for (hit, di) in hits.iter().zip(&owner) {
        used[*di] |= hit.outside_manifest;
    }
    let mut out = Vec::new();
    for (di, dep) in all.iter().enumerate() {
        if dep.keep || used[di] || dep.idents.iter().any(|i| ix.uses(i)) {
            continue;
        }
        let text =
            ix.files.iter().find(|f| f.entry.rel == dep.manifest).map(|f| f.entry.text.as_str()).unwrap_or_default();
        let line = text.lines().position(|l| l.contains(&dep.name)).map_or(0, |i| i + 1);
        let risk = if matches!(dep.eco, Eco::Requirements | Eco::Pyproject) {
            Some("Python servers and plugins are often used without an import")
        } else if dep.section == "devDependencies" {
            Some("dev tools are often run without being named anywhere")
        } else {
            None
        };
        out.push(Candidate {
            id: format!("dep:{}:{}", dep.manifest, dep.name),
            kind: CandKind::Dep,
            tier: if risk.is_some() { Tier::Risky } else { Tier::Safe },
            name: dep.name.clone(),
            path: dep.manifest.clone(),
            line,
            lines: 1,
            bytes: 0,
            why: match risk {
                Some(risk) => format!("nothing imports it, but {risk}"),
                None => "nothing imports or runs it".into(),
            },
            edits: vec![Edit::Dep {
                path: dep.manifest.clone(),
                eco: dep.eco,
                section: dep.section.clone(),
                name: dep.name.clone(),
            }],
        });
    }
    Ok((total, out))
}

fn remove_npm(text: &str, section: &str, name: &str) -> Result<String> {
    // Take the one line out, so the rest of the file keeps its formatting.
    let lines: Vec<&str> = text.split_inclusive('\n').collect();
    let open = format!("\"{section}\"");
    let entry = format!("\"{name}\"");
    let mut inside = false;
    for (i, line) in lines.iter().enumerate() {
        let t = line.trim();
        if !inside {
            inside = t.starts_with(&open) && t.ends_with('{');
            continue;
        }
        if t.starts_with('}') {
            break;
        }
        if t.starts_with(&entry) && t[entry.len()..].trim_start().starts_with(':') {
            let mut out: Vec<String> = lines.iter().map(|l| l.to_string()).collect();
            out.remove(i);
            // the entry before it is now the last one: no trailing comma
            if !t.ends_with(',') && i > 0 && out.get(i).is_some_and(|l| l.trim().starts_with('}')) {
                let prev = &mut out[i - 1];
                if let Some(comma) = prev.rfind(',')
                    && prev[comma + 1..].trim().is_empty()
                {
                    prev.remove(comma);
                }
            }
            // nothing left in the section: close it on one line
            if i > 0 && out[i - 1].trim_end().ends_with('{') && out.get(i).is_some_and(|l| l.trim().starts_with('}')) {
                let close = out.remove(i);
                let open = out[i - 1].trim_end().to_string();
                out[i - 1] = format!("{open}{}", close.trim_start());
            }
            return Ok(out.concat());
        }
    }
    // An unusual layout: rewrite the file.
    let mut pkg: Value = serde_json::from_str(text).context("package.json isn't valid JSON")?;
    let removed = pkg[section].as_object_mut().and_then(|deps| deps.shift_remove(name));
    if removed.is_none() {
        bail!("{name} isn't in {section}");
    }
    Ok(serde_json::to_string_pretty(&pkg)? + "\n")
}

fn remove_toml(text: &str, eco: Eco, section: &str, name: &str) -> Result<String> {
    let mut doc = text.parse::<DocumentMut>().context("the manifest isn't valid TOML")?;
    let removed = match (eco, section) {
        (Eco::Pyproject, "project") => {
            let list = doc.get_mut("project").and_then(|p| p.get_mut("dependencies")).and_then(|d| d.as_array_mut());
            list.is_some_and(|list| {
                let before = list.len();
                list.retain(|v| v.as_str().and_then(requirement_name) != Some(name));
                list.len() < before
            })
        }
        (Eco::Pyproject, _) => {
            let table = doc.get_mut("tool").and_then(|t| t.get_mut("poetry")).and_then(|p| p.get_mut("dependencies"));
            table.and_then(|t| t.as_table_like_mut()).is_some_and(|t| t.remove(name).is_some())
        }
        _ => doc.get_mut(section).and_then(|t| t.as_table_like_mut()).is_some_and(|t| t.remove(name).is_some()),
    };
    if !removed {
        bail!("{name} isn't declared where the scan saw it");
    }
    Ok(doc.to_string())
}

/// The manifest without one dependency.
pub fn remove(text: &str, eco: Eco, section: &str, name: &str) -> Result<String> {
    match eco {
        Eco::Npm => remove_npm(text, section, name),
        Eco::Cargo | Eco::Pyproject => remove_toml(text, eco, section, name),
        Eco::Requirements => {
            Ok(text.split_inclusive('\n').filter(|line| requirement_name(line) != Some(name)).collect())
        }
    }
}

/// The install command that brings node_modules in line with a package.json, if there is a node_modules to keep in line.
pub fn sync_command(root: &Path, manifest: &str) -> Option<(Vec<String>, PathBuf)> {
    if !manifest.ends_with("package.json") {
        return None;
    }
    let dir = root.join(manifest).parent()?.to_path_buf();
    if !dir.join("node_modules").is_dir() {
        return None;
    }
    let argv: &[&str] = if dir.join("pnpm-lock.yaml").is_file() {
        &["pnpm", "install"]
    } else if dir.join("yarn.lock").is_file() {
        &["yarn", "install"]
    } else if dir.join("bun.lock").is_file() || dir.join("bun.lockb").is_file() {
        &["bun", "install"]
    } else {
        &["npm", "install", "--no-audit", "--no-fund"]
    };
    Some((argv.iter().map(|s| s.to_string()).collect(), dir))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn npm_removal_keeps_the_formatting() {
        let pkg = "{\n  \"name\": \"x\",\n  \"dependencies\": {\n    \"a\": \"1\",\n    \"left-pad\": \"^1.3.0\"\n  },\n  \"devDependencies\": {\n    \"left-pad\": \"1\"\n  }\n}\n";
        let out = remove(pkg, Eco::Npm, "dependencies", "left-pad").unwrap();
        assert_eq!(
            out,
            "{\n  \"name\": \"x\",\n  \"dependencies\": {\n    \"a\": \"1\"\n  },\n  \"devDependencies\": {\n    \"left-pad\": \"1\"\n  }\n}\n"
        );
        let out = remove(pkg, Eco::Npm, "dependencies", "a").unwrap();
        assert!(out.contains("\"dependencies\": {\n    \"left-pad\": \"^1.3.0\"\n  }"));
        let empty = remove(&out, Eco::Npm, "dependencies", "left-pad").unwrap();
        assert!(empty.contains("  \"dependencies\": {},\n  \"devDependencies\": {"), "{empty}");
        let one_line = "{\"dependencies\":{\"a\":\"1\",\"b\":\"2\"}}";
        let out: Value = serde_json::from_str(&remove(one_line, Eco::Npm, "dependencies", "a").unwrap()).unwrap();
        assert_eq!(out["dependencies"].as_object().unwrap().len(), 1);
        assert!(remove(pkg, Eco::Npm, "dependencies", "nope").is_err());
    }

    #[test]
    fn toml_and_requirements_removal() {
        let cargo = "[package]\nname = \"x\"\n\n[dependencies]\n# why\nserde = { version = \"1\", features = [\"derive\"] }\nrand = \"0.9\"\n";
        assert_eq!(remove(cargo, Eco::Cargo, "dependencies", "rand").unwrap(), cargo.replace("rand = \"0.9\"\n", ""));
        let py = "[project]\ndependencies = [\n    \"requests>=2\",\n    \"six\",\n]\n";
        let out = remove(py, Eco::Pyproject, "project", "six").unwrap();
        assert!(out.contains("requests>=2") && !out.contains("six"));
        let req = "requests>=2\n# pinned\nsix==1.16  # old\n-r other.txt\n";
        assert_eq!(remove(req, Eco::Requirements, "", "six").unwrap(), "requests>=2\n# pinned\n-r other.txt\n");
    }

    #[test]
    fn names() {
        assert_eq!(requirement_name("requests[socks]>=2.0 ; python_version > '3'"), Some("requests"));
        assert_eq!(requirement_name("-e ."), None);
        assert_eq!(requirement_name("pkg @ git+https://x/y"), None);
        assert!(python_idents("Pillow").contains(&"PIL".to_string()));
        assert!(python_idents("python-slugify").contains(&"slugify".to_string()));
        assert!(python_idents("google-cloud-storage").contains(&"google".to_string()));
        assert!(npm_tooling("@types/node") && npm_tooling("css-loader") && !npm_tooling("left-pad"));
        assert!(contains_word("\"build\": \"tsc -p .\"", "tsc"));
        assert!(!contains_word("\"x\": \"@scope/tsc-like\"", "tsc"));
    }
}
