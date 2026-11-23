//! Where the weight of a build is: what kinds of file, which files, and what's in there twice.

use crate::util;
use anyhow::{Result, bail};
use serde::Serialize;
use std::collections::{BTreeMap, HashMap};
use std::path::Path;

/// Folders a build usually lands in, tried when none is given.
const USUAL: &[&str] = &["dist", "build", "out", ".next", "public", "target/release", "bin/Release", "release"];

/// Smaller duplicates don't move the total.
const MIN_DUPLICATE: u64 = 4096;

#[derive(Serialize)]
pub struct Weight {
    pub paths: Vec<String>,
    pub bytes: u64,
    pub files: u64,
    /// (extension, bytes, files), heaviest first.
    pub kinds: Vec<(String, u64, u64)>,
    /// (path, bytes), heaviest first.
    pub heaviest: Vec<(String, u64)>,
    /// Source maps: useful for debugging, usually not meant to ship.
    pub source_maps: u64,
    /// Bytes spent on second and later copies of the same file.
    pub duplicated: u64,
    /// (bytes of one copy, paths)
    pub duplicates: Vec<(u64, Vec<String>)>,
}

fn collect(root: &Path, dir: &Path, out: &mut Vec<(String, u64)>) {
    let Ok(meta) = std::fs::symlink_metadata(dir) else { return };
    if meta.is_file() {
        let rel = dir.strip_prefix(root).unwrap_or(dir);
        out.push((util::forward_slashes(rel), meta.len()));
    } else if meta.is_dir() {
        for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
            collect(root, &entry.path(), out);
        }
    }
}

/// paths: files or folders under root. Empty: the usual build folders that exist.
pub fn weigh(root: &Path, paths: &[String]) -> Result<Weight> {
    let paths: Vec<String> = if paths.is_empty() {
        USUAL.iter().filter(|p| root.join(p).exists()).map(|p| p.to_string()).collect()
    } else {
        paths.to_vec()
    };
    if paths.is_empty() {
        bail!("no build folder found here. Name one: shrink-anything weigh dist");
    }
    let mut files: Vec<(String, u64)> = Vec::new();
    for p in &paths {
        if !root.join(p).exists() {
            bail!("{p} doesn't exist. Build the project first?");
        }
        collect(root, &root.join(p), &mut files);
    }
    let bytes = files.iter().map(|f| f.1).sum();
    let mut kinds: BTreeMap<String, (u64, u64)> = BTreeMap::new();
    for (path, size) in &files {
        let name = path.rsplit('/').next().unwrap_or(path);
        let ext = name.rsplit_once('.').map_or("(none)".to_string(), |(_, e)| format!(".{}", e.to_ascii_lowercase()));
        let kind = kinds.entry(ext).or_default();
        kind.0 += size;
        kind.1 += 1;
    }
    let mut kinds: Vec<(String, u64, u64)> = kinds.into_iter().map(|(ext, (b, n))| (ext, b, n)).collect();
    kinds.sort_by_key(|kind| std::cmp::Reverse(kind.1));
    let source_maps = files.iter().filter(|f| f.0.ends_with(".map")).map(|f| f.1).sum();

    let mut by_size: HashMap<u64, Vec<&str>> = HashMap::new();
    for (path, size) in files.iter().filter(|f| f.1 >= MIN_DUPLICATE) {
        by_size.entry(*size).or_default().push(path);
    }
    let mut duplicates: Vec<(u64, Vec<String>)> = Vec::new();
    for (size, same_size) in by_size.into_iter().filter(|(_, g)| g.len() > 1) {
        let mut by_hash: HashMap<u64, Vec<String>> = HashMap::new();
        for path in same_size {
            if let Ok(content) = std::fs::read(root.join(path)) {
                by_hash.entry(util::hash(&content)).or_default().push(path.to_string());
            }
        }
        duplicates.extend(by_hash.into_values().filter(|g| g.len() > 1).map(|mut g| {
            g.sort();
            (size, g)
        }));
    }
    duplicates.sort_by(|a, b| {
        (b.0 * (b.1.len() as u64 - 1)).cmp(&(a.0 * (a.1.len() as u64 - 1))).then_with(|| a.1.cmp(&b.1))
    });
    let duplicated = duplicates.iter().map(|(size, g)| size * (g.len() as u64 - 1)).sum();

    files.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    Ok(Weight {
        paths,
        bytes,
        files: files.len() as u64,
        kinds,
        heaviest: files.into_iter().take(10).collect(),
        source_maps,
        duplicated,
        duplicates,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::tests::TempDir;

    #[test]
    fn weighs_a_build() {
        let tmp = TempDir::new("weigh");
        tmp.write("dist/app.js", &"x".repeat(9000));
        tmp.write("dist/app.js.map", &"m".repeat(5000));
        tmp.write("dist/img/a.png", &"p".repeat(6000));
        tmp.write("dist/img/b.png", &"p".repeat(6000));
        tmp.write("dist/img/c.png", &"q".repeat(6000));
        let w = weigh(&tmp.0, &[]).unwrap();
        assert_eq!((w.paths.as_slice(), w.bytes, w.files), (["dist".to_string()].as_slice(), 32_000, 5));
        assert_eq!(w.kinds[0], (".png".to_string(), 18_000, 3));
        assert_eq!(w.heaviest[0], ("dist/app.js".to_string(), 9000));
        assert_eq!(w.source_maps, 5000);
        assert_eq!(w.duplicated, 6000);
        assert_eq!(w.duplicates, [(6000, vec!["dist/img/a.png".to_string(), "dist/img/b.png".to_string()])]);
        assert!(weigh(&tmp.0, &["nope".to_string()]).is_err());
    }
}
