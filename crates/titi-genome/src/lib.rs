//! Ranked workspace map: files, exports, import edges, PageRank, prompt projection.
//!
//! Spec: `docs/research/empryo-port/README.md` (E3).

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
    pub size: u64,
    pub mtime: SystemTime,
}

/// What one [`Genome::refresh`] actually did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RefreshStats {
    /// Files re-read and re-parsed (new or changed).
    pub parsed: usize,
    /// Files dropped because they vanished from disk.
    pub removed: usize,
    /// Files in the index afterwards.
    pub total: usize,
}

#[derive(Debug, Clone, Default)]
pub struct Genome {
    pub files: HashMap<String, FileRecord>,
    pub ranks: HashMap<String, f64>,
    pub dependents: HashMap<String, usize>,
}

impl Genome {
    pub fn index(root: impl AsRef<Path>) -> std::io::Result<Self> {
        let mut genome = Self::default();
        genome.refresh(root)?;
        Ok(genome)
    }

    /// Re-walk `root` and re-parse only the files whose size or mtime moved.
    /// Removed files drop out; ranks are recomputed every time (it is cheap
    /// relative to parsing).
    pub fn refresh(&mut self, root: impl AsRef<Path>) -> std::io::Result<RefreshStats> {
        let root = root.as_ref();
        let listed = scan::list_files(root)?;
        let known: HashSet<String> = listed.iter().map(|file| file.path.clone()).collect();
        let mut parsed = 0;

        for file in &listed {
            let unchanged = self
                .files
                .get(&file.path)
                .is_some_and(|record| record.size == file.size && record.mtime == file.mtime);
            if unchanged {
                continue;
            }
            let source = fs::read_to_string(&file.abs).unwrap_or_default();
            let result = parse::parse(&file.path, &source, &known);
            self.files.insert(
                file.path.clone(),
                FileRecord {
                    language: Language::from_path(&file.path),
                    path: file.path.clone(),
                    exports: result.exports,
                    imports: result.imports,
                    size: file.size,
                    mtime: file.mtime,
                },
            );
            parsed += 1;
        }

        let before = self.files.len();
        self.files.retain(|path, _| known.contains(path));
        let removed = before - self.files.len();

        let (ranks, dependents) = graph::rank(&self.files);
        self.ranks = ranks;
        self.dependents = dependents;
        Ok(RefreshStats {
            parsed,
            removed,
            total: self.files.len(),
        })
    }

    pub fn project(&self, limit: usize) -> String {
        render(self, limit, &HashSet::new())
    }

    /// Projection biased toward files this session edited or read.
    pub fn project_with(&self, limit: usize, touched: &[String]) -> String {
        let touched: HashSet<String> = touched.iter().cloned().collect();
        render(self, limit, &touched)
    }
}
