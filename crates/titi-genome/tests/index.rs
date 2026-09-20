use std::fs;
use std::path::Path;

use titi_genome::Genome;

fn write(root: &Path, rel: &str, body: &str) {
    let path = root.join(rel);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, body).unwrap();
}

#[test]
fn indexes_rust_graph_and_projects_ranked_map() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    write(
        root,
        "src/lib.rs",
        r#"
pub mod auth;
pub mod util;
pub use auth::Session;
"#,
    );
    write(
        root,
        "src/auth.rs",
        r#"
use crate::util::hash;
pub struct Session;
pub fn login() { let _ = hash(); }
"#,
    );
    write(
        root,
        "src/util.rs",
        r#"
pub fn hash() {}
"#,
    );
    write(root, "src/scratch.txt", "ignored");
    write(root, "target/debug/lib.rs", "pub fn noise() {}");
    write(root, ".gitignore", "target/\n");

    let genome = Genome::index(root).unwrap();
    assert!(genome.files.contains_key("src/lib.rs"));
    assert!(genome.files.contains_key("src/auth.rs"));
    assert!(genome.files.contains_key("src/util.rs"));
    assert!(!genome.files.contains_key("src/scratch.txt"));
    assert!(!genome.files.keys().any(|path| path.starts_with("target/")));

    assert!(genome.files["src/lib.rs"]
        .exports
        .iter()
        .any(|name| name == "Session" || name == "auth" || name == "util"));
    assert!(genome.files["src/auth.rs"]
        .exports
        .contains(&"Session".into()));
    assert!(genome.files["src/auth.rs"]
        .imports
        .contains(&"src/util.rs".into()));
    assert!(genome.files["src/lib.rs"]
        .imports
        .contains(&"src/auth.rs".into()));

    let util_rank = genome.ranks["src/util.rs"];
    let auth_rank = genome.ranks["src/auth.rs"];
    assert!(util_rank >= auth_rank, "util={util_rank} auth={auth_rank}");
    // util is depended on by both lib (`pub mod util`) and auth (`use crate::util`).
    assert_eq!(genome.dependents["src/util.rs"], 2);
    assert_eq!(genome.dependents["src/auth.rs"], 1);

    let projected = genome.project(8);
    assert!(projected.contains("<genome>"));
    assert!(projected.contains("src/util.rs:(→2)"));
    assert!(projected.contains("+hash") || projected.contains("+Session"));
}

#[test]
fn indexes_this_workspace() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    let genome = Genome::index(root).unwrap();

    assert!(
        genome.files.len() > 20,
        "expected a real workspace, got {}",
        genome.files.len()
    );
    assert!(genome
        .files
        .contains_key("crates/titi-engine/src/runtime.rs"));
    assert!(genome.files.contains_key("crates/titi-genome/src/scan.rs"));
    assert!(
        !genome.files.keys().any(|path| path.starts_with("target/")),
        "target/ must be pruned"
    );

    // runtime.rs is imported by many crates and must outrank a leaf module.
    let engine = genome.ranks["crates/titi-engine/src/runtime.rs"];
    let leaf = genome.ranks["crates/titi-genome/src/scan.rs"];
    assert!(engine > leaf, "engine={engine} leaf={leaf}");

    let projected = genome.project(12);
    assert!(projected.starts_with("<genome>\n"));
    assert!(projected.ends_with("</genome>"));
    assert!(projected.lines().count() > 6, "{projected}");
}

#[test]
fn empryoignore_hides_from_discovery() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    write(root, "src/keep.rs", "pub fn keep() {}");
    write(root, "fixtures/noise.rs", "pub fn noise() {}");
    write(root, ".empryoignore", "fixtures/\n");

    let genome = Genome::index(root).unwrap();
    assert!(genome.files.contains_key("src/keep.rs"));
    assert!(!genome.files.keys().any(|path| path.contains("fixtures")));
}

#[test]
fn typescript_relative_imports_resolve() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    write(
        root,
        "src/index.ts",
        r#"
import { Session } from "./auth";
export function boot() {}
"#,
    );
    write(
        root,
        "src/auth.ts",
        r#"
export class Session {}
"#,
    );

    let genome = Genome::index(root).unwrap();
    assert!(genome.files["src/index.ts"]
        .imports
        .contains(&"src/auth.ts".into()));
    assert!(genome.files["src/auth.ts"]
        .exports
        .contains(&"Session".into()));
    assert_eq!(genome.dependents["src/auth.ts"], 1);
}
