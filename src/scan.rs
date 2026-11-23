//! One pass over the project: what's there, and what could go.

use crate::candidate::{CandKind, Candidate, Edit, Tier};
use crate::find::{self, dupes::DupeGroup};
use crate::index::Index;
use crate::walk::Role;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

#[derive(Clone, Copy, Default, PartialEq, Debug, Serialize, Deserialize)]
pub struct Metrics {
    pub files: u64,
    /// Every line of every source file.
    pub lines: u64,
    /// Lines with code on them: no blanks, no comments.
    pub code_lines: u64,
    pub bytes: u64,
    pub deps: u64,
    /// Images, fonts and other files shipped as they are.
    #[serde(default)]
    pub assets: u64,
    #[serde(default)]
    pub asset_bytes: u64,
}

impl Metrics {
    /// A rough count of what an AI agent pays to read the code: four characters a token.
    pub fn tokens(&self) -> u64 {
        self.bytes / 4
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Scan {
    pub metrics: Metrics,
    /// Source files per language, most first.
    pub languages: Vec<(String, u64)>,
    /// Languages treated as a library: their public names are kept for --aggressive.
    pub library: Vec<String>,
    pub candidates: Vec<Candidate>,
    pub duplicates: Vec<DupeGroup>,
}

impl Scan {
    pub fn tier(&self, tier: Tier) -> impl Iterator<Item = &Candidate> {
        self.candidates.iter().filter(move |c| c.tier == tier)
    }

    pub fn lines(&self, tier: Tier) -> usize {
        self.tier(tier).filter(|c| c.kind != CandKind::Dep).map(|c| c.lines).sum()
    }

    /// Bytes of assets that can go in this tier.
    pub fn bytes(&self, tier: Tier) -> u64 {
        self.tier(tier).map(|c| c.bytes).sum()
    }

    pub fn duplicate_bytes(&self) -> u64 {
        self.duplicates.iter().map(|d| d.reclaim_bytes()).sum()
    }

    pub fn duplicate_lines(&self) -> usize {
        self.duplicates.iter().map(|d| d.reclaim).sum()
    }
}

fn metrics(ix: &Index, deps: u64) -> (Metrics, Vec<(String, u64)>) {
    let mut m = Metrics {
        deps,
        assets: ix.assets.len() as u64,
        asset_bytes: ix.assets.iter().map(|a| a.bytes).sum(),
        ..Metrics::default()
    };
    let mut langs: BTreeMap<&str, u64> = BTreeMap::new();
    for file in ix.files.iter().filter(|f| f.entry.role == Role::Source && !f.entry.generated) {
        m.files += 1;
        m.lines += file.line_count() as u64;
        m.code_lines += file.code.lines().filter(|l| !l.trim().is_empty()).count() as u64;
        m.bytes += file.entry.text.len() as u64;
        if let Some(lang) = file.entry.lang {
            *langs.entry(lang.spec.name).or_insert(0) += 1;
        }
    }
    let mut langs: Vec<(String, u64)> = langs.into_iter().map(|(name, n)| (name.to_string(), n)).collect();
    langs.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    (m, langs)
}

/// Drops candidates that another candidate already removes: code inside a dead file, a method inside a dead class.
fn drop_nested(candidates: &mut Vec<Candidate>) {
    let dead_files: Vec<String> = candidates
        .iter()
        .flat_map(|c| c.edits.iter())
        .filter_map(|e| if let Edit::Delete { path } = e { Some(path.clone()) } else { None })
        .collect();
    let spans: Vec<(usize, String, usize, usize)> = candidates
        .iter()
        .enumerate()
        .flat_map(|(i, c)| {
            c.edits.iter().filter_map(move |e| match e {
                Edit::Remove { path, start, end } => Some((i, path.clone(), *start, *end)),
                _ => None,
            })
        })
        .collect();
    let nested: Vec<bool> = candidates
        .iter()
        .enumerate()
        .map(|(i, c)| {
            c.kind == CandKind::Symbol
                && c.edits.iter().all(|e| match e {
                    Edit::Remove { path, start, end } => {
                        dead_files.contains(path)
                            || spans.iter().any(|(j, p, s, e2)| {
                                *j != i && p == path && s <= start && end <= e2 && (s, e2) != (start, end)
                            })
                    }
                    _ => false,
                })
        })
        .collect();
    let mut keep = nested.iter().map(|n| !n);
    candidates.retain(|_| keep.next().unwrap_or(true));
}

/// library: Some(true) treats the project as a library, Some(false) as an app, None guesses per language.
pub fn scan(root: &Path, library: Option<bool>) -> Result<Scan> {
    let ix = Index::build(root, library)?;
    let (dep_count, mut candidates) = find::deps::find(&ix)?;
    candidates.extend(find::files::find(&ix)?);
    candidates.extend(find::symbols::find(&ix));
    let (dead_assets, same_assets) = find::assets::find(&ix)?;
    candidates.extend(dead_assets);
    drop_nested(&mut candidates);
    candidates.sort_by(|a, b| {
        (a.tier, a.kind as u8, std::cmp::Reverse(a.lines as u64 + a.bytes), &a.path, a.line).cmp(&(
            b.tier,
            b.kind as u8,
            std::cmp::Reverse(b.lines as u64 + b.bytes),
            &b.path,
            b.line,
        ))
    });
    let (metrics, languages) = metrics(&ix, dep_count);
    let library =
        if ix.lib.all { vec!["everything".to_string()] } else { ix.lib.langs.iter().map(|l| l.to_string()).collect() };
    let mut duplicates = find::dupes::find(&ix);
    duplicates.extend(same_assets);
    Ok(Scan { metrics, languages, library, candidates, duplicates })
}
