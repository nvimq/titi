//! TUI app composition for titi-cli: first frame + transcript accordion.
//!
//! Terminal-independent core — tests drive it with an in-memory render; the
//! binary wraps it with crossterm raw mode + alternate screen.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use titi_tui::component::Component as _;
use titi_tui::composer::{
    Composer, PasteResult, QueueMode, Queued, PASTE_INLINE_MAX_LINES,
};
use titi_tui::markdown::Section;
use titi_tui::caps::MousePreset;
use titi_tui::overlay::{composite_rows, Anchor};
use titi_tui::panels::{
    ApprovalPanel, CompletionPanel, SelectionPanel, SessionAction, SessionSwitcher,
};
use titi_tui::selection::Selection;
use titi_tui::status::AgentState;
use titi_tui::theme::{global, Theme};
use titi_tui::transcript::{Alert, Entry, Transcript};
use titi_tui::slash::{Route, SlashRegistry};

use crate::first_frame::{FirstFrame, SubmitOutcome};

/// Composite app: startup state machine + transcript.
pub struct App {
    first_frame: FirstFrame,
    transcript: Transcript,
    theme: Arc<Theme>,
    width: u16,
    selection: Option<Selection>,
    /// The modal overlay panel currently shown, if any.
    overlay: Option<ActiveOverlay>,
    /// Paste collapse + attachment numbering state.
    composer: Composer,
    /// Session id awaiting close approval (`SessionAction::Close`).
    pending_close: Option<String>,
    /// Slash-command registry (builtin names reserved, then file
    /// expansion, then passthrough to the LLM).
    slash: SlashRegistry,
    /// Floating non-modal slash autocomplete.
    completion: CompletionPanel,
    /// A queued message pulled back into the editor via Alt+Up; shown
    /// highlighted until Esc clears the highlight (does not re-queue).
    highlighted: Option<Queued>,
}

impl App {
    /// Create the app.  `ready` is flipped by the provider-init thread.
    pub fn new(_ready: Arc<AtomicBool>, banner: Vec<String>, theme: Arc<Theme>) -> Self {
        App {
            first_frame: FirstFrame::new(banner),
            transcript: Transcript::new(),
            theme,
            width: 80,
            selection: None,
            overlay: None,
            composer: Composer::new(),
            pending_close: None,
            slash: Self::default_slash_registry(),
            completion: CompletionPanel::new(),
            highlighted: None,
        }
    }

    /// The built-in slash commands (names reserved — see the Slash DoD).
    ///
    /// `help` lists them, `details` toggles transcript sections, `mouse`
    /// switches the tracking preset, `model` opens the picker, `sessions`
    /// opens the switcher.
    fn default_slash_registry() -> SlashRegistry {
        let mut registry = SlashRegistry::new();
        registry.register_builtin("help", "Show available commands");
        registry.register_builtin("model", "Switch the active model");
        registry.register_builtin("sessions", "Open the session switcher");
        registry.register_builtin("mouse", "Set mouse tracking: off|wheel|buttons|all");
        registry.register_builtin("details", "Toggle transcript section visibility");
        registry
    }

    /// Whether a modal overlay panel is currently shown.
    pub fn overlay_open(&self) -> bool {
        self.overlay.is_some()
    }
    /// Route a `/`-prefixed input line: builtin (reserved name), expanded
    /// file command, or passthrough to the LLM.  Never matches a plain
    /// prompt (no leading `/`).
    pub fn route_slash(&self, input: &str) -> Route {
        self.slash.route(input)
    }

    /// Refresh the floating completion panel for the current input:
    /// suggestions only while typing a bare `/name` (no arguments yet);
    /// anything else hides it.
    pub fn slash_completions(&mut self, input: &str) {
        if !input.starts_with('/') || input.contains(' ') {
            self.completion.hide();
            return;
        }
        self.completion.refresh(self.slash.complete(input));
    }

    /// Whether the completion panel is currently shown.
    pub fn completion_visible(&self) -> bool {
        self.completion.is_visible()
    }

    /// Move the completion highlight (Up/Down; wraps).
    pub fn completion_move(&mut self, up: bool) {
        if up {
            self.completion.move_up();
        } else {
            self.completion.move_down();
        }
    }

    /// Accept the highlighted completion: return its command name and hide
    /// the panel.  `None` when the panel is hidden/empty.
    pub fn completion_accept(&mut self) -> Option<String> {
        let name = self.completion.selected_name()?.to_owned();
        self.completion.hide();
        Some(name)
    }

    /// Hide the completion panel without touching the input buffer.
    pub fn completion_hide(&mut self) {
        self.completion.hide();
    }

    /// Show the model picker over the given model ids.
    pub fn open_model_picker(&mut self, models: Vec<String>) {
        let labels = models.to_vec();
        self.overlay = Some(ActiveOverlay::ModelPicker(SelectionPanel::new(
            "Model", models, labels,
        )));
    }

    /// Show the session switcher: the live session first, then the ids
    /// stored under `<agent_dir>/sessions`.
    pub fn open_session_switcher(&mut self) {
        let mut titles = vec!["current".to_owned()];
        titles.extend(list_sessions());
        self.open_session_switcher_over(titles);
    }

    /// Show the session switcher over explicit titles (tests, embedded
    /// session sources).
    pub fn open_session_switcher_over(&mut self, titles: Vec<String>) {
        self.overlay = Some(ActiveOverlay::SessionSwitcher(SessionSwitcher::new(titles)));
    }

    /// Show an approval prompt; `approved` decides the pending action.
    pub fn open_approval(&mut self, prompt: &str) {
        self.overlay = Some(ActiveOverlay::Approval(ApprovalPanel::prompt(prompt)));
    }

    /// Queue a session close behind an approval prompt.  Esc / No / Cancel
    /// never deletes ([`App::confirm_pending_close`] runs only on Yes).
    pub fn request_session_close(&mut self, id: &str) {
        self.pending_close = Some(id.to_owned());
        self.open_approval(&format!("Close session {id}?"));
    }

    /// The session id awaiting close approval, if any.
    pub fn take_pending_close(&mut self) -> Option<String> {
        self.pending_close.take()
    }

    /// Route a decoded key to the open overlay.  Returns the panel's
    /// outcome once it closes; `None` while it stays open or no overlay is
    /// shown.  A switcher `Close` never returns directly — it opens the
    /// approval prompt ([`App::request_session_close`]), and only an
    /// explicit Yes reaches the deletion.
    pub fn overlay_input(&mut self, data: &str) -> Option<OverlayOutcome> {
        let mut active = self.overlay.take()?;
        active.handle_input(data);
        if !active.is_closed() {
            self.overlay = Some(active);
            return None;
        }
        // Intercept the switcher's Close: resolve the session id and gate
        // the deletion behind the approval panel.
        if let ActiveOverlay::SessionSwitcher(s) = &active
            && let Some(SessionAction::Close(i)) = s.action()
            && let Some(id) = s.titles().get(*i).cloned()
        {
            self.request_session_close(&id);
            return None;
        }
        active.outcome()
    }

    /// Append a bracketed paste to the input buffer.  Multi-line pastes are
    /// inserted as one block (never executed line-by-line); pastes longer
    /// than [`PASTE_INLINE_MAX_LINES`] collapse to an inline preview; a
    /// single image path becomes an `[Image #N]` attachment marker.
    pub fn paste(&mut self, text: &str) -> String {
        match self.composer.collapse_paste(text, PASTE_INLINE_MAX_LINES) {
            PasteResult::Text(text) => text,
            PasteResult::Collapsed {
                preview,
                omitted_lines,
            } => format!("{preview}\n… (+{omitted_lines} lines)"),
            PasteResult::Attachment { marker, .. } => marker,
        }
    }

    /// Set the terminal width (resize).
    pub fn resize(&mut self, width: u16) {
        self.width = width;
    }

    /// Current width.
    pub fn width(&self) -> u16 {
        self.width
    }

    /// Mouse press: anchor a drag-select at (x, y).
    pub fn mouse_press(&mut self, x: u16, y: u16) {
        self.selection = Some(Selection::anchor(x, y));
    }

    /// Mouse drag: move the selection cursor.
    pub fn mouse_drag(&mut self, x: u16, y: u16) {
        if let Some(sel) = &mut self.selection {
            sel.drag(x, y);
        }
    }

    /// Mouse release: commit the selection.
    pub fn mouse_release(&mut self) {
        if let Some(sel) = &mut self.selection {
            sel.release();
        }
    }

    /// Clear the active selection.
    pub fn clear_selection(&mut self) {
        self.selection = None;
    }

    /// The active selection, if any.
    pub fn selection(&self) -> Option<Selection> {
        self.selection
    }

    /// Agent state.
    pub fn state(&self) -> AgentState {
        self.first_frame.state()
    }

    /// Provider readiness.
    pub fn is_ready(&self) -> bool {
        self.first_frame.is_ready()
    }

    /// Queued prompts count.
    pub fn queue_len(&self) -> usize {
        self.first_frame.queue_len()
    }

    /// Apply a `/details` directive to the transcript.
    pub fn details(&mut self, directive: &str) -> bool {
        let changed = self.transcript.details(directive);
        if changed && self.transcript.all_hidden() {
            // Floating-alert backstop: all sections hidden — surface a
            // notice instead of a silent transcript.
            self.transcript.set_alert(Alert {
                text: "all sections hidden — use /details to show a section".into(),
            });
        } else if changed {
            self.transcript.clear_alert();
        }
        changed
    }

    /// All transcript sections hidden.
    pub fn all_hidden(&self) -> bool {
        self.transcript.all_hidden()
    }

    /// Append a transcript entry (thinking/tools/subagents/activity).
    pub fn push_transcript(&mut self, section: Section, text: impl Into<String>) {
        self.transcript.push(Entry::new(section, text));
    }

    /// Set the floating alert directly.
    pub fn set_alert(&mut self, text: impl Into<String>) {
        self.transcript.set_alert(Alert { text: text.into() });
    }

    /// Submit a prompt; queued while starting, delivered when ready.
    pub fn submit(&mut self, prompt: String) -> SubmitOutcome {
        self.first_frame.submit(prompt)
    }

    /// Flush queued prompts after provider readiness; returns them.
    pub fn flush_queued(&mut self, ready: &AtomicBool) -> Vec<String> {
        if ready.load(Ordering::SeqCst) && !self.is_ready() {
            self.first_frame.provider_ready()
        } else {
            Vec::new()
        }
    }
    /// Queue a message for the stream (Steer / FollowUp).
    pub fn push_queued(&mut self, text: impl Into<String>, mode: QueueMode) {
        self.composer.push_queue(text.into(), mode);
    }

    /// Number of messages waiting in the stream queue.
    pub fn stream_queue_len(&self) -> usize {
        self.composer.queue_len()
    }

    /// Pull the last queued message back into the editor (Alt+Up).  The
    /// returned text is marked highlighted — Esc clears the highlight
    /// without re-queueing.  `None` when the queue is empty.
    pub fn pull_last_queued(&mut self) -> Option<String> {
        let queued = self.composer.dequeue_last()?;
        self.highlighted = Some(queued.clone());
        Some(queued.text)
    }

    /// Whether a queued message is currently highlighted in the editor.
    pub fn queue_highlighted(&self) -> bool {
        self.highlighted.is_some()
    }

    /// Clear the queued-message highlight without deleting the text (Esc).
    pub fn clear_highlight(&mut self) {
        self.highlighted = None;
    }

    /// Render the full frame: banner, transcript accordion, status line.
    pub fn render(&mut self) -> Vec<String> {
        let mut rows = Vec::new();
        // First frame = banner + status line (painted before provider ready).
        let (first, _) = self.first_frame.first_frame(self.width);
        rows.extend(first);
        rows.push(String::new());
        rows.extend(self.transcript.render(self.width, &self.theme));
        rows.push(self.first_frame.status_line(self.width));
        // Selection coordinates are absolute screen rows; apply the
        // selectedBg overlay over the whole rendered frame.
        if let Some(sel) = &self.selection {
            rows = sel.apply_background(&rows, &self.theme);
        }
        // Modal overlay composites last — panels sit on top of the frame
        // and of any selection background.
        let w = self.width;
        let overlay_rows = match &mut self.overlay {
            Some(active) => active.render(w),
            None => Vec::new(),
        };
        if !overlay_rows.is_empty() {
            rows = composite_rows(&rows, &overlay_rows, w, Anchor::BottomCenter);
        }
        // Floating slash autocomplete sits above the overlay — it is
        // non-modal, so it renders even while an overlay panel is shown.
        let completion_rows = self.completion.render(w);
        if !completion_rows.is_empty() {
            rows = composite_rows(&rows, &completion_rows, w, Anchor::TopCenter);
        }
        rows
    }

    /// Time-to-first-frame (from construction to first render).
    pub fn time_to_first_frame(&self) -> Duration {
        self.first_frame.frame_elapsed()
    }
}

/// The config key that stores the mouse-tracking preset.
pub const MOUSE_TRACKING_KEY: &str = "display.mouse_tracking";

/// Load the persisted mouse preset from the titi config.
///
/// `agent_dir` is the settings root (see [`titi_config::agent_dir`]).
/// Returns `None` when the key is absent or unparsable (caller falls back to
/// its own default).
pub fn load_mouse_preset_from(agent_dir: &std::path::Path) -> Option<MousePreset> {
    use titi_config::settings::Settings;
    let settings = Settings::load(
        agent_dir,
        &std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from(".")),
        &[],
    )
    .ok()?;
    let value = settings.get(MOUSE_TRACKING_KEY)?;
    let name = match value {
        serde_json::Value::String(s) => s,
        _ => return None,
    };
    MousePreset::parse(&name)
}

/// Load the persisted mouse preset using the real agent directory.
pub fn load_mouse_preset() -> Option<MousePreset> {
    load_mouse_preset_from(&titi_config::agent_dir())
}

/// Persist the mouse preset to the titi config (`display.mouse_tracking`).
pub fn save_mouse_preset_to(agent_dir: &std::path::Path, preset: MousePreset) -> Result<(), String> {
    use titi_config::settings::Settings;
    let mut settings = Settings::load(
        agent_dir,
        &std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from(".")),
        &[],
    )
    .map_err(|e| format!("{e}"))?;
    settings
        .set(MOUSE_TRACKING_KEY, serde_json::json!(preset.name()))
        .map_err(|e| format!("{e}"))
}

/// Persist the mouse preset using the real agent directory.
pub fn save_mouse_preset(preset: MousePreset) -> Result<(), String> {
    save_mouse_preset_to(&titi_config::agent_dir(), preset)
}

/// Load the process-wide default theme (built-in `dark`), falling back to a
/// minimal theme if the loader fails.
pub fn default_theme() -> Result<Arc<Theme>, String> {
    let name = global().init("dark");
    match global().current() {
        Some(theme) => Ok(theme),
        None => Theme::new(
            name,
            std::collections::HashMap::new(),
            std::collections::HashMap::new(),
            titi_tui::theme::ColorMode::Truecolor,
            titi_tui::theme::SymbolPreset::Unicode,
            std::collections::HashMap::new(),
            None,
            None,
        )
        .map(Arc::new),
    }
}

/// The modal overlay panel currently shown, kept typed so the application
/// can extract its result when it closes (contract:
/// `docs/research/agent-ux/README.md` — panels implement Overlay; Esc is
/// always cancel-without-delete).
pub enum ActiveOverlay {
    ModelPicker(SelectionPanel<String>),
    SessionSwitcher(SessionSwitcher),
    Approval(ApprovalPanel),
}

impl ActiveOverlay {
    fn render(&mut self, width: u16) -> Vec<String> {
        match self {
            ActiveOverlay::ModelPicker(p) => p.render(width),
            ActiveOverlay::SessionSwitcher(s) => s.render(width),
            ActiveOverlay::Approval(a) => a.render(width),
        }
    }

    fn handle_input(&mut self, data: &str) {
        match self {
            ActiveOverlay::ModelPicker(p) => p.handle_input(data),
            ActiveOverlay::SessionSwitcher(s) => s.handle_input(data),
            ActiveOverlay::Approval(a) => a.handle_input(data),
        }
    }

    fn is_closed(&self) -> bool {
        match self {
            ActiveOverlay::ModelPicker(p) => p.is_closed(),
            ActiveOverlay::SessionSwitcher(s) => s.is_closed(),
            ActiveOverlay::Approval(a) => a.is_closed(),
        }
    }

    /// Extract the outcome of a closed panel.
    fn outcome(self) -> Option<OverlayOutcome> {
        match self {
            ActiveOverlay::ModelPicker(p) => p
                .into_result()
                .and_then(|r| r.selected)
                .map(OverlayOutcome::ModelSelected),
            ActiveOverlay::SessionSwitcher(s) => {
                let titles = s.titles().to_vec();
                match s.into_action() {
                    Some(SessionAction::Switch(i)) => titles
                        .get(i)
                        .cloned()
                        .map(OverlayOutcome::SessionSwitched),
                    Some(SessionAction::New) => Some(OverlayOutcome::SessionNew),
                    Some(SessionAction::Cancel) => Some(OverlayOutcome::SessionCancelled),
                    // Refresh keeps the panel open; Close is intercepted in
                    // App::overlay_input — neither reaches here.
                    Some(SessionAction::Refresh) | Some(SessionAction::Close(_)) => None,
                    None => None,
                }
            }
            ActiveOverlay::Approval(a) => a.into_result().map(|r| {
                OverlayOutcome::Approval(!r.cancelled && r.selected == Some("Yes"))
            }),
        }
    }
}

/// What a closed overlay panel decided.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OverlayOutcome {
    /// Model picker: the chosen model id.
    ModelSelected(String),
    /// Session switcher, Enter: the session to switch to.
    SessionSwitched(String),
    /// Session switcher, Ctrl+N: create a new session.
    SessionNew,
    /// Session switcher, Esc: cancelled — nothing deleted.
    SessionCancelled,
    /// Approval panel: `true` only for an explicit Yes; Esc/No/Cancel are
    /// all cancel-without-delete.
    Approval(bool),
}

/// Models offered by the picker — the fallback chains from
/// `docs/research/STATE.md`.
pub fn model_choices() -> Vec<String> {
    [
        "opencode-go/glm-5.3-flash",
        "clinepass/glm-5.3",
        "opencode-go/deepseek-v4-flash",
        "clinepass/deepseek-v4-flash",
        "bai/glm-5.3-flash",
        "bai/qwen3.8-flash",
        "clinepass/deepseek-v4-pro",
        "qwen3.8-max",
    ]
    .into_iter()
    .map(String::from)
    .collect()
}

/// List session ids stored under `<agent_dir>/sessions` (newest first).
pub fn list_sessions_from(agent_dir: &std::path::Path) -> Vec<String> {
    let dir = agent_dir.join("sessions");
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut ids: Vec<(std::time::SystemTime, String)> = entries
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "jsonl"))
        .filter_map(|e| {
            let modified = e.metadata().ok()?.modified().ok()?;
            Some((modified, e.path().file_stem()?.to_string_lossy().into_owned()))
        })
        .collect();
    ids.sort_by_key(|a| std::cmp::Reverse(a.0));
    ids.into_iter().map(|(_, id)| id).collect()
}

/// [`list_sessions_from`] against the real agent directory.
pub fn list_sessions() -> Vec<String> {
    list_sessions_from(&titi_config::agent_dir())
}

/// Delete a session's JSONL file.  Callers must gate this behind an
/// approval prompt — Esc never reaches here.
pub fn delete_session_from(agent_dir: &std::path::Path, id: &str) -> Result<(), String> {
    let path = agent_dir.join("sessions").join(format!("{id}.jsonl"));
    std::fs::remove_file(&path).map_err(|e| format!("{e}"))
}

/// [`delete_session_from`] against the real agent directory.
pub fn delete_session(id: &str) -> Result<(), String> {
    delete_session_from(&titi_config::agent_dir(), id)
}
