//! Full-screen chat.
//!
//! The state machine does not touch the terminal, so tests drive it with
//! keys and engine events. [`run`] is the only place that owns the screen.

use std::io::{self, Stdout};
use std::time::{Duration, Instant};

use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::crossterm::execute;
use ratatui::crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::layout::{Alignment, Constraint, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Padding, Paragraph};
use titi_core::session::Role;
use titi_engine::{Engine, EngineCommand, EngineEvent};
use tokio::sync::mpsc::error::TryRecvError;

use crate::herdr::{self, AgentState};
use crate::session_log::SessionLog;

const QUIT_WINDOW: Duration = Duration::from_secs(2);
const TOOL_PREVIEW: usize = 120;

/// One key the state machine understands. The terminal loop translates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    Char(char),
    Backspace,
    Enter,
    CtrlC,
    CtrlD,
    Esc,
}

/// What the screen asks the engine or the process to do.
#[derive(Debug, Clone, PartialEq)]
pub enum ChatEffect {
    Send(EngineCommand),
    Quit,
}

/// A command for the engine, plus the transcript line that should be stored.
#[derive(Debug, Clone, PartialEq)]
pub struct Applied {
    pub effect: Option<ChatEffect>,
    pub log: Option<(Role, String)>,
}

impl Applied {
    fn none() -> Self {
        Self {
            effect: None,
            log: None,
        }
    }

    fn effect(effect: ChatEffect) -> Self {
        Self {
            effect: Some(effect),
            log: None,
        }
    }

    fn send(command: EngineCommand, log: Option<(Role, String)>) -> Self {
        Self {
            effect: Some(ChatEffect::Send(command)),
            log,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LineKind {
    User,
    Assistant,
    Tool,
    Error,
    Note,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TranscriptLine {
    kind: LineKind,
    text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PendingApproval {
    call_id: String,
    name: String,
}

/// Conversation on screen. No terminal, no session file.
pub struct Chat {
    lines: Vec<TranscriptLine>,
    input: String,
    turn_active: bool,
    model: String,
    models: Vec<String>,
    session_label: String,
    context_percent: Option<u8>,
    reply: String,
    thinking: String,
    assistant_at: Option<usize>,
    thinking_at: Option<usize>,
    approval: Option<PendingApproval>,
    quit_armed: Option<Instant>,
    hint: String,
}

impl Chat {
    pub fn new(model: impl Into<String>, session_id: &str) -> Self {
        let model = model.into();
        Self {
            lines: Vec::new(),
            input: String::new(),
            turn_active: false,
            model: model.clone(),
            models: vec![model],
            session_label: short_session(session_id),
            context_percent: None,
            reply: String::new(),
            thinking: String::new(),
            assistant_at: None,
            thinking_at: None,
            approval: None,
            quit_armed: None,
            hint: String::new(),
        }
    }

    pub fn on_key(&mut self, key: Key, now: Instant) -> Applied {
        if self.approval.is_some() {
            return self.approval_key(key);
        }
        match key {
            Key::CtrlC if self.turn_active => {
                self.disarm();
                Applied::effect(ChatEffect::Send(EngineCommand::Cancel))
            }
            Key::CtrlC => self.arm_quit(now),
            Key::CtrlD if self.input.is_empty() => Applied::effect(ChatEffect::Quit),
            Key::Enter => self.submit(),
            Key::Backspace => {
                self.disarm();
                self.input.pop();
                Applied::none()
            }
            Key::Char(ch) => {
                self.disarm();
                self.input.push(ch);
                Applied::none()
            }
            Key::Esc | Key::CtrlD => {
                self.disarm();
                Applied::none()
            }
        }
    }

    pub fn on_event(&mut self, event: EngineEvent) -> Applied {
        match event {
            EngineEvent::TurnStarted { model, .. } => {
                self.turn_active = true;
                self.model = model.to_string();
                self.reply.clear();
                self.assistant_at = None;
                self.drop_thinking();
                Applied::none()
            }
            EngineEvent::StreamDelta { text, .. } => {
                self.drop_thinking();
                self.reply.push_str(&text);
                self.show_reply();
                Applied::none()
            }
            EngineEvent::ThinkingDelta { text, .. } if self.reply.is_empty() => {
                self.thinking.push_str(&text);
                self.show_thinking();
                Applied::none()
            }
            EngineEvent::ToolStarted { name, .. } => {
                self.push(LineKind::Tool, format!("tool {name}"));
                Applied::none()
            }
            EngineEvent::ToolApprovalNeeded { call_id, name, .. } => {
                self.approval = Some(PendingApproval {
                    call_id: call_id.to_string(),
                    name: name.to_string(),
                });
                self.hint.clear();
                Applied::none()
            }
            EngineEvent::ToolFinished {
                output, is_error, ..
            } => {
                let preview = one_line(&output, TOOL_PREVIEW);
                let text = if is_error {
                    format!("tool error  {preview}")
                } else if preview.is_empty() {
                    "tool done".to_owned()
                } else {
                    format!("tool done  {preview}")
                };
                let kind = if is_error {
                    LineKind::Error
                } else {
                    LineKind::Tool
                };
                self.push(kind, text);
                Applied::none()
            }
            EngineEvent::ContextUsage { tokens, window, .. } if window > 0 => {
                let percent = tokens.saturating_mul(100) / window;
                self.context_percent = Some(u8::try_from(percent.min(100)).unwrap_or(100));
                Applied::none()
            }
            EngineEvent::ModelSwitched { to, .. } => {
                self.model = to.to_string();
                self.push(LineKind::Note, format!("model {to}"));
                Applied::none()
            }
            EngineEvent::Compacted { folded, .. } => {
                self.push(LineKind::Note, format!("folded {folded} earlier messages"));
                Applied::none()
            }
            EngineEvent::Failed { message, .. } => {
                self.push(LineKind::Error, one_line(&message, TOOL_PREVIEW));
                self.finish_turn()
            }
            EngineEvent::Cancelled { .. } => {
                self.push(LineKind::Note, "cancelled".to_owned());
                self.finish_turn()
            }
            EngineEvent::TurnFinished { .. } => self.finish_turn(),
            _ => Applied::none(),
        }
    }

    /// Insert pasted text into the composer. Newlines become spaces.
    pub fn paste(&mut self, text: &str) {
        if self.approval.is_some() {
            return;
        }
        self.disarm();
        for ch in text.chars() {
            if ch == '\n' || ch == '\r' {
                if !self.input.ends_with(' ') {
                    self.input.push(' ');
                }
            } else if !ch.is_control() {
                self.input.push(ch);
            }
        }
    }

    fn approval_key(&mut self, key: Key) -> Applied {
        let Some(pending) = self.approval.clone() else {
            return Applied::none();
        };
        match key {
            Key::Char('y') | Key::Char('Y') | Key::Enter => {
                self.approval = None;
                Applied::effect(ChatEffect::Send(EngineCommand::ApproveTool {
                    call_id: pending.call_id.into(),
                    approved: true,
                }))
            }
            Key::Char('n') | Key::Char('N') | Key::Esc => {
                self.approval = None;
                Applied::effect(ChatEffect::Send(EngineCommand::ApproveTool {
                    call_id: pending.call_id.into(),
                    approved: false,
                }))
            }
            Key::CtrlC => {
                self.approval = None;
                self.disarm();
                Applied::effect(ChatEffect::Send(EngineCommand::Cancel))
            }
            _ => Applied::none(),
        }
    }

    fn submit(&mut self) -> Applied {
        let text = self.input.trim().to_owned();
        if text.is_empty() {
            return Applied::none();
        }
        if let Some(applied) = self.switch_model(&text) {
            self.input.clear();
            self.disarm();
            return applied;
        }
        self.input.clear();
        self.disarm();
        self.push(LineKind::User, text.clone());
        let log = Some((Role::User, text.clone()));
        if self.turn_active {
            Applied::send(EngineCommand::Steer { text: text.into() }, log)
        } else {
            self.turn_active = true;
            Applied::send(EngineCommand::SubmitPrompt { text: text.into() }, log)
        }
    }

    fn switch_model(&mut self, text: &str) -> Option<Applied> {
        let Some(raw) = text.strip_prefix("/model") else {
            return None;
        };
        if !raw.is_empty() && !raw.starts_with(char::is_whitespace) {
            return None;
        }
        let rest = raw.trim();
        if self.models.is_empty() {
            self.push(LineKind::Error, "no models".to_owned());
            return Some(Applied::none());
        }
        let next = if rest.is_empty() {
            let index = self
                .models
                .iter()
                .position(|id| id == &self.model)
                .unwrap_or(0);
            self.models[(index + 1) % self.models.len()].clone()
        } else if let Some(found) = self
            .models
            .iter()
            .find(|id| id.as_str() == rest || id.rsplit('/').next() == Some(rest))
        {
            found.clone()
        } else {
            self.push(LineKind::Error, format!("unknown model {rest}"));
            return Some(Applied::none());
        };
        self.model = next.clone();
        self.push(LineKind::Note, format!("model {next}"));
        Some(Applied::send(
            EngineCommand::SwitchModel { model: next.into() },
            None,
        ))
    }

    fn arm_quit(&mut self, now: Instant) -> Applied {
        if let Some(armed) = self.quit_armed
            && now.saturating_duration_since(armed) <= QUIT_WINDOW
        {
            return Applied::effect(ChatEffect::Quit);
        }
        self.quit_armed = Some(now);
        self.hint = "ctrl-c again to quit".to_owned();
        Applied::none()
    }

    fn disarm(&mut self) {
        self.quit_armed = None;
        self.hint.clear();
    }

    fn finish_turn(&mut self) -> Applied {
        let reply = std::mem::take(&mut self.reply);
        self.turn_active = false;
        self.approval = None;
        self.assistant_at = None;
        self.drop_thinking();
        if reply.trim().is_empty() {
            Applied::none()
        } else {
            Applied {
                effect: None,
                log: Some((Role::Assistant, reply)),
            }
        }
    }

    fn show_reply(&mut self) {
        let text = self.reply.clone();
        if let Some(at) = self.assistant_at
            && let Some(line) = self.lines.get_mut(at)
        {
            line.text = text;
            return;
        }
        self.lines.push(TranscriptLine {
            kind: LineKind::Assistant,
            text,
        });
        self.assistant_at = Some(self.lines.len() - 1);
    }

    fn show_thinking(&mut self) {
        let text = format!("thinking  {}", tail_chars(&self.thinking, 100));
        if let Some(at) = self.thinking_at
            && let Some(line) = self.lines.get_mut(at)
        {
            line.text = text;
            return;
        }
        self.lines.push(TranscriptLine {
            kind: LineKind::Note,
            text,
        });
        self.thinking_at = Some(self.lines.len() - 1);
    }

    fn drop_thinking(&mut self) {
        if let Some(index) = self.thinking_at.take()
            && index < self.lines.len()
        {
            self.lines.remove(index);
            if let Some(at) = self.assistant_at.as_mut()
                && *at > index
            {
                *at -= 1;
            }
        }
        self.thinking.clear();
    }

    fn push(&mut self, kind: LineKind, text: String) {
        self.lines.push(TranscriptLine { kind, text });
    }

    fn agent_state(&self) -> AgentState {
        if self.approval.is_some() || self.quit_armed.is_some() {
            AgentState::Blocked
        } else if self.turn_active {
            AgentState::Working
        } else {
            AgentState::Idle
        }
    }
}

/// Draws the chat until the user quits. Restores the terminal on the way out.
pub fn run(
    mut engine: Engine,
    session_log: Option<SessionLog>,
    models: Vec<String>,
    session_id: String,
) -> io::Result<()> {
    let model = models
        .first()
        .cloned()
        .unwrap_or_else(|| "model".to_owned());
    let mut chat = Chat::new(model, &session_id);
    if !models.is_empty() {
        chat.models = models;
    }
    let mut herdr_reporter = herdr::Reporter::from_env();
    if let Some(reporter) = &mut herdr_reporter {
        reporter.set_session(&session_id);
        reporter.report(AgentState::Idle, None);
    }
    let mut reported = AgentState::Idle;
    let mut screen = Screen::open()?;
    let result = loop {
        let state = chat.agent_state();
        if state != reported
            && let Some(reporter) = &herdr_reporter
        {
            reporter.report(state, None);
            reported = state;
        }
        screen.terminal.draw(|frame| draw(frame, &chat))?;
        if pump(&mut engine, &mut chat, &session_log)? {
            break Ok(());
        }
    };
    if let Some(reporter) = &herdr_reporter {
        reporter.report(AgentState::Idle, None);
    }
    result
}

struct Screen {
    terminal: Terminal<CrosstermBackend<Stdout>>,
}

impl Screen {
    fn open() -> io::Result<Self> {
        enable_raw_mode()?;
        let mut stdout = io::stdout();
        execute!(
            stdout,
            EnterAlternateScreen,
            ratatui::crossterm::cursor::Hide
        )?;
        let backend = CrosstermBackend::new(stdout);
        let terminal = Terminal::new(backend)?;
        Ok(Self { terminal })
    }
}

impl Drop for Screen {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(
            self.terminal.backend_mut(),
            ratatui::crossterm::cursor::Show,
            LeaveAlternateScreen
        );
    }
}

fn draw(frame: &mut ratatui::Frame<'_>, chat: &Chat) {
    let area = frame.area();
    let ink = Ink::titanium();
    frame.render_widget(Block::default().style(ink.page()), area);
    if area.height < 6 || area.width < 16 {
        return;
    }
    let cols = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(1),
        Constraint::Length(4),
    ])
    .split(area);
    frame.render_widget(masthead(chat, cols[0].width, &ink), cols[0]);
    if chat.lines.is_empty() {
        frame.render_widget(empty_state(cols[1].height, &ink), cols[1]);
    } else {
        frame.render_widget(
            transcript(chat, cols[1].width, cols[1].height, &ink),
            cols[1],
        );
    }
    frame.render_widget(composer(chat, cols[2].width, &ink), cols[2]);
}

/// Titanium, softened so the accent does not glow like a default cyan theme.
struct Ink {
    page: Color,
    card: Color,
    line: Color,
    text: Color,
    muted: Color,
    dim: Color,
    accent: Color,
    gold: Color,
    green: Color,
    amber: Color,
    red: Color,
}

impl Ink {
    fn titanium() -> Self {
        Self {
            page: Color::Rgb(12, 14, 18),
            card: Color::Rgb(21, 24, 32),
            line: Color::Rgb(42, 48, 56),
            text: Color::Rgb(232, 236, 244),
            muted: Color::Rgb(156, 163, 176),
            dim: Color::Rgb(107, 114, 128),
            accent: Color::Rgb(110, 186, 255),
            gold: Color::Rgb(212, 192, 144),
            green: Color::Rgb(125, 211, 168),
            amber: Color::Rgb(255, 179, 71),
            red: Color::Rgb(255, 112, 128),
        }
    }

    fn page(&self) -> Style {
        Style::default().bg(self.page).fg(self.text)
    }

    fn fg(&self, color: Color) -> Style {
        Style::default().fg(color)
    }
}

fn masthead(chat: &Chat, width: u16, ink: &Ink) -> Paragraph<'static> {
    let state = if chat.approval.is_some() {
        "needs you"
    } else if chat.turn_active {
        "working"
    } else {
        "ready"
    };
    let state_color = if chat.approval.is_some() {
        ink.amber
    } else if chat.turn_active {
        ink.accent
    } else {
        ink.dim
    };
    let ctx = match chat.context_percent {
        Some(percent) => format!("  {percent}%"),
        None => String::new(),
    };
    let left = " titi";
    let mid = format!("  {state}");
    let mut right = format!("{}{ctx}  {} ", chat.model, chat.session_label);
    let fixed = titi_tui::width::visible_width(left) + titi_tui::width::visible_width(&mid) + 2;
    let room = (width as usize).saturating_sub(fixed);
    if titi_tui::width::visible_width(&right) > room {
        right = titi_tui::width::truncate_to_width(&right, room);
    }
    let used = fixed + titi_tui::width::visible_width(&right);
    let gap = (width as usize).saturating_sub(used);
    let line = Line::from(vec![
        Span::styled(left, ink.fg(ink.accent).add_modifier(Modifier::BOLD)),
        Span::styled(mid, ink.fg(state_color)),
        Span::styled(" ".repeat(gap), ink.page()),
        Span::styled(right, ink.fg(ink.muted)),
    ]);
    Paragraph::new(line).style(ink.page())
}

fn empty_state(height: u16, ink: &Ink) -> Paragraph<'static> {
    let block = 5usize;
    let pad = (height as usize).saturating_sub(block) / 2;
    let mut rows = vec![Line::from(""); pad];
    rows.push(Line::from(Span::styled(
        "titi",
        ink.fg(ink.accent).add_modifier(Modifier::BOLD),
    )));
    rows.push(Line::from(""));
    rows.push(Line::from(Span::styled(
        "say what you want done",
        ink.fg(ink.muted),
    )));
    rows.push(Line::from(""));
    rows.push(Line::from(Span::styled(
        "enter  send      /model  switch      ctrl-c  quit",
        ink.fg(ink.dim),
    )));
    Paragraph::new(rows)
        .alignment(Alignment::Center)
        .style(ink.page())
}

fn transcript(chat: &Chat, width: u16, height: u16, ink: &Ink) -> Paragraph<'static> {
    let inner = (width as usize).saturating_sub(2).max(8);
    let mut rows: Vec<Line<'static>> = Vec::new();
    for (index, line) in chat.lines.iter().enumerate() {
        let gap = matches!(line.kind, LineKind::User | LineKind::Assistant) && index > 0;
        if gap && !rows.is_empty() {
            rows.push(Line::from(""));
        }
        rows.extend(message_rows(line, inner, ink));
    }
    let keep = height as usize;
    let start = rows.len().saturating_sub(keep);
    Paragraph::new(rows.into_iter().skip(start).collect::<Vec<_>>()).style(ink.page())
}

fn message_rows(line: &TranscriptLine, width: usize, ink: &Ink) -> Vec<Line<'static>> {
    match line.kind {
        LineKind::User => speech("you", ink.gold, ink.text, &line.text, width, ink),
        LineKind::Assistant => speech("titi", ink.accent, ink.text, &line.text, width, ink),
        LineKind::Tool => vec![chip(tool_chip(&line.text, ink), ink, width)],
        LineKind::Error => vec![chip(("✕", ink.red, line.text.clone(), ink.red), ink, width)],
        LineKind::Note => vec![chip(("·", ink.dim, line.text.clone(), ink.dim), ink, width)],
    }
}

fn speech(
    name: &str,
    label: Color,
    body: Color,
    text: &str,
    width: usize,
    ink: &Ink,
) -> Vec<Line<'static>> {
    // "you" and "titi" share a column so a short message stays one row.
    let tag = format!("{name:<4}");
    let wrap_at = width.saturating_sub(10).max(8);
    let pieces = wrap_plain(text, wrap_at);
    let mut rows = Vec::with_capacity(pieces.len());
    for (index, piece) in pieces.into_iter().enumerate() {
        let row = if index == 0 {
            vec![
                Span::styled("  ", ink.page()),
                Span::styled(tag.clone(), ink.fg(label).add_modifier(Modifier::BOLD)),
                Span::styled(" │ ", ink.fg(label)),
                Span::styled(piece, ink.fg(body)),
            ]
        } else {
            vec![
                Span::styled("       │ ", ink.fg(label)),
                Span::styled(piece, ink.fg(body)),
            ]
        };
        rows.push(Line::from(row));
    }
    rows
}

fn tool_chip(text: &str, ink: &Ink) -> (&'static str, Color, String, Color) {
    // The engine records "tool <name>" and "tool done  <preview>".
    // The screen says the same thing without the debug prefix.
    if let Some(rest) = text.strip_prefix("tool error") {
        return ("✕", ink.red, rest.trim().to_owned(), ink.red);
    }
    if let Some(rest) = text.strip_prefix("tool done") {
        let detail = rest.trim();
        let body = if detail.is_empty() {
            "done".to_owned()
        } else {
            detail.to_owned()
        };
        return ("✓", ink.green, body, ink.muted);
    }
    if let Some(rest) = text.strip_prefix("tool ") {
        return ("▸", ink.amber, rest.trim().to_owned(), ink.gold);
    }
    ("▸", ink.amber, text.to_owned(), ink.muted)
}

fn chip(parts: (&str, Color, String, Color), ink: &Ink, width: usize) -> Line<'static> {
    let (mark, mark_color, text, text_color) = parts;
    let room = width.saturating_sub(6).max(4);
    let shown = titi_tui::width::truncate_to_width(&text, room);
    Line::from(vec![
        Span::styled("   ", ink.page()),
        Span::styled(mark.to_owned(), ink.fg(mark_color)),
        Span::styled(" ", ink.page()),
        Span::styled(shown, ink.fg(text_color)),
    ])
}

fn composer(chat: &Chat, width: u16, ink: &Ink) -> Paragraph<'static> {
    let (border, caption_color) = if chat.approval.is_some() {
        (ink.amber, ink.amber)
    } else if chat.turn_active {
        (ink.accent, ink.accent)
    } else {
        (ink.line, ink.dim)
    };
    let caption = titi_tui::width::truncate_to_width(
        &composer_caption(chat),
        (width as usize).saturating_sub(4),
    );
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(ink.fg(border))
        .title_bottom(
            Line::from(Span::styled(format!(" {caption} "), ink.fg(caption_color))).centered(),
        )
        .padding(Padding::horizontal(1))
        .style(Style::default().bg(ink.card).fg(ink.text));
    let inner = (width as usize).saturating_sub(6).max(4);
    let line = if let Some(pending) = &chat.approval {
        Line::from(Span::styled(
            titi_tui::width::truncate_to_width(
                &format!("{}   y allow    n refuse", pending.name),
                inner,
            ),
            ink.fg(ink.amber).add_modifier(Modifier::BOLD),
        ))
    } else if chat.input.is_empty() {
        let placeholder = if chat.turn_active {
            "steer this turn…"
        } else {
            "ask titi…"
        };
        Line::from(vec![
            Span::styled("› ", ink.fg(ink.accent)),
            Span::styled(placeholder, ink.fg(ink.dim)),
        ])
    } else {
        let room = inner.saturating_sub(4).max(1);
        Line::from(vec![
            Span::styled("› ", ink.fg(ink.accent)),
            Span::styled(fit_tail(&chat.input, room), ink.fg(ink.text)),
            Span::styled("▍", ink.fg(ink.accent)),
        ])
    };
    Paragraph::new(line).block(block)
}

fn composer_caption(chat: &Chat) -> String {
    let ctx = match chat.context_percent {
        Some(percent) => format!("{percent}%  ·  "),
        None => String::new(),
    };
    let keys = if chat.approval.is_some() {
        "y allow  ·  n refuse"
    } else if !chat.hint.is_empty() {
        chat.hint.as_str()
    } else if chat.turn_active {
        "enter steers  ·  ctrl-c stops"
    } else {
        "enter sends  ·  /model  ·  ctrl-c quits"
    };
    format!("{ctx}{keys}")
}

fn wrap_plain(text: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    let mut rows = Vec::new();
    for paragraph in text.split('\n') {
        if paragraph.is_empty() {
            rows.push(String::new());
            continue;
        }
        let chars: Vec<char> = paragraph.chars().collect();
        let mut index = 0;
        while index < chars.len() {
            let mut col = 0usize;
            let mut last_space = None;
            let mut end = index;
            while end < chars.len() {
                let cell = titi_tui::width::visible_width(&chars[end].to_string());
                if col + cell > width && end > index {
                    break;
                }
                if chars[end] == ' ' {
                    last_space = Some(end);
                }
                col += cell;
                end += 1;
            }
            let cut = if end < chars.len() {
                last_space.filter(|at| *at > index).unwrap_or(end)
            } else {
                end
            };
            let piece: String = chars[index..cut].iter().collect();
            rows.push(piece.trim_end().to_owned());
            index = cut;
            while index < chars.len() && chars[index] == ' ' {
                index += 1;
            }
        }
    }
    if rows.is_empty() {
        rows.push(String::new());
    }
    rows
}

fn fit_tail(text: &str, width: usize) -> String {
    if titi_tui::width::visible_width(text) <= width {
        return text.to_owned();
    }
    let chars: Vec<char> = text.chars().collect();
    let mut end = chars.len();
    let mut col = 0usize;
    let room = width.saturating_sub(1);
    while end > 0 {
        let cell = titi_tui::width::visible_width(&chars[end - 1].to_string());
        if col + cell > room {
            break;
        }
        col += cell;
        end -= 1;
    }
    format!("…{}", chars[end..].iter().collect::<String>())
}

fn short_session(id: &str) -> String {
    let chars: Vec<char> = id.chars().collect();
    if chars.len() <= 14 {
        id.to_owned()
    } else {
        chars[chars.len() - 12..].iter().collect()
    }
}

fn one_line(text: &str, max: usize) -> String {
    let mut out = String::new();
    let mut count = 0;
    for ch in text.chars() {
        if count >= max {
            out.push('…');
            break;
        }
        if ch.is_control() {
            if !out.ends_with(' ') {
                out.push(' ');
                count += 1;
            }
        } else {
            out.push(ch);
            count += 1;
        }
    }
    out
}

fn tail_chars(text: &str, max: usize) -> String {
    let chars: Vec<char> = text.chars().collect();
    if chars.len() <= max {
        text.to_owned()
    } else {
        chars[chars.len() - max..].iter().collect()
    }
}

fn map_key(code: KeyCode, modifiers: KeyModifiers) -> Option<Key> {
    let control = modifiers.contains(KeyModifiers::CONTROL);
    match code {
        KeyCode::Char('c') if control => Some(Key::CtrlC),
        KeyCode::Char('d') if control => Some(Key::CtrlD),
        KeyCode::Char(ch) if !control && !modifiers.contains(KeyModifiers::ALT) => {
            Some(Key::Char(ch))
        }
        KeyCode::Backspace => Some(Key::Backspace),
        KeyCode::Enter => Some(Key::Enter),
        KeyCode::Esc => Some(Key::Esc),
        _ => None,
    }
}

/// Polls the keyboard and the engine once. `Ok(true)` means the user quit.
fn pump(
    engine: &mut Engine,
    chat: &mut Chat,
    session_log: &Option<SessionLog>,
) -> io::Result<bool> {
    if event::poll(Duration::from_millis(50))? {
        match event::read()? {
            Event::Key(key) if key.kind == KeyEventKind::Press => {
                if let Some(mapped) = map_key(key.code, key.modifiers) {
                    let applied = chat.on_key(mapped, Instant::now());
                    if dispatch(engine, chat, session_log, applied) {
                        return Ok(true);
                    }
                }
            }
            Event::Paste(text) => chat.paste(&text),
            _ => {}
        }
    }
    loop {
        match engine.try_recv() {
            Ok(event) => {
                let applied = chat.on_event(event);
                record(chat, session_log, applied.log);
            }
            Err(TryRecvError::Empty) => break,
            Err(TryRecvError::Disconnected) => {
                chat.push(LineKind::Error, "engine stopped".to_owned());
                break;
            }
        }
    }
    Ok(false)
}

fn dispatch(
    engine: &mut Engine,
    chat: &mut Chat,
    session_log: &Option<SessionLog>,
    applied: Applied,
) -> bool {
    record(chat, session_log, applied.log);
    match applied.effect {
        Some(ChatEffect::Quit) => {
            let _ = engine.try_send(EngineCommand::Shutdown);
            true
        }
        Some(ChatEffect::Send(command)) => {
            if engine.try_send(command).is_err() {
                chat.push(LineKind::Error, "engine stopped".to_owned());
            }
            false
        }
        None => false,
    }
}

fn record(chat: &mut Chat, session_log: &Option<SessionLog>, write: Option<(Role, String)>) {
    let Some(log) = session_log else {
        return;
    };
    let Some((role, text)) = write else {
        return;
    };
    let result = match role {
        Role::User => log.user(&text),
        Role::Assistant => log.assistant(&text),
        Role::System => log.system(&text),
    };
    if let Err(reason) = result {
        chat.push(LineKind::Error, format!("session: not saved ({reason})"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use titi_engine::TurnId;
    use titi_providers::StopReason;

    fn chat() -> Chat {
        Chat::new("openai/gpt-4.1", "session-123")
    }

    fn type_text(chat: &mut Chat, text: &str) {
        let now = Instant::now();
        for ch in text.chars() {
            chat.on_key(Key::Char(ch), now);
        }
    }

    #[test]
    fn enter_while_idle_submits_and_logs_the_user() {
        let mut chat = chat();
        type_text(&mut chat, "hi");
        let applied = chat.on_key(Key::Enter, Instant::now());
        assert!(matches!(
            applied.effect,
            Some(ChatEffect::Send(EngineCommand::SubmitPrompt { .. }))
        ));
        assert_eq!(applied.log, Some((Role::User, "hi".to_owned())));
        assert!(chat.turn_active);
    }

    #[test]
    fn enter_during_a_turn_steers_and_leaves_it_active() {
        let mut chat = chat();
        chat.on_event(EngineEvent::TurnStarted {
            turn_id: TurnId(1),
            model: "openai/gpt-4.1".into(),
        });
        type_text(&mut chat, "look again");
        let applied = chat.on_key(Key::Enter, Instant::now());
        match applied.effect {
            Some(ChatEffect::Send(EngineCommand::Steer { text })) => {
                assert_eq!(text.as_str(), "look again");
            }
            other => panic!("expected steer, got {other:?}"),
        }
        assert!(chat.turn_active);
        assert_eq!(applied.log, Some((Role::User, "look again".to_owned())));
    }

    #[test]
    fn approval_yes_and_no() {
        let mut chat = chat();
        chat.on_event(EngineEvent::ToolApprovalNeeded {
            turn_id: TurnId(1),
            call_id: "call-1".into(),
            name: "bash".into(),
        });
        let yes = chat.on_key(Key::Char('y'), Instant::now());
        match yes.effect {
            Some(ChatEffect::Send(EngineCommand::ApproveTool { call_id, approved })) => {
                assert_eq!(call_id.as_str(), "call-1");
                assert!(approved);
            }
            other => panic!("expected approval, got {other:?}"),
        }
        assert!(chat.approval.is_none());

        chat.on_event(EngineEvent::ToolApprovalNeeded {
            turn_id: TurnId(1),
            call_id: "call-2".into(),
            name: "edit".into(),
        });
        chat.on_key(Key::Char('x'), Instant::now());
        assert!(chat.input.is_empty());
        let no = chat.on_key(Key::Char('n'), Instant::now());
        match no.effect {
            Some(ChatEffect::Send(EngineCommand::ApproveTool { approved, .. })) => {
                assert!(!approved);
            }
            other => panic!("expected refusal, got {other:?}"),
        }
    }

    #[test]
    fn stream_accumulates_and_finish_logs_the_assistant() {
        let mut chat = chat();
        chat.on_event(EngineEvent::TurnStarted {
            turn_id: TurnId(7),
            model: "openai/gpt-4.1".into(),
        });
        chat.on_event(EngineEvent::StreamDelta {
            turn_id: TurnId(7),
            text: "hel".into(),
        });
        chat.on_event(EngineEvent::StreamDelta {
            turn_id: TurnId(7),
            text: "lo".into(),
        });
        let applied = chat.on_event(EngineEvent::TurnFinished {
            turn_id: TurnId(7),
            reason: StopReason::Stop,
        });
        assert_eq!(applied.log, Some((Role::Assistant, "hello".to_owned())));
        assert!(!chat.turn_active);
    }

    #[test]
    fn second_ctrl_c_within_two_seconds_quits() {
        let mut chat = chat();
        let start = Instant::now();
        let first = chat.on_key(Key::CtrlC, start);
        assert!(first.effect.is_none());
        let second = chat.on_key(Key::CtrlC, start + Duration::from_millis(500));
        assert_eq!(second.effect, Some(ChatEffect::Quit));
    }

    #[test]
    fn ctrl_c_during_a_turn_cancels() {
        let mut chat = chat();
        chat.on_event(EngineEvent::TurnStarted {
            turn_id: TurnId(1),
            model: "openai/gpt-4.1".into(),
        });
        let applied = chat.on_key(Key::CtrlC, Instant::now());
        assert_eq!(
            applied.effect,
            Some(ChatEffect::Send(EngineCommand::Cancel))
        );
    }

    #[test]
    fn slash_model_cycles_without_sending_a_prompt() {
        let mut chat = chat();
        chat.models = vec![
            "openai/gpt-4.1".to_owned(),
            "opencode-go/glm-5.3-flash".to_owned(),
        ];
        type_text(&mut chat, "/model");
        let applied = chat.on_key(Key::Enter, Instant::now());
        match applied.effect {
            Some(ChatEffect::Send(EngineCommand::SwitchModel { model })) => {
                assert_eq!(model.as_str(), "opencode-go/glm-5.3-flash");
            }
            other => panic!("expected a model switch, got {other:?}"),
        }
        assert!(applied.log.is_none());
        assert!(!chat.turn_active);
    }

    #[test]
    fn a_late_ctrl_c_does_not_quit() {
        let mut chat = chat();
        let start = Instant::now();
        chat.on_key(Key::CtrlC, start);
        let later = chat.on_key(Key::CtrlC, start + Duration::from_secs(3));
        assert!(later.effect.is_none());
        assert!(chat.quit_armed.is_some());
    }

    #[test]
    fn a_frame_shows_the_model_the_reply_and_the_composer() {
        let backend = ratatui::backend::TestBackend::new(80, 24);
        let mut terminal = match ratatui::Terminal::new(backend) {
            Ok(terminal) => terminal,
            Err(error) => panic!("test backend: {error}"),
        };
        let mut chat = chat();
        type_text(&mut chat, "hi");
        chat.on_key(Key::Enter, Instant::now());
        chat.on_event(EngineEvent::StreamDelta {
            turn_id: TurnId(1),
            text: "hello".into(),
        });
        assert!(terminal.draw(|frame| draw(frame, &chat)).is_ok());
        let view: String = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        assert!(view.contains("titi"), "{view}");
        assert!(view.contains("openai/gpt-4.1"), "{view}");
        assert!(view.contains("you"), "{view}");
        assert!(view.contains("hi"), "{view}");
        assert!(view.contains("hello"), "{view}");
    }
}
