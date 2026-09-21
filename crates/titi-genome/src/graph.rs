use std::collections::{HashMap, HashSet};

use crate::{FileRecord, SymbolRecord};

const DAMPING: f64 = 0.85;
const ITERATIONS: usize = 20;

pub fn rank(
    files: &HashMap<String, FileRecord>,
    symbols: &HashMap<String, SymbolRecord>,
) -> (HashMap<String, f64>, HashMap<String, usize>) {
    let nodes: Vec<String> = files.keys().cloned().collect();
    let n = nodes.len();
    let mut dependents: HashMap<String, usize> = nodes.iter().map(|k| (k.clone(), 0)).collect();
    let mut inbound: HashMap<String, Vec<String>> = HashMap::new();
    let mut outbound: HashMap<String, usize> = HashMap::new();

    for (from, record) in files {
        // Two edge kinds, one graph: a file imports another, or it mentions a
        // symbol that other file defines. Either way it depends on it.
        let mut targets: HashSet<&String> = record.imports.iter().collect();
        for name in &record.used_symbols {
            if let Some(symbol) = symbols.get(name) {
                for definer in &symbol.files {
                    if definer != from && files.contains_key(definer) {
                        targets.insert(definer);
                    }
                }
            }
        }
        outbound.insert(from.clone(), targets.len().max(1));
        for to in targets {
            if files.contains_key(to) {
                inbound.entry(to.clone()).or_default().push(from.clone());
                *dependents.entry(to.clone()).or_default() += 1;
            }
        }
    }

    if n == 0 {
        return (HashMap::new(), dependents);
    }

    let mut score: HashMap<String, f64> =
        nodes.iter().map(|k| (k.clone(), 1.0 / n as f64)).collect();
    for _ in 0..ITERATIONS {
        let mut next = HashMap::new();
        for node in &nodes {
            let mut incoming = 0.0;
            if let Some(sources) = inbound.get(node) {
                for src in sources {
                    let out = *outbound.get(src).unwrap_or(&1) as f64;
                    incoming += score.get(src).copied().unwrap_or(0.0) / out;
                }
            }
            next.insert(
                node.clone(),
                (1.0 - DAMPING) / n as f64 + DAMPING * incoming,
            );
        }
        score = next;
    }
    (score, dependents)
}
