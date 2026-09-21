//! Builds the recap sections from what the session actually persisted.
//!
//! The transcript holds the conversation; the trajectory holds what the agent
//! did — tool calls, their durations, their outcomes. The recap joins both so
//! the answer to "what happened here" is on one screen, with each block
//! collapsible.

use std::collections::BTreeMap;
use std::path::Path;

use titi_core::session::{Role, SessionStore};
use titi_core::trajectory::{EventKind, TrajectoryRecorder};
use titi_tui::recap::RecapSection;

/// Reads a session and returns its recap sections, collapsed-ready.
pub fn build(agent_dir: &Path, session_id: &str) -> Result<Vec<RecapSection>, String> {
    let store = SessionStore::new(agent_dir).map_err(|error| error.to_string())?;
    let entries = store.open(session_id).map_err(|error| error.to_string())?;
    let checkpoints = store
        .checkpoints(session_id)
        .map(|list| list.len())
        .unwrap_or(0);
    let events = TrajectoryRecorder::open(agent_dir, session_id)
        .map(|recorder| recorder.tail(usize::MAX))
        .unwrap_or_default();

    let now = now_ms();
    Ok(vec![
        session_section(session_id, &entries, checkpoints, now),
        turns_section(&entries),
        tools_section(&events),
        files_section(&events),
        problems_section(&events),
        trajectory_section(&events, now),
    ])
}

fn session_section(
    session_id: &str,
    entries: &[titi_core::session::Entry],
    checkpoints: usize,
    now: u64,
) -> RecapSection {
    let user = entries.iter().filter(|e| e.role == Role::User).count();
    let assistant = entries.iter().filter(|e| e.role == Role::Assistant).count();
    let system = entries.iter().filter(|e| e.role == Role::System).count();
    let span = match (entries.first(), entries.last()) {
        (Some(first), Some(last)) if entries.len() > 1 => {
            format!("{}, last {}", ago(first.ts, now), ago(last.ts, now))
        }
        (Some(first), _) => ago(first.ts, now),
        _ => "empty".to_owned(),
    };
    RecapSection::new(
        "Session",
        format!("{session_id} · {} entries", entries.len()),
        vec![
            format!("id: {session_id}"),
            format!("started: {span}"),
            format!("roles: {user} user, {assistant} assistant, {system} system"),
            format!("checkpoints: {checkpoints}"),
        ],
    )
}

fn turns_section(entries: &[titi_core::session::Entry]) -> RecapSection {
    let user = entries.iter().filter(|e| e.role == Role::User).count();
    let chars: usize = entries
        .iter()
        .filter(|e| e.role == Role::Assistant)
        .map(|e| e.content.chars().count())
        .sum();
    let longest = entries
        .iter()
        .filter(|e| e.role == Role::Assistant)
        .map(|e| e.content.chars().count())
        .max()
        .unwrap_or(0);
    let mut lines = vec![
        format!("prompts: {user}"),
        format!("replies: {chars} chars total, longest {longest}"),
    ];
    // The prompts themselves, newest last — what the session was actually about.
    for entry in entries.iter().filter(|e| e.role == Role::User) {
        let prompt: String = entry.content.lines().next().unwrap_or("").to_owned();
        lines.push(format!("· {prompt}"));
    }
    RecapSection::new("Turns", format!("{user} prompts"), lines)
}

fn tools_section(events: &[titi_core::trajectory::TrajectoryEvent]) -> RecapSection {
    let mut calls: BTreeMap<&str, usize> = BTreeMap::new();
    for event in events {
        if let EventKind::ToolCall { name, .. } = &event.kind {
            *calls.entry(name.as_str()).or_default() += 1;
        }
    }
    let total: usize = calls.values().sum();
    if total == 0 {
        return RecapSection::empty("Tools", "none called");
    }
    let mut ranked: Vec<(&str, usize)> = calls.into_iter().collect();
    ranked.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(b.0)));

    let mut lines = Vec::new();
    for (name, count) in &ranked {
        let (calls, spent, failed) = stats_for(events, name);
        let average = spent.checked_div(calls as u64).unwrap_or(0);
        lines.push(format!(
            "{name}: {count} call(s), {failed} failed, {} total, {} avg",
            duration(spent),
            duration(average)
        ));
    }
    RecapSection::new("Tools", format!("{total} calls"), lines)
}

/// `(calls with a result, total duration, failures)` for one tool name.
fn stats_for(events: &[titi_core::trajectory::TrajectoryEvent], name: &str) -> (usize, u64, usize) {
    let mut names: BTreeMap<&str, &str> = BTreeMap::new();
    for event in events {
        if let EventKind::ToolCall { id, name, .. } = &event.kind {
            names.insert(id.as_str(), name.as_str());
        }
    }
    let mut calls = 0;
    let mut spent = 0;
    let mut failed = 0;
    for event in events {
        if let EventKind::ToolResult {
            id,
            duration_ms,
            ok,
        } = &event.kind
            && names.get(id.as_str()) == Some(&name)
        {
            calls += 1;
            spent += duration_ms;
            if !ok {
                failed += 1;
            }
        }
    }
    (calls, spent, failed)
}

fn files_section(events: &[titi_core::trajectory::TrajectoryEvent]) -> RecapSection {
    let mut files: BTreeMap<String, usize> = BTreeMap::new();
    for event in events {
        if let EventKind::ToolCall { name, args, .. } = &event.kind
            && matches!(name.as_str(), "read" | "write" | "edit")
            && let Some(path) = args.get("path").and_then(|value| value.as_str())
        {
            *files.entry(path.to_owned()).or_default() += 1;
        }
    }
    if files.is_empty() {
        return RecapSection::empty("Files", "none touched");
    }
    let mut ranked: Vec<(String, usize)> = files.into_iter().collect();
    ranked.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    let lines: Vec<String> = ranked
        .iter()
        .map(|(path, count)| format!("{path} ×{count}"))
        .collect();
    RecapSection::new("Files", format!("{} touched", ranked.len()), lines)
}

fn problems_section(events: &[titi_core::trajectory::TrajectoryEvent]) -> RecapSection {
    let mut names: BTreeMap<&str, &str> = BTreeMap::new();
    for event in events {
        if let EventKind::ToolCall { id, name, .. } = &event.kind {
            names.insert(id.as_str(), name.as_str());
        }
    }
    let failures: Vec<String> = events
        .iter()
        .filter_map(|event| match &event.kind {
            EventKind::ToolResult {
                id,
                duration_ms,
                ok: false,
            } => Some(format!(
                "{} failed after {}",
                names.get(id.as_str()).copied().unwrap_or("tool"),
                duration(*duration_ms)
            )),
            _ => None,
        })
        .collect();
    if failures.is_empty() {
        return RecapSection::empty("Problems", "none");
    }
    RecapSection::new(
        "Problems",
        format!("{} failed tool call(s)", failures.len()),
        failures,
    )
}

fn trajectory_section(events: &[titi_core::trajectory::TrajectoryEvent], now: u64) -> RecapSection {
    if events.is_empty() {
        return RecapSection::empty("Trajectory", "nothing recorded");
    }
    let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
    for event in events {
        let kind = match &event.kind {
            EventKind::UserMessage { .. } => "user",
            EventKind::AssistantMessage { .. } => "assistant",
            EventKind::ToolCall { .. } => "tool_call",
            EventKind::ToolResult { .. } => "tool_result",
            EventKind::TurnEnd => "turn_end",
            EventKind::Compaction { .. } => "compaction",
            EventKind::GepaReview => "gepa_review",
        };
        *counts.entry(kind).or_default() += 1;
    }
    let first = events.first().map(|event| event.ts).unwrap_or(now);
    let last = events.last().map(|event| event.ts).unwrap_or(now);
    let lines: Vec<String> = counts
        .iter()
        .map(|(kind, count)| format!("{kind}: {count}"))
        .collect();
    RecapSection::new(
        "Trajectory",
        format!(
            "{} events, {} → {}",
            events.len(),
            ago(first, now),
            ago(last, now)
        ),
        lines,
    )
}

/// Milliseconds since the Unix epoch.
fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as u64)
        .unwrap_or(0)
}

/// A coarse "how long ago", which is all a recap needs.
fn ago(ts: u64, now: u64) -> String {
    let seconds = now.saturating_sub(ts) / 1000;
    match seconds {
        0..=4 => "just now".to_owned(),
        5..=59 => format!("{seconds}s ago"),
        60..=3599 => format!("{}m ago", seconds / 60),
        3600..=86_399 => format!("{}h ago", seconds / 3600),
        _ => format!("{}d ago", seconds / 86_400),
    }
}

/// Milliseconds as a compact duration.
fn duration(ms: u64) -> String {
    match ms {
        0..=999 => format!("{ms}ms"),
        1000..=59_999 => format!("{:.1}s", ms as f64 / 1000.0),
        _ => format!("{}m", ms / 60_000),
    }
}
