//! Ranked workspace map: files, exports, import edges, PageRank, prompt projection.
//!
//! Spec: `docs/research/reference-product-port/README.md` (E3).

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;
use std::time::SystemTime;

mod graph;
mod parse;
mod project;
mod scan;

pub use parse::Language;
pub use project::render;
pub use scan::list_files;

/// Crate version, mirrors the workspace release.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Debug, Clone)]
pub struct FileRecord {
    pub path: String,
    pub language: Language,
    pub exports: Vec<String>,
    pub imports: Vec<String>,
    pub mtime: SystemTime,
}

#[derive(Debug, Clone, Default)]
pub struct Genome {
    pub files: HashMap<String, FileRecord>,
    pub ranks: HashMap<String, f64>,
    pub dependents: HashMap<String, usize>,
}

impl Genome {
    pub fn index(root: impl AsRef<Path>) -> std::io::Result<Self> {
        let root = root.as_ref();
        let listed = list_files(root)?;
        let known: HashSet<String> = listed.iter().map(|file| file.path.clone()).collect();
        let mut files = HashMap::new();
        for file in listed {
            let source = fs::read_to_string(&file.abs).unwrap_or_default();
            let parsed = parse::parse(&file.path, &source, &known);
            files.insert(
                file.path.clone(),
                FileRecord {
                    language: Language::from_path(&file.path),
                    path: file.path,
                    exports: parsed.exports,
                    imports: parsed.imports,
                    mtime: file.mtime,
                },
            );
        }
        let (ranks, dependents) = graph::rank(&files);
        Ok(Self {
            files,
            ranks,
            dependents,
        })
    }

    pub fn project(&self, limit: usize) -> String {
        render(self, limit)
    }
}
