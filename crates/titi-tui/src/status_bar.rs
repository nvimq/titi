//! OMP default status-line product (`omp://theme.md` tokens, default preset).
//!
//! Left: `pi` · `model` · `mode` · `collab` · `path` · `git` · `pr` ·
//! `context_pct` · `cost` — invisible segments are dropped.
//! Right: `session_name`.
//! Separator: `powerline-thin`.

use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::SystemTime;

use crate::theme::{Theme, ThemeColor};
use crate::width::{truncate_to_width, visible_width};

/// Snapshot of values the default preset can paint. Empty optionals hide.
#[derive(Debug, Clone, PartialEq)]
pub struct StatusSnapshot {
    /// Active model id (provider prefix is stripped for display).
    pub model: String,
    /// Plan/prewalk/loop label; `None` hides the mode segment (plain agent).
    pub mode: Option<String>,
    /// Working directory, already absolute or `~` form.
    pub path: String,
    /// Git branch name.
    pub git_branch: Option<String>,
    pub git_unstaged: u32,
    pub git_staged: u32,
    pub git_untracked: u32,
    /// Open PR label.
    pub pr: Option<String>,
    /// Collab badge.
    pub collab: Option<String>,
    /// Context window fill 0–100.
    pub context_pct: Option<u8>,
    /// Session spend in USD; `None` or `0` hides.
    pub cost_usd: Option<f64>,
    /// Right-group session title.
    pub session_name: String,
}

impl Default for StatusSnapshot {
    fn default() -> Self {
        StatusSnapshot {
            model: "no-model".to_owned(),
            mode: None,
            path: String::new(),
            git_branch: None,
            git_unstaged: 0,
            git_staged: 0,
            git_untracked: 0,
            pr: None,
            collab: None,
            context_pct: None,
            cost_usd: None,
            session_name: String::new(),
        }
    }
}

/// Build a snapshot from the live process (cwd + git HEAD + porcelain dirty).
pub fn live_snapshot(model: &str, session_name: &str) -> StatusSnapshot {
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let path = abbreviate_path(&cwd.to_string_lossy(), 40);
    let git = git_info(&cwd);
    StatusSnapshot {
        model: short_model(model),
        path,
        git_branch: git.branch,
        git_unstaged: git.unstaged,
        git_staged: git.staged,
        git_untracked: git.untracked,
        session_name: session_name.to_owned(),
        ..StatusSnapshot::default()
    }
}

/// Paint the default-preset bar, padded/truncated to `width` cells.
pub fn render_status_bar(theme: &Theme, width: u16, snap: &StatusSnapshot) -> String {
    let inner = width as usize;
    if inner == 0 {
        return String::new();
    }

    let left = left_parts(theme, snap);
    let right = right_parts(theme, snap);
    let sep = theme.symbol("sep.powerlineThinLeft");
    let sep_s = if sep.is_empty() {
        " > ".to_owned()
    } else {
        format!(" {} ", theme.fg(ThemeColor::StatusLineSep, sep))
    };

    let left_s = left.join(&sep_s);
    let right_s = right.join(&sep_s);
    let left_w = visible_width(&left_s);
    let right_w = visible_width(&right_s);
    let gap = inner.saturating_sub(left_w.saturating_add(right_w));
    let mut line = if right_w == 0 {
        left_s
    } else if left_w + right_w >= inner {
        let keep = inner.saturating_sub(right_w.saturating_add(1));
        format!("{} {}", truncate_to_width(&left_s, keep), right_s)
    } else {
        format!("{left_s}{}{right_s}", " ".repeat(gap))
    };
    let w = visible_width(&line);
    if w < inner {
        line.push_str(&" ".repeat(inner - w));
    } else if w > inner {
        line = truncate_to_width(&line, inner);
    }
    line
}

fn left_parts(theme: &Theme, snap: &StatusSnapshot) -> Vec<String> {
    let mut parts = Vec::new();

    let pi = theme.symbol("icon.pi");
    if !pi.is_empty() {
        parts.push(theme.fg(ThemeColor::Accent, pi));
    }

    let model_icon = theme.symbol("icon.model");
    let model = if model_icon.is_empty() {
        snap.model.clone()
    } else {
        format!("{model_icon} {}", snap.model)
    };
    parts.push(theme.fg(ThemeColor::StatusLineModel, &model));

    if let Some(mode) = &snap.mode {
        parts.push(theme.fg(ThemeColor::Accent, mode));
    }
    if let Some(collab) = &snap.collab {
        parts.push(theme.fg(ThemeColor::Warning, collab));
    }

    if !snap.path.is_empty() {
        let folder = theme.symbol("icon.folder");
        let text = if folder.is_empty() {
            snap.path.clone()
        } else {
            format!("{folder} {}", snap.path)
        };
        parts.push(theme.fg(ThemeColor::StatusLinePath, &text));
    }

    if let Some(branch) = &snap.git_branch {
        let dirty = snap.git_unstaged > 0 || snap.git_staged > 0 || snap.git_untracked > 0;
        let branch_icon = theme.symbol("icon.branch");
        let mut git = if branch_icon.is_empty() {
            branch.clone()
        } else {
            format!("{branch_icon} {branch}")
        };
        if snap.git_unstaged > 0 {
            git.push(' ');
            git.push_str(&theme.fg(ThemeColor::StatusLineDirty, &format!("*{}", snap.git_unstaged)));
        }
        if snap.git_staged > 0 {
            git.push(' ');
            git.push_str(&theme.fg(ThemeColor::StatusLineStaged, &format!("+{}", snap.git_staged)));
        }
        if snap.git_untracked > 0 {
            git.push(' ');
            git.push_str(&theme.fg(
                ThemeColor::StatusLineUntracked,
                &format!("?{}", snap.git_untracked),
            ));
        }
        let color = if dirty {
            ThemeColor::StatusLineGitDirty
        } else {
            ThemeColor::StatusLineGitClean
        };
        parts.push(theme.fg(color, &git));
    }

    if let Some(pr) = &snap.pr {
        let icon = theme.symbol("icon.pr");
        let text = if icon.is_empty() {
            pr.clone()
        } else {
            format!("{icon} {pr}")
        };
        parts.push(theme.fg(ThemeColor::Accent, &text));
    }
    if let Some(pct) = snap.context_pct {
        parts.push(theme.fg(ThemeColor::StatusLineContext, &format!("{pct}%")));
    }
    if let Some(cost) = snap.cost_usd.filter(|c| *c > 0.0) {
        parts.push(theme.fg(ThemeColor::StatusLineCost, &format!("${cost:.2}")));
    }
    parts
}

fn right_parts(theme: &Theme, snap: &StatusSnapshot) -> Vec<String> {
    if snap.session_name.is_empty() {
        Vec::new()
    } else {
        vec![theme.fg(ThemeColor::Accent, &snap.session_name)]
    }
}

fn short_model(id: &str) -> String {
    id.rsplit('/').next().unwrap_or(id).to_owned()
}

fn abbreviate_path(raw: &str, max_len: usize) -> String {
    let mut s = raw.to_owned();
    if let Ok(home) = std::env::var("HOME")
        && let Some(rest) = s.strip_prefix(&home)
    {
        s = format!("~{rest}");
    }
    let count = s.chars().count();
    if count <= max_len {
        return s;
    }
    let chars: Vec<char> = s.chars().collect();
    let keep = max_len.saturating_sub(1);
    let start = chars.len().saturating_sub(keep);
    format!("…{}", chars[start..].iter().collect::<String>())
}

struct GitInfo {
    root: PathBuf,
    head_mtime: Option<SystemTime>,
    index_mtime: Option<SystemTime>,
    branch: Option<String>,
    unstaged: u32,
    staged: u32,
    untracked: u32,
}

fn git_cache() -> &'static Mutex<Option<GitInfo>> {
    static CACHE: OnceLock<Mutex<Option<GitInfo>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(None))
}

fn git_info(start: &Path) -> GitInfo {
    let empty = GitInfo {
        root: PathBuf::new(),
        head_mtime: None,
        index_mtime: None,
        branch: None,
        unstaged: 0,
        staged: 0,
        untracked: 0,
    };
    let Some((root, head_path, index_path)) = find_git(start) else {
        return empty;
    };
    let head_mtime = std::fs::metadata(&head_path).ok().and_then(|m| m.modified().ok());
    let index_mtime = std::fs::metadata(&index_path).ok().and_then(|m| m.modified().ok());
    if let Ok(guard) = git_cache().lock()
        && let Some(cached) = guard.as_ref()
        && cached.root == root
        && cached.head_mtime == head_mtime
        && cached.index_mtime == index_mtime
    {
        return GitInfo {
            root: cached.root.clone(),
            head_mtime,
            index_mtime,
            branch: cached.branch.clone(),
            unstaged: cached.unstaged,
            staged: cached.staged,
            untracked: cached.untracked,
        };
    }
    let branch = read_branch(&head_path);
    let (unstaged, staged, untracked) = git_porcelain_counts(&root);
    let fresh = GitInfo {
        root,
        head_mtime,
        index_mtime,
        branch,
        unstaged,
        staged,
        untracked,
    };
    if let Ok(mut guard) = git_cache().lock() {
        *guard = Some(GitInfo {
            root: fresh.root.clone(),
            head_mtime: fresh.head_mtime,
            index_mtime: fresh.index_mtime,
            branch: fresh.branch.clone(),
            unstaged: fresh.unstaged,
            staged: fresh.staged,
            untracked: fresh.untracked,
        });
    }
    fresh
}

fn find_git(start: &Path) -> Option<(PathBuf, PathBuf, PathBuf)> {
    let mut dir = start.to_path_buf();
    loop {
        let git = dir.join(".git");
        if git.is_dir() {
            return Some((dir, git.join("HEAD"), git.join("index")));
        }
        if git.is_file() {
            let text = std::fs::read_to_string(&git).ok()?;
            let gitdir = PathBuf::from(text.strip_prefix("gitdir:")?.trim());
            let gitdir = if gitdir.is_absolute() {
                gitdir
            } else {
                dir.join(gitdir)
            };
            return Some((dir, gitdir.join("HEAD"), gitdir.join("index")));
        }
        dir = dir.parent()?.to_path_buf();
    }
}

fn read_branch(head_path: &Path) -> Option<String> {
    let head = std::fs::read_to_string(head_path).ok()?;
    let head = head.trim();
    if let Some(branch) = head.strip_prefix("ref: refs/heads/") {
        return Some(branch.to_owned());
    }
    if head.len() >= 7 {
        return Some(head.chars().take(7).collect());
    }
    None
}

fn git_porcelain_counts(repo: &Path) -> (u32, u32, u32) {
    let out = std::process::Command::new("git")
        .args([
            "-C",
            &repo.to_string_lossy(),
            "status",
            "--porcelain=v1",
            "-unormal",
        ])
        .env("GIT_OPTIONAL_LOCKS", "1")
        .output();
    let Ok(out) = out else {
        return (0, 0, 0);
    };
    if !out.status.success() {
        return (0, 0, 0);
    }
    parse_porcelain(&String::from_utf8_lossy(&out.stdout))
}

fn parse_porcelain(text: &str) -> (u32, u32, u32) {
    let mut unstaged = 0u32;
    let mut staged = 0u32;
    let mut untracked = 0u32;
    for line in text.lines() {
        let bytes = line.as_bytes();
        if bytes.len() < 2 {
            continue;
        }
        let x = bytes[0] as char;
        let y = bytes[1] as char;
        if x == '?' && y == '?' {
            untracked = untracked.saturating_add(1);
            continue;
        }
        if x == '!' && y == '!' {
            continue;
        }
        if x != ' ' && x != '?' {
            staged = staged.saturating_add(1);
        }
        if y != ' ' && y != '?' {
            unstaged = unstaged.saturating_add(1);
        }
    }
    (unstaged, staged, untracked)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::{global, Theme};

    fn theme() -> std::sync::Arc<Theme> {
        global().init("titanium");
        global().current().expect("titanium")
    }

    fn snap() -> StatusSnapshot {
        StatusSnapshot {
            model: "glm-5.3-flash".into(),
            path: "~/proj/titi".into(),
            git_branch: Some("main".into()),
            session_name: "titi".into(),
            ..StatusSnapshot::default()
        }
    }

    #[test]
    fn default_order_pi_model_path_git_session() {
        let theme = theme();
        let line = render_status_bar(&theme, 120, &snap());
        let pi = theme.symbol("icon.pi");
        let model = theme.symbol("icon.model");
        let branch = theme.symbol("icon.branch");
        assert!(line.contains(pi), "pi: {line}");
        assert!(line.contains("glm-5.3-flash"), "model: {line}");
        assert!(line.contains("~/proj/titi"), "path: {line}");
        assert!(line.contains("main"), "git: {line}");
        assert!(line.contains("titi"), "session: {line}");
        let i_pi = line.find(pi).expect("pi");
        let i_model = line.find(model).expect("model icon");
        let i_path = line.find("~/proj/titi").expect("path");
        let i_git = line.find(branch).expect("branch icon");
        assert!(i_pi < i_model && i_model < i_path && i_path < i_git, "order: {line}");
        assert!(!line.contains("agent"), "mode hidden unless Plan/Loop: {line}");
    }

    #[test]
    fn hidden_segments_stay_hidden() {
        let theme = theme();
        let line = render_status_bar(&theme, 80, &snap());
        assert!(!line.contains('$'), "zero cost hidden: {line}");
        assert!(!line.contains('%'), "no context hidden: {line}");
    }

    #[test]
    fn dirty_flags_use_omp_markers() {
        let theme = theme();
        let mut s = snap();
        s.git_unstaged = 2;
        s.git_staged = 1;
        s.git_untracked = 3;
        let line = render_status_bar(&theme, 120, &s);
        assert!(line.contains("*2"), "{line}");
        assert!(line.contains("+1"), "{line}");
        assert!(line.contains("?3"), "{line}");
    }

    #[test]
    fn short_model_strips_provider_prefix() {
        let live = live_snapshot("opencode-go/glm-5.3-flash", "titi");
        assert_eq!(live.model, "glm-5.3-flash");
    }

    #[test]
    fn porcelain_counts_unstaged_staged_untracked() {
        let src = " M a.rs\nM  b.rs\nMM c.rs\n?? d.rs\n!! ignored\n";
        assert_eq!(parse_porcelain(src), (2, 2, 1));
    }
}
