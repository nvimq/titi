//! First-frame — banner + status line before provider ready, input queued
//! during init and delivered after ready, time-to-first-frame measurement.
//!
//! Contract: `docs/research/agent-ux/README.md` (Definition of Done, first
//! item).  The core is terminal-independent so the integration test drives
//! it with a mock provider without a TTY.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use titi_tui::status::{AgentState, StatusEvent, StatusLine};

/// Result of submitting a prompt while the provider is still starting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubmitOutcome {
    /// Provider not ready yet — prompt held in the queue.
    Queued,
    /// Provider ready — prompt handed to the caller immediately.
    Delivered,
}

/// First-frame state: banner rows, status line, input queue, readiness.
pub struct FirstFrame {
    banner: Vec<String>,
    status: StatusLine,
    queue: VecDeque<String>,
    ready: bool,
    delivered: usize,
    started_at: Instant,
}

impl FirstFrame {
    /// Create in the `Starting` state.  `started_at` anchors the
    /// time-to-first-frame measurement.
    pub fn new(banner: Vec<String>) -> Self {
        FirstFrame {
            banner,
            status: StatusLine::new(),
            queue: VecDeque::new(),
            ready: false,
            delivered: 0,
            started_at: Instant::now(),
        }
    }

    /// Render the very first frame: banner rows followed by the status line.
    ///
    /// Returns the frame rows and the elapsed time since construction — the
    /// `time-to-first-frame` measurement.
    pub fn first_frame(&mut self, width: u16) -> (Vec<String>, Duration) {
        let elapsed = self.started_at.elapsed();
        let mut rows = self.banner.clone();
        rows.push(self.status.render(width));
        (rows, elapsed)
    }

    /// Banner rows (no status). Status is the box-composer top border.
    pub fn banner(&self) -> &[String] {
        &self.banner
    }

    /// Elapsed since construction — the time-to-first-frame measurement.
    pub fn frame_elapsed(&self) -> Duration {
        self.started_at.elapsed()
    }

    /// Current agent state.
    pub fn state(&self) -> AgentState {
        self.status.state()
    }

    /// Whether the provider has finished initializing.
    pub fn is_ready(&self) -> bool {
        self.ready
    }

    /// Number of prompts currently queued.
    pub fn queue_len(&self) -> usize {
        self.queue.len()
    }

    /// Number of prompts that have been delivered since ready.
    pub fn delivered(&self) -> usize {
        self.delivered
    }

    /// Submit a prompt.
    ///
    /// While the provider is starting the prompt is queued; once ready it is
    /// returned immediately.
    pub fn submit(&mut self, text: String) -> SubmitOutcome {
        if self.ready {
            self.status.transition(StatusEvent::PromptSent);
            self.delivered += 1;
            SubmitOutcome::Delivered
        } else {
            self.queue.push_back(text);
            SubmitOutcome::Queued
        }
    }

    /// Mark the provider ready and flush the queue.
    ///
    /// Transitions the status line to `Ready` and returns every prompt that
    /// was queued during startup, in order.
    pub fn provider_ready(&mut self) -> Vec<String> {
        if self.ready {
            return Vec::new();
        }
        self.ready = true;
        self.status.transition(StatusEvent::ProviderReady);
        self.delivered = self.queue.len();
        self.queue.drain(..).collect()
    }

    /// Render the current status line.
    pub fn status_line(&mut self, width: u16) -> String {
        self.status.render(width)
    }

    /// Render the full frame (banner + status line) for a redraw.
    pub fn frame(&mut self, width: u16) -> Vec<String> {
        let mut rows = self.banner.clone();
        rows.push(self.status.render(width));
        rows
    }
}
