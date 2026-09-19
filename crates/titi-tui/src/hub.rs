//! Agent Hub roster overlay (`omp://agent-hub`).
//!
//! Compact-overlay port of the OMP table: Main is excluded, empty-state copy
//! is verbatim, and `j/k` / `t` / `Tab` / `r` / `x` / Enter / Esc match the
//! host table. No broker IPC — peers are injected by the application.

use std::sync::Arc;

use crate::component::Component;
use crate::theme::{Theme, ThemeColor};
use crate::width::{truncate_to_width, visible_width};

/// OMP `MAIN_AGENT_ID` — driving session, never a hub row.
pub const MAIN_AGENT_ID: &str = "Main";

/// OMP `AgentStatus`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentStatus {
    Running,
    Idle,
    Parked,
    Aborted,
}

impl AgentStatus {
    /// Lowercase wire name (`running` / `idle` / `parked` / `aborted`).
    pub fn as_str(self) -> &'static str {
        match self {
            AgentStatus::Running => "running",
            AgentStatus::Idle => "idle",
            AgentStatus::Parked => "parked",
            AgentStatus::Aborted => "aborted",
        }
    }

    fn order(self) -> u8 {
        match self {
            AgentStatus::Running => 0,
            AgentStatus::Idle => 1,
            AgentStatus::Parked => 2,
            AgentStatus::Aborted => 3,
        }
    }

    /// Theme symbol key for this status (OMP `statusGlyph`).
    pub fn symbol_key(self) -> &'static str {
        match self {
            AgentStatus::Running => "status.running",
            AgentStatus::Idle => "status.enabled",
            AgentStatus::Parked => "status.shadowed",
            AgentStatus::Aborted => "status.aborted",
        }
    }
}

/// OMP `AgentKind`. Advisors stay on the roster but are read-only.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentKind {
    Main,
    Sub,
    Advisor,
}

impl AgentKind {
    fn as_str(self) -> &'static str {
        match self {
            AgentKind::Main => "main",
            AgentKind::Sub => "sub",
            AgentKind::Advisor => "advisor",
        }
    }
}

/// One hub row. `id == "Main"` is stripped before paint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HubPeer {
    pub id: String,
    pub display_name: String,
    pub kind: AgentKind,
    pub parent_id: Option<String>,
    pub status: AgentStatus,
}

impl HubPeer {
    pub fn sub(id: impl Into<String>, status: AgentStatus) -> Self {
        let id = id.into();
        HubPeer {
            display_name: id.clone(),
            id,
            kind: AgentKind::Sub,
            parent_id: Some(MAIN_AGENT_ID.to_owned()),
            status,
        }
    }
}

/// Glyphs for status + cursor (unicode defaults, or theme-resolved).
#[derive(Debug, Clone)]
pub struct HubGlyphs {
    pub cursor: String,
    pub running: String,
    pub idle: String,
    pub parked: String,
    pub aborted: String,
}

impl HubGlyphs {
    /// OMP unicode preset (`status.*` + `nav.cursor`).
    pub fn unicode() -> Self {
        HubGlyphs {
            cursor: "❯".to_owned(),
            running: "⟳".to_owned(),
            idle: "●".to_owned(),
            parked: "○".to_owned(),
            aborted: "⏹".to_owned(),
        }
    }

    pub fn from_theme(theme: &Theme) -> Self {
        HubGlyphs {
            cursor: themed_symbol(theme, "nav.cursor", "❯"),
            running: theme.fg(
                ThemeColor::Accent,
                &plain_symbol(theme, "status.running", "⟳"),
            ),
            idle: theme.fg(
                ThemeColor::Success,
                &plain_symbol(theme, "status.enabled", "●"),
            ),
            parked: theme.fg(
                ThemeColor::Muted,
                &plain_symbol(theme, "status.shadowed", "○"),
            ),
            aborted: theme.fg(
                ThemeColor::Error,
                &plain_symbol(theme, "status.aborted", "⏹"),
            ),
        }
    }

    fn for_status(&self, status: AgentStatus) -> &str {
        match status {
            AgentStatus::Running => &self.running,
            AgentStatus::Idle => &self.idle,
            AgentStatus::Parked => &self.parked,
            AgentStatus::Aborted => &self.aborted,
        }
    }
}

fn plain_symbol(theme: &Theme, key: &str, fallback: &str) -> String {
    let g = theme.symbol(key);
    if g.is_empty() {
        fallback.to_owned()
    } else {
        g.to_owned()
    }
}

fn themed_symbol(theme: &Theme, key: &str, fallback: &str) -> String {
    plain_symbol(theme, key, fallback)
}

/// Drop `Main` and sort like the OMP first paint (`STATUS_ORDER` then id).
pub fn visible_peers(peers: impl IntoIterator<Item = HubPeer>) -> Vec<HubPeer> {
    let mut out: Vec<HubPeer> = peers
        .into_iter()
        .filter(|p| p.id != MAIN_AGENT_ID)
        .collect();
    out.sort_by(|a, b| {
        a.status
            .order()
            .cmp(&b.status.order())
            .then_with(|| a.id.cmp(&b.id))
    });
    out
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ViewMode {
    Roster,
    Tree,
}

/// Compact Agent Hub overlay.
pub struct HubRoster {
    peers: Vec<HubPeer>,
    selected: usize,
    view: ViewMode,
    details: bool,
    notice: Option<String>,
    max_visible: Option<usize>,
    closed: bool,
    cancelled: bool,
    pending: Option<HubCommand>,
    glyphs: HubGlyphs,
    theme: Option<Arc<Theme>>,
}

/// Hub action that should be forwarded to the engine without closing the overlay.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HubCommand {
    Revive(String),
    Stop(String),
}

impl HubRoster {
    pub fn new(peers: Vec<HubPeer>) -> Self {
        Self::with_glyphs(visible_peers(peers), HubGlyphs::unicode(), None)
    }

    pub fn with_theme(peers: Vec<HubPeer>, theme: Arc<Theme>) -> Self {
        let glyphs = HubGlyphs::from_theme(&theme);
        Self::with_glyphs(visible_peers(peers), glyphs, Some(theme))
    }

    fn with_glyphs(peers: Vec<HubPeer>, glyphs: HubGlyphs, theme: Option<Arc<Theme>>) -> Self {
        HubRoster {
            peers,
            selected: 0,
            view: ViewMode::Roster,
            details: false,
            notice: None,
            max_visible: None,
            closed: false,
            cancelled: false,
            pending: None,
            glyphs,
            theme,
        }
    }

    pub fn is_closed(&self) -> bool {
        self.closed
    }

    pub fn cancelled(&self) -> bool {
        self.cancelled
    }

    pub fn selected_id(&self) -> Option<&str> {
        self.peers.get(self.selected).map(|p| p.id.as_str())
    }

    pub fn into_selected(self) -> Option<String> {
        if self.cancelled {
            return None;
        }
        self.peers.get(self.selected).map(|p| p.id.clone())
    }

    /// Engine command requested by a hub key that does not close the overlay.
    pub fn pending_command(&self) -> Option<HubCommand> {
        self.pending.clone()
    }

    pub fn take_pending_command(&mut self) -> Option<HubCommand> {
        self.pending.take()
    }

    pub fn peers(&self) -> &[HubPeer] {
        &self.peers
    }

    pub fn notice(&self) -> Option<&str> {
        self.notice.as_deref()
    }

    pub fn set_max_visible(&mut self, rows: usize) {
        self.max_visible = Some(rows.max(1));
    }

    fn selected_peer(&self) -> Option<&HubPeer> {
        self.peers.get(self.selected)
    }

    fn selected_peer_mut(&mut self) -> Option<&mut HubPeer> {
        self.peers.get_mut(self.selected)
    }

    fn move_sel(&mut self, delta: isize) {
        if self.peers.is_empty() {
            return;
        }
        let n = self.peers.len() as isize;
        let next = (self.selected as isize + delta).clamp(0, n - 1) as usize;
        self.selected = next;
    }

    fn activate(&mut self) {
        if self.peers.get(self.selected).is_some() {
            self.closed = true;
            self.cancelled = false;
        }
    }

    fn cancel(&mut self) {
        self.closed = true;
        self.cancelled = true;
    }

    fn revive_selected(&mut self) {
        let Some(peer) = self.selected_peer() else {
            return;
        };
        if peer.kind == AgentKind::Advisor {
            self.notice = Some(format!(
                "\"{}\" is a read-only advisor transcript — nothing to revive.",
                peer.id
            ));
            return;
        }
        if peer.status != AgentStatus::Parked {
            self.notice = Some(format!(
                "Agent \"{}\" is {} — only parked agents can be revived.",
                peer.id,
                peer.status.as_str()
            ));
            return;
        }
        let id = peer.id.clone();
        if let Some(peer) = self.selected_peer_mut() {
            peer.status = AgentStatus::Idle;
        }
        self.pending = Some(HubCommand::Revive(id));
        self.notice = None;
    }

    fn kill_selected(&mut self) {
        let Some(peer) = self.selected_peer() else {
            return;
        };
        if peer.kind == AgentKind::Advisor {
            self.notice = Some(format!(
                "\"{}\" is a read-only advisor transcript — cannot be killed.",
                peer.id
            ));
            return;
        }
        let id = peer.id.clone();
        if let Some(peer) = self.selected_peer_mut() {
            peer.status = AgentStatus::Aborted;
        }
        self.pending = Some(HubCommand::Stop(id));
        self.notice = None;
    }

    fn depth_of(&self, id: &str) -> usize {
        if self.view != ViewMode::Tree {
            return 0;
        }
        let mut depth = 0;
        let mut current = self.peers.iter().find(|p| p.id == id);
        let mut guard = 0;
        while let Some(peer) = current {
            let Some(parent) = peer.parent_id.as_deref() else {
                break;
            };
            if parent == MAIN_AGENT_ID {
                break;
            }
            if !self.peers.iter().any(|p| p.id == parent) {
                break;
            }
            depth += 1;
            current = self.peers.iter().find(|p| p.id == parent);
            guard += 1;
            if guard > self.peers.len() {
                break;
            }
        }
        depth
    }

    fn windowed(&self) -> Vec<usize> {
        let n = self.peers.len();
        let Some(cap) = self.max_visible else {
            return (0..n).collect();
        };
        if n <= cap {
            return (0..n).collect();
        }
        let cap = cap.max(1);
        let mut start = self.selected.saturating_sub(cap / 2);
        if start + cap > n {
            start = n - cap;
        }
        (start..start + cap).collect()
    }

    fn paint(&self, text: &str, color: Option<ThemeColor>) -> String {
        match (&self.theme, color) {
            (Some(theme), Some(c)) => theme.fg(c, text),
            (Some(theme), None) => theme.bold(text),
            (None, _) => text.to_owned(),
        }
    }

    fn footer(&self, inner_w: usize) -> String {
        let next_view = match self.view {
            ViewMode::Roster => "by parent",
            ViewMode::Tree => "flat",
        };
        let raw = if self.details && !self.peers.is_empty() {
            format!("Tab:roster  PgUp/PgDn:scroll  Enter:open  t:{next_view}  Esc:roster")
        } else {
            format!("j/k:select  Enter:open  t:{next_view}  Tab:details  r/x:manage  Esc:close")
        };
        let shown = truncate_to_width(&raw, inner_w);
        match &self.theme {
            Some(theme) => theme.fg(ThemeColor::Dim, &shown),
            None => shown,
        }
    }

    fn empty_lines(&self, inner_w: usize) -> Vec<String> {
        let glyph = self.glyphs.for_status(AgentStatus::Parked);
        let title = self.paint("No agents in this session", None);
        let line0 = truncate_to_width(&format!("{glyph} {title}"), inner_w);
        let rest = [
            "Finished, parked, and killed subagents remain with the session that created them.",
            "Resume that session with omp-dev --continue, or spawn a task here.",
        ];
        let mut lines = vec![line0];
        for line in rest {
            let painted = match &self.theme {
                Some(theme) => theme.fg(ThemeColor::Dim, line),
                None => line.to_owned(),
            };
            lines.push(truncate_to_width(&painted, inner_w));
        }
        lines
    }

    fn row_label(&self, peer: &HubPeer, selected: bool) -> String {
        let cursor = if selected {
            match &self.theme {
                Some(theme) => theme.fg(ThemeColor::Accent, &self.glyphs.cursor),
                None => self.glyphs.cursor.clone(),
            }
        } else {
            " ".to_owned()
        };
        let glyph = self.glyphs.for_status(peer.status);
        let indent = "  ".repeat(self.depth_of(&peer.id));
        let id = sanitize_display(&peer.id);
        let styled_id = match (&self.theme, selected) {
            (Some(theme), true) => theme.bold(&theme.fg(ThemeColor::Accent, &id)),
            (Some(theme), false) => theme.bold(&id),
            (None, _) => id,
        };
        let mut fields = vec![format!("{cursor} {glyph} {indent}{styled_id}")];
        if !peer.display_name.is_empty() && peer.display_name != peer.id {
            let name = sanitize_display(&peer.display_name);
            fields.push(match &self.theme {
                Some(theme) => theme.fg(ThemeColor::Dim, &name),
                None => name,
            });
        }
        if self.view == ViewMode::Roster {
            if let Some(parent) = peer.parent_id.as_deref() {
                if parent != MAIN_AGENT_ID {
                    let tag = format!("↳ {}", sanitize_display(parent));
                    fields.push(match &self.theme {
                        Some(theme) => theme.fg(ThemeColor::Dim, &tag),
                        None => tag,
                    });
                }
            }
        }
        if peer.kind == AgentKind::Advisor {
            let tag = "read-only";
            fields.push(match &self.theme {
                Some(theme) => theme.fg(ThemeColor::Warning, tag),
                None => tag.to_owned(),
            });
        }
        fields.join("  ")
    }

    fn detail_lines(&self, inner_w: usize) -> Vec<String> {
        let Some(peer) = self.selected_peer() else {
            return Vec::new();
        };
        let parent = peer.parent_id.as_deref().unwrap_or(MAIN_AGENT_ID);
        let lines = [
            format!("id: {}", sanitize_display(&peer.id)),
            format!("status: {}", peer.status.as_str()),
            format!("parent: {}", sanitize_display(parent)),
            format!("kind: {}", peer.kind.as_str()),
        ];
        lines
            .into_iter()
            .map(|line| match &self.theme {
                Some(theme) => truncate_to_width(&theme.fg(ThemeColor::Dim, &line), inner_w),
                None => truncate_to_width(&line, inner_w),
            })
            .collect()
    }
}

fn sanitize_display(text: &str) -> String {
    text.chars()
        .map(|c| match c {
            '\t' | '\n' | '\r' => ' ',
            c if c.is_control() => ' ',
            other => other,
        })
        .collect()
}

fn box_top_title(inner_w: usize, title: &str) -> String {
    if title.is_empty() {
        return format!("╭{}╮", "─".repeat(inner_w));
    }
    let shown = truncate_to_width(&format!(" {title} "), inner_w.saturating_sub(1));
    let fill = inner_w
        .saturating_sub(1)
        .saturating_sub(visible_width(&shown));
    format!("╭─{shown}{}╮", "─".repeat(fill))
}

fn box_bot(inner_w: usize) -> String {
    format!("╰{}╯", "─".repeat(inner_w))
}

fn box_row(inner_w: usize, content: &str) -> String {
    let truncated = truncate_to_width(content, inner_w);
    let pad = inner_w.saturating_sub(visible_width(&truncated));
    format!("│ {truncated}{} │", " ".repeat(pad))
}

impl Component for HubRoster {
    fn render(&mut self, width: u16) -> Vec<String> {
        if self.closed {
            return Vec::new();
        }
        let w = width as usize;
        if w < 8 {
            if self.peers.is_empty() {
                return vec!["No agents in this session".to_owned()];
            }
            return self
                .windowed()
                .into_iter()
                .filter_map(|i| {
                    let peer = self.peers.get(i)?;
                    Some(self.row_label(peer, i == self.selected))
                })
                .collect();
        }
        let inner_w = w.saturating_sub(4).max(6);
        let title = match (self.details, self.selected_peer()) {
            (true, Some(peer)) => format!("Agent Hub · {}", peer.id),
            _ => "Agent Hub".to_owned(),
        };
        let mut body: Vec<String> = Vec::new();
        if self.details && !self.peers.is_empty() {
            body.extend(self.detail_lines(inner_w));
        } else if self.peers.is_empty() {
            body.extend(self.empty_lines(inner_w));
        } else {
            for i in self.windowed() {
                let Some(peer) = self.peers.get(i) else {
                    continue;
                };
                body.push(self.row_label(peer, i == self.selected));
            }
        }
        if let Some(notice) = &self.notice {
            let painted = match &self.theme {
                Some(theme) => theme.fg(ThemeColor::Error, notice),
                None => notice.clone(),
            };
            body.push(truncate_to_width(&painted, inner_w));
        }
        body.push(self.footer(inner_w));

        let mut rows = Vec::with_capacity(body.len() + 2);
        rows.push(box_top_title(inner_w, &title));
        for line in body {
            rows.push(box_row(inner_w, &line));
        }
        rows.push(box_bot(inner_w));
        rows
    }

    fn handle_input(&mut self, data: &str) {
        if self.closed {
            return;
        }
        match data {
            "\x1b" | "\x1b\x1b" => {
                if self.details && !self.peers.is_empty() {
                    self.details = false;
                } else {
                    self.cancel();
                }
            }
            "\t" => {
                if !self.peers.is_empty() {
                    self.details = !self.details;
                }
            }
            "\x1b[D" => {
                if self.details && !self.peers.is_empty() {
                    self.details = false;
                }
            }
            "t" => {
                self.view = match self.view {
                    ViewMode::Roster => ViewMode::Tree,
                    ViewMode::Tree => ViewMode::Roster,
                };
            }
            "j" | "\x1b[B" => self.move_sel(1),
            "k" | "\x1b[A" => self.move_sel(-1),
            "\r" | "\n" => self.activate(),
            "r" => self.revive_selected(),
            "x" => self.kill_selected(),
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn render(hub: &mut HubRoster) -> String {
        hub.render(80).join("\n")
    }

    #[test]
    fn empty_state_copy() {
        let mut hub = HubRoster::new(Vec::new());
        let out = hub.render(120).join("\n");
        assert!(out.contains("Agent Hub"), "{out}");
        assert!(out.contains("○"), "{out}");
        assert!(out.contains("No agents in this session"), "{out}");
        assert!(
            out.contains(
                "Finished, parked, and killed subagents remain with the session that created them."
            ),
            "{out}"
        );
        assert!(
            out.contains("Resume that session with omp-dev --continue, or spawn a task here."),
            "{out}"
        );
        assert!(out.contains("j/k:select"), "{out}");
        assert!(out.contains("╰"), "{out}");
        assert!(!out.contains("▶"), "{out}");
    }

    #[test]
    fn main_is_excluded() {
        let mut hub = HubRoster::new(vec![
            HubPeer {
                id: MAIN_AGENT_ID.to_owned(),
                display_name: "Main".into(),
                kind: AgentKind::Main,
                parent_id: None,
                status: AgentStatus::Idle,
            },
            HubPeer::sub("Worker", AgentStatus::Running),
        ]);
        let out = render(&mut hub);
        assert!(out.contains("Worker"), "{out}");
        assert!(!out.contains(" Main"), "{out}");
        assert_eq!(hub.selected_id(), Some("Worker"));
    }

    #[test]
    fn jk_moves_and_enter_selects() {
        let mut hub = HubRoster::new(vec![
            HubPeer::sub("Alpha", AgentStatus::Running),
            HubPeer::sub("Beta", AgentStatus::Idle),
        ]);
        hub.handle_input("j");
        assert_eq!(hub.selected_id(), Some("Beta"));
        hub.handle_input("k");
        assert_eq!(hub.selected_id(), Some("Alpha"));
        hub.handle_input("\r");
        assert!(hub.is_closed());
        assert!(!hub.cancelled());
        assert_eq!(hub.into_selected().as_deref(), Some("Alpha"));
    }

    #[test]
    fn empty_enter_stays_open() {
        let mut hub = HubRoster::new(Vec::new());
        hub.handle_input("\r");
        assert!(!hub.is_closed());
    }

    #[test]
    fn revive_and_kill_notices() {
        let mut hub = HubRoster::new(vec![HubPeer::sub("W", AgentStatus::Idle)]);
        hub.handle_input("r");
        assert_eq!(
            hub.notice(),
            Some("Agent \"W\" is idle — only parked agents can be revived.")
        );
        hub.handle_input("x");
        assert_eq!(hub.peers()[0].status, AgentStatus::Aborted);
        assert!(hub.notice().is_none());
        assert_eq!(
            hub.take_pending_command(),
            Some(HubCommand::Stop("W".into()))
        );
    }

    #[test]
    fn revive_parked_to_idle() {
        let mut hub = HubRoster::new(vec![HubPeer::sub("Parked", AgentStatus::Parked)]);
        hub.handle_input("r");
        assert_eq!(hub.peers()[0].status, AgentStatus::Idle);
        assert!(hub.notice().is_none());
        assert_eq!(
            hub.take_pending_command(),
            Some(HubCommand::Revive("Parked".into()))
        );
    }

    #[test]
    fn advisor_is_read_only() {
        let mut hub = HubRoster::new(vec![HubPeer {
            id: "Review".into(),
            display_name: "Review".into(),
            kind: AgentKind::Advisor,
            parent_id: Some(MAIN_AGENT_ID.to_owned()),
            status: AgentStatus::Parked,
        }]);
        hub.handle_input("r");
        assert!(
            hub.notice()
                .is_some_and(|n| n.contains("read-only advisor transcript — nothing to revive")),
            "{:?}",
            hub.notice()
        );
        hub.handle_input("x");
        assert!(
            hub.notice().is_some_and(|n| n.contains("cannot be killed")),
            "{:?}",
            hub.notice()
        );
        assert_eq!(hub.peers()[0].status, AgentStatus::Parked);
    }

    #[test]
    fn tree_indent_and_tab_details() {
        let mut hub = HubRoster::new(vec![
            HubPeer::sub("Parent", AgentStatus::Idle),
            HubPeer {
                id: "Child".into(),
                display_name: "Child".into(),
                kind: AgentKind::Sub,
                parent_id: Some("Parent".into()),
                status: AgentStatus::Running,
            },
        ]);
        hub.handle_input("t");
        let out = render(&mut hub);
        assert!(out.contains("t:flat"), "{out}");
        assert!(out.contains("Child"), "{out}");
        hub.handle_input("\t");
        let details = render(&mut hub);
        assert!(details.contains("Agent Hub · Child"), "{details}");
        assert!(details.contains("status: running"), "{details}");
        assert!(details.contains("Tab:roster"), "{details}");
        hub.handle_input("\x1b");
        assert!(!hub.is_closed());
        hub.handle_input("\x1b");
        assert!(hub.cancelled());
    }

    #[test]
    fn status_sort_running_first() {
        let peers = visible_peers(vec![
            HubPeer::sub("z-idle", AgentStatus::Idle),
            HubPeer::sub("a-run", AgentStatus::Running),
        ]);
        assert_eq!(peers[0].id, "a-run");
        assert_eq!(peers[1].id, "z-idle");
    }
}
