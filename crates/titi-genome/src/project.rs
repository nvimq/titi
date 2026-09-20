use std::collections::HashSet;
use std::time::Duration;

use crate::Genome;

const NEW_WINDOW: Duration = Duration::from_secs(48 * 3600);

/// Multiplier applied to files the session just edited or read, so the map
/// follows the work instead of the static graph alone.
const TOUCHED_BOOST: f64 = 3.0;

pub fn render(genome: &Genome, limit: usize, touched: &HashSet<String>) -> String {
    let mut ranked: Vec<(&str, f64)> = genome
        .files
        .keys()
        // Angle brackets would forge a closing tag and break the frame.
        .filter(|path| !path.contains(['<', '>']))
        .map(|path| {
            let base = genome.ranks.get(path).copied().unwrap_or(0.0);
            let score = if touched.contains(path) {
                base * TOUCHED_BOOST
            } else {
                base
            };
            (path.as_str(), score)
        })
        .collect();
    ranked.sort_by(|a, b| {
        b.1.partial_cmp(&a.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.0.cmp(b.0))
    });
    ranked.truncate(limit.max(1));

    let mut out = String::from("<genome>\n");
    for (path, _) in ranked {
        let file = &genome.files[path];
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
