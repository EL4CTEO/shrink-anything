//! Something that can go, and the edits that remove it.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize, clap::ValueEnum)]
#[serde(rename_all = "lowercase")]
pub enum CandKind {
    /// A function, method, type or constant nothing mentions.
    #[value(name = "code")]
    #[serde(rename = "code")]
    Symbol,
    /// A file nothing imports or names.
    File,
    /// A dependency nothing uses.
    Dep,
    /// An image, font or other shipped file nothing names.
    Asset,
}

impl CandKind {
    pub fn label(self) -> &'static str {
        match self {
            CandKind::Symbol => "code",
            CandKind::File => "file",
            CandKind::Dep => "dep",
            CandKind::Asset => "asset",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Tier {
    /// Cut by default: nothing mentions it and nothing can reach it another way.
    Safe,
    /// Nothing mentions it, but something could reach it without naming it. Needs --aggressive or a reviewer.
    Risky,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Eco {
    Npm,
    Cargo,
    Requirements,
    Pyproject,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "edit", rename_all = "lowercase")]
pub enum Edit {
    /// Lines start..=end (0-based) of the file as it was when the scan ran.
    Remove {
        path: String,
        start: usize,
        end: usize,
    },
    Delete {
        path: String,
    },
    Dep {
        path: String,
        eco: Eco,
        section: String,
        name: String,
    },
}

impl Edit {
    pub fn path(&self) -> &str {
        match self {
            Edit::Remove { path, .. } | Edit::Delete { path } | Edit::Dep { path, .. } => path,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Candidate {
    pub id: String,
    pub kind: CandKind,
    pub tier: Tier,
    pub name: String,
    pub path: String,
    /// 1-based; 0 for a whole file.
    pub line: usize,
    /// Lines that go with it.
    pub lines: usize,
    /// Bytes that go with it, for an asset.
    #[serde(default)]
    pub bytes: u64,
    pub why: String,
    #[serde(default, skip_serializing)]
    pub edits: Vec<Edit>,
}
