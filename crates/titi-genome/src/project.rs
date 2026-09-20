use std::time::Duration;

use crate::Genome;

const NEW_WINDOW: Duration = Duration::from_secs(48 * 3600);

pub fn render(genome: &Genome, limit: usize) -> String {
    let mut ranked: Vec<_> = genome.files.values().collect();
    ranked.sort_by(|a, b| {
        let ra = genome.ranks.get(&a.path).copied().unwrap_or(0.0);
        let rb = genome.ranks.get(&b.path).copied().unwrap_or(0.0);
        rb.partial_cmp(&ra)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.path.cmp(&b.path))
    });
    ranked.truncate(limit.max(1));

    let mut out = String::from("<genome>\n");
    for file in ranked {
        let dependents = genome.dependents.get(&file.path).copied().unwrap_or(0);
        let new = file
            .mtime
            .elapsed()
            .ok()
            .is_some_and(|elapsed| elapsed <= NEW_WINDOW);
        out.push_str(&file.path);
        out.push_str(":(→");
        out.push_str(&dependents.to_string());
        out.push(')');
        if new {
            out.push_str(" [NEW]");
        }
        out.push('\n');
        for name in file.exports.iter().take(8) {
            out.push_str("  +");
            out.push_str(name);
            out.push('\n');
        }
    }
    out.push_str("</genome>");
    out
}
