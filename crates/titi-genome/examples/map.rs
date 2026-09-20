//! Print the Genome projection for a workspace: `cargo run -p titi-genome --example map -- [path]`

fn main() -> std::io::Result<()> {
    let root = std::env::args()
        .nth(1)
        .unwrap_or_else(|| ".".to_owned());
    let genome = titi_genome::Genome::index(&root)?;
    let limit = std::env::args()
        .nth(2)
        .and_then(|raw| raw.parse().ok())
        .unwrap_or(24);
    eprintln!(
        "{} files, {} edges",
        genome.files.len(),
        genome.dependents.values().sum::<usize>()
    );
    print!("{}", genome.project(limit));
    Ok(())
}
