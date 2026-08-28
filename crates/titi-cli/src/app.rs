//! TUI app composition for titi-cli: first frame + transcript accordion.
//!
//! Terminal-independent core — tests drive it with an in-memory render; the
//! binary wraps it with crossterm raw mode + alternate screen.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use titi_tui::markdown::Section;
use titi_tui::caps::MousePreset;
use titi_tui::selection::Selection;
use titi_tui::status::AgentState;
use titi_tui::theme::{global, Theme};
use titi_tui::transcript::{Alert, Entry, Transcript};

use crate::first_frame::{FirstFrame, SubmitOutcome};

/// Composite app: startup state machine + transcript.
pub struct App {
    first_frame: FirstFrame,
    transcript: Transcript,
    theme: Arc<Theme>,
    width: u16,
    selection: Option<Selection>,
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
