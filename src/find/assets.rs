//! Dead weight: images, fonts, sounds and other shipped files that nothing names, and the same file stored twice.
//!
//! An asset is found by its name in a string, so the test is the same as for a dead file: no code,
//! config or README mentions it. Tests rarely load assets, so the check proves less here than for
//! code. That's why anything loaded by pattern (sprite_01..09, a folder read at run time) is left for a reviewer.

use crate::candidate::{CandKind, Candidate, Edit, Tier};
use crate::find::dupes::{Copy, DupeGroup};
use crate::find::files::LOADERS;
use crate::index::Index;
use crate::util;
use crate::walk;
use anyhow::Result;
use std::collections::{HashMap, HashSet};

/// Names a platform, a store or a browser asks for by convention.
const KEEP: &[&str] = &[
    "favicon",
    "robots",
    "icon",
    "logo",
    "apple-touch-icon",
    "android-chrome",
    "mstile",
    "og",
    "og-image",
    "opengraph-image",
    "twitter-image",
    "screenshot",
    "banner",
    "cover",
    "splash",
    "default",
    "placeholder",
    "social",
    "preview",
    "thumbnail",
    "appicon",
    "launchimage",
    "readme",
    "hero",
    "ic_launcher",
    "ic_launcher_round",
    "ic_launcher_foreground",
    "ic_launcher_background",
    "launch_background",
    "store",
    "feature",
];

/// Folders whose files are used by where they are.
const KEEP_DIRS: &[&str] =
    &[".github", "docs", "doc", "screenshots", "fastlane", "metadata", ".vscode", "store", "licenses"];

/// Smaller duplicates aren't worth an agent's time.
const MIN_DUPLICATE: u64 = 2048;

/// "hero" from "hero@2x.png", "btn" from "btn.9.png".
fn stem_of(name: &str) -> &str {
    let stem = name.rsplit_once('.').map_or(name, |(stem, _)| stem);
    let stem = stem.strip_suffix(".9").unwrap_or(stem);
    stem.rsplit_once('@').filter(|(_, scale)| scale.ends_with('x')).map_or(stem, |(base, _)| base)
}

/// "sprite_" from "sprite_07", "wallpaper-" from "wallpaper-1600x1200": the part a loop or a template builds the name from.
fn series_of(stem: &str) -> Option<&str> {
    let digits = |s: &str| -> usize { s.len() - s.trim_end_matches(|c: char| c.is_ascii_digit()).len() };
    let mut cut = digits(stem);
    if cut > 0 && stem[..stem.len() - cut].ends_with('x') {
        let wide = digits(&stem[..stem.len() - cut - 1]);
        if wide > 0 {
            cut += 1 + wide;
        }
    }
    let base = &stem[..stem.len() - cut];
    (cut > 0 && base.len() >= 3).then_some(base)
}

pub fn find(ix: &Index) -> Result<(Vec<Candidate>, Vec<DupeGroup>)> {
    // Game engines refer to assets by id, not by name: nothing here can be judged from text.
    let by_id = ix.root.join("ProjectSettings").is_dir()
        || std::fs::read_dir(&ix.root)
            .is_ok_and(|rd| rd.flatten().any(|e| e.file_name().to_string_lossy().ends_with(".uproject")));
    if by_id {
        return Ok((Vec::new(), Vec::new()));
    }
    let loader_dirs: HashSet<&str> = ix
        .files
        .iter()
        .filter(|f| f.entry.lang.is_some() && LOADERS.iter().any(|m| f.refs().contains(m)))
        .map(|f| f.entry.dir())
        .collect();
    let mut wanted: Vec<(usize, Vec<String>)> = Vec::new();
    for (ai, asset) in ix.assets.iter().enumerate() {
        let (dir, name) = asset.rel.rsplit_once('/').unwrap_or(("", &asset.rel));
        let stem = stem_of(name);
        let lower = stem.to_ascii_lowercase();
        let conventional = KEEP.contains(&lower.as_str())
            || ["icon", "logo", "favicon", "social", "preview", "screenshot", "banner", "og-image", "badge", "demo"]
                .iter()
                .any(|w| lower.contains(w));
        let placed = dir.split('/').any(|part| KEEP_DIRS.contains(&part.to_ascii_lowercase().as_str()));
        if stem.len() < 3 || conventional || placed || walk::is_test(&asset.rel) {
            continue;
        }
        let mut needles = vec![stem.to_string()];
        needles.extend(series_of(stem).map(String::from));
        // the folder's name right before a quote or an interpolation: "img/flags/" + code, flags/{{ code }}.svg
        let folder = dir.rsplit('/').next().unwrap_or_default();
        if folder.len() >= 3 {
            needles.extend(["{", "$", "%", "\"", "'", "\x60", "<"].iter().map(|after| format!("{folder}/{after}")));
        }
        wanted.push((ai, needles));
    }
    let needles: Vec<String> = wanted.iter().flat_map(|(_, n)| n.iter().cloned()).collect();
    let hits = ix.search(&needles)?;
    let mut out = Vec::new();
    let mut at = 0;
    for (ai, own) in &wanted {
        let mentioned = hits[at..at + own.len()].iter().any(|h| !h.files.is_empty() || h.docs);
        at += own.len();
        if mentioned {
            continue;
        }
        let asset = &ix.assets[*ai];
        let (dir, name) = asset.rel.rsplit_once('/').unwrap_or(("", &asset.rel));
        let dir_name = dir.rsplit('/').next().unwrap_or_default();
        let parent = dir.rsplit_once('/').map_or("", |(up, _)| up);
        // A folder that's only copied into the build doesn't make its files used; one read by pattern does.
        let risk = if ix.path_names.contains(&format!("*{dir_name}")) {
            Some("in a folder that code reads by pattern")
        } else if loader_dirs.contains(dir) || loader_dirs.contains(parent) {
            Some("next to code that loads files by path")
        } else if dir.split('/').any(|part| ["public", "static", "www", "wwwroot", "htdocs", "web"].contains(&part)) {
            Some("in a folder served as it is: a page outside this repo may link to it")
        } else {
            None
        };
        let stem = stem_of(name);
        out.push(Candidate {
            id: format!("asset:{}", asset.rel),
            kind: CandKind::Asset,
            tier: if risk.is_some() { Tier::Risky } else { Tier::Safe },
            name: name.to_string(),
            path: asset.rel.clone(),
            line: 0,
            lines: 0,
            bytes: asset.bytes,
            why: match risk {
                Some(risk) => format!("no code, config or README mentions {stem}, but it is {risk}"),
                None => format!("no code, config or README mentions {stem}"),
            },
            edits: vec![Edit::Delete { path: asset.rel.clone() }],
        });
    }

    // the same bytes stored twice
    let mut by_size: HashMap<u64, Vec<usize>> = HashMap::new();
    for (ai, asset) in ix.assets.iter().enumerate().filter(|(_, a)| a.bytes >= MIN_DUPLICATE) {
        by_size.entry(asset.bytes).or_default().push(ai);
    }
    let mut groups = Vec::new();
    for (bytes, same_size) in by_size.into_iter().filter(|(_, g)| g.len() > 1) {
        let mut by_hash: HashMap<u64, Vec<usize>> = HashMap::new();
        for ai in same_size {
            if let Ok(content) = std::fs::read(ix.root.join(&ix.assets[ai].rel)) {
                by_hash.entry(util::hash(&content)).or_default().push(ai);
            }
        }
        for same in by_hash.into_values().filter(|g| g.len() > 1) {
            groups.push(DupeGroup {
                kind: "asset".into(),
                lines: 0,
                reclaim: 0,
                bytes,
                copies: same
                    .iter()
                    .map(|ai| Copy { path: ix.assets[*ai].rel.clone(), start: 0, end: 0, name: String::new() })
                    .collect(),
            });
        }
    }
    groups.sort_by(|a, b| {
        b.reclaim_bytes().cmp(&a.reclaim_bytes()).then_with(|| a.copies[0].path.cmp(&b.copies[0].path))
    });
    Ok((out, groups))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names() {
        assert_eq!(stem_of("hero@2x.png"), "hero");
        assert_eq!(stem_of("btn.9.png"), "btn");
        assert_eq!(stem_of("a.b.svg"), "a.b");
        assert_eq!(series_of("sprite_07"), Some("sprite_"));
        assert_eq!(series_of("sprite"), None);
        assert_eq!(series_of("wallpaper-1600x1200"), Some("wallpaper-"));
        assert_eq!(series_of("a1"), None);
    }
}
