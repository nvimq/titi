//! `titi` — omp port in Rust.
//!
//! First-frame: banner + status line painted before the provider finishes
//! initializing; input typed during startup is queued and flushed to the
//! agent once the provider reports ready.  Transcript accordion sections
//! render per DoD defaults: `/details <section> <mode>` switches visibility;
//! floating-alert backstop surfaces when every section is hidden.
//!
//! Mouse: `--mouse <off|wheel|buttons|all>` selects the tracking preset
//! (1000/1002/1003 + SGR 1006); drag-select paints the selection background
//! (selectedBg) instead of SGR inverse.
//!
//! Contract: `docs/research/agent-ux/README.md` (DoD).

use std::io::{self, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use crossterm::cursor::{Hide, Show};
use crossterm::event::{
    self, DisableBracketedPaste, EnableBracketedPaste, Event, KeyCode, KeyEvent, KeyModifiers,
    MouseButton, MouseEvent, MouseEventKind,
};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, size, Clear, ClearType, EnterAlternateScreen,
    LeaveAlternateScreen,
};

use titi_cli::app::{
    default_theme, delete_session, load_mouse_preset, model_choices, save_mouse_preset, App,
    OverlayOutcome,
};
use titi_cli::first_frame::SubmitOutcome;
use titi_tui::caps::MousePreset;
use titi_tui::slash::Route;

/// The startup banner shown before the provider is ready.
fn banner() -> Vec<String> {
    vec![
        format!("titi v{} — omp port in Rust", titi_core::VERSION),
        String::new(),
    ]
}

/// Provider initialization.  Runs on a background thread; flips `ready` once
/// the provider can accept prompts.
fn init_provider(ready: Arc<AtomicBool>) {
    let _ = titi_providers::creds::resolve_credential(&titi_providers::creds::LadderCtx::default());
    ready.store(true, Ordering::SeqCst);
}

fn main() -> io::Result<()> {
    // `--mouse <preset>` (default: persisted `display.mouse_tracking`, else
    // off — no tracking, terminal-native selection works; drag-select
    // requires `buttons` or `all`).
    let mut mouse = load_mouse_preset().unwrap_or(MousePreset::Off);
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next()
        && arg == "--mouse"
        && let Some(preset) = args.next().and_then(|v| MousePreset::parse(&v))
    {
        mouse = preset;
    }

    let ready = Arc::new(AtomicBool::new(false));
    {
        let ready = Arc::clone(&ready);
        std::thread::spawn(move || init_provider(ready));
    }

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, Hide, EnableBracketedPaste)?;
    write!(stdout, "{}", mouse.enable())?;

    let theme = default_theme().map_err(io::Error::other)?;
    let mut app = App::new(Arc::clone(&ready), banner(), theme);
    if let Ok((w, _)) = size() {
        app.resize(w);
    }
    let rows = app.render();
    for row in &rows {
        writeln!(stdout, "{row}")?;
    }
    stdout.flush()?;
    eprintln!("time-to-first-frame: {} ms", app.time_to_first_frame().as_millis());
    eprintln!("mouse preset: {}", mouse.name());

    let mut input = String::new();

    loop {
        if event::poll(Duration::from_millis(50))? {
            match event::read()? {
                Event::Key(key) => {
                    if key.modifiers.contains(KeyModifiers::CONTROL)
                        && key.code == KeyCode::Char('c')
                    {
                        break;
                    }
                    // Modal: an open overlay consumes every key before the
                    // prompt line sees it.
                    if app.overlay_open() {
                        if let Some(data) = overlay_key_data(&key) {
                            if let Some(outcome) = app.overlay_input(data) {
                                handle_outcome(&mut app, outcome);
                            }
                            render(&mut app, &mut stdout, &input)?;
                        }
                    } else {
                        match key.code {
                            KeyCode::Char('x')
                                if key.modifiers.contains(KeyModifiers::CONTROL) =>
                            {
                                app.open_session_switcher();
                                render(&mut app, &mut stdout, &input)?;
                            }
                            KeyCode::Char('m')
                                if key.modifiers.contains(KeyModifiers::CONTROL) =>
                            {
                                app.open_model_picker(model_choices());
                                render(&mut app, &mut stdout, &input)?;
                            }
                            KeyCode::Char(c) => {
                                input.push(c);
                                app.slash_completions(&input);
                                render(&mut app, &mut stdout, &input)?;
                            }
                            KeyCode::Tab => {
                                if app.completion_visible() {
                                    if let Some(name) = app.completion_accept() {
                                        input = name;
                                    }
                                    render(&mut app, &mut stdout, &input)?;
                                }
                            }
                            KeyCode::Up => {
                                if app.completion_visible() {
                                    app.completion_move(true);
                                    render(&mut app, &mut stdout, &input)?;
                                }
                            }
                            KeyCode::Down => {
                                if app.completion_visible() {
                                    app.completion_move(false);
                                    render(&mut app, &mut stdout, &input)?;
                                }
                            }
                            KeyCode::Esc => {
                                if app.completion_visible() {
                                    app.completion_hide();
                                    render(&mut app, &mut stdout, &input)?;
                                }
                            }
                            KeyCode::Enter => {
                                if !input.is_empty() {
                                    let cmd = std::mem::take(&mut input);
                                    app.completion_hide();
                                    match app.route_slash(&cmd) {
                                        Route::Builtin(name) => match name.as_str() {
                                            "details" => {
                                                let directive = cmd.trim_start_matches("/details ");
                                                app.details(directive);
                                            }
                                            "mouse" => {
                                                let preset = cmd.trim_start_matches("/mouse ");
                                                if let Some(next) = MousePreset::parse(preset) {
                                                    write!(stdout, "{}", mouse.disable())?;
                                                    mouse = next;
                                                    write!(stdout, "{}", mouse.enable())?;
                                                    match save_mouse_preset(mouse) {
                                                        Ok(()) => {
                                                            eprintln!("mouse preset: {} (saved)", mouse.name());
                                                        }
                                                        Err(reason) => {
                                                            eprintln!(
                                                                "mouse preset: {} (not saved: {reason})",
                                                                mouse.name()
                                                            );
                                                        }
                                                    }
                                                }
                                            }
                                            "model" => app.open_model_picker(model_choices()),
                                            "sessions" => app.open_session_switcher(),
                                            "help" => eprintln!("commands: help, details, model, sessions, mouse"),
                                            other => eprintln!("builtin: {other}"),
                                        },
                                        Route::Expanded(prompt) => {
                                            match app.submit(prompt.clone()) {
                                                SubmitOutcome::Queued => {
                                                    eprintln!("queued (provider starting): {prompt}");
                                                }
                                                SubmitOutcome::Delivered => {
                                                    eprintln!("delivered: {prompt}");
                                                }
                                            }
                                        }
                                        Route::Passthrough => {
                                            match app.submit(cmd.clone()) {
                                                SubmitOutcome::Queued => {
                                                    eprintln!("queued (provider starting): {cmd}");
                                                }
                                                SubmitOutcome::Delivered => {
                                                    eprintln!("delivered: {cmd}");
                                                }
                                            }
                                        }
                                    }
                                }
                                render(&mut app, &mut stdout, &input)?;
                            }
                            KeyCode::Backspace => {
                                input.pop();
                                app.slash_completions(&input);
                                render(&mut app, &mut stdout, &input)?;
                            }
                            _ => {}
                        }
                    }
                }
                Event::Resize(w, _h) => {
                    app.resize(w);
                    render(&mut app, &mut stdout, &input)?;
                }
                Event::Mouse(mouse_event) => {
                    handle_mouse(&mut app, mouse_event);
                    render(&mut app, &mut stdout, &input)?;
                }
                Event::Paste(text) if !app.overlay_open() => {
                    // One paste = one event: the whole block is inserted
                    // into the buffer, never executed line-by-line.
                    let appended = app.paste(&text);
                    input.push_str(&appended);
                    render(&mut app, &mut stdout, &input)?;
                }
                _ => {}
            }
        }

        // Flush queued prompts once the provider is ready.
        let flushed = app.flush_queued(&ready);
        for prompt in &flushed {
            eprintln!("flushed after ready: {prompt}");
        }
        if !flushed.is_empty() {
            render(&mut app, &mut stdout, &input)?;
        }
    }

    execute!(stdout, Show, LeaveAlternateScreen, DisableBracketedPaste)?;
    write!(stdout, "{}", mouse.disable())?;
    disable_raw_mode()?;
    Ok(())
}

/// Route a mouse event into the app's selection model.
fn handle_mouse(app: &mut App, event: MouseEvent) {
    match event.kind {
        MouseEventKind::Down(MouseButton::Left) => app.mouse_press(event.column, event.row),
        MouseEventKind::Drag(MouseButton::Left) => app.mouse_drag(event.column, event.row),
        MouseEventKind::Up(MouseButton::Left) => app.mouse_release(),
        MouseEventKind::Moved => app.clear_selection(),
        _ => {}
    }
}

/// Map a key event to an overlay input sequence — panels consume raw
/// decoded input (Esc, arrows, Enter, Ctrl+D/N/R, k/j).  Keys without a
/// mapping are ignored while an overlay is open (modal).
fn overlay_key_data(key: &KeyEvent) -> Option<&'static str> {
    match (key.code, key.modifiers) {
        (KeyCode::Esc, _) => Some("\x1b"),
        (KeyCode::Enter, _) => Some("\r"),
        (KeyCode::Up, _) => Some("\x1b[A"),
        (KeyCode::Down, _) => Some("\x1b[B"),
        (KeyCode::Char('d'), m) if m.contains(KeyModifiers::CONTROL) => Some("\x04"),
        (KeyCode::Char('n'), m) if m.contains(KeyModifiers::CONTROL) => Some("\x0e"),
        (KeyCode::Char('r'), m) if m.contains(KeyModifiers::CONTROL) => Some("\x12"),
        (KeyCode::Char('k'), m) if !m.contains(KeyModifiers::CONTROL) => Some("k"),
        (KeyCode::Char('j'), m) if !m.contains(KeyModifiers::CONTROL) => Some("j"),
        _ => None,
    }
}

/// Act on a closed overlay panel's outcome.  An approval Yes is the only
/// path that deletes; Esc / No / Cancel never do.
fn handle_outcome(app: &mut App, outcome: OverlayOutcome) {
    match outcome {
        OverlayOutcome::ModelSelected(model) => eprintln!("model: {model}"),
        OverlayOutcome::SessionSwitched(id) => eprintln!("session: switched to {id}"),
        OverlayOutcome::SessionNew => eprintln!("session: new"),
        OverlayOutcome::SessionCancelled => {
            eprintln!("session switcher: cancelled (nothing deleted)");
        }
        OverlayOutcome::Approval(true) => match app.take_pending_close() {
            Some(id) if id == "current" => {
                eprintln!("session: cannot close the live session");
            }
            Some(id) => match delete_session(&id) {
                Ok(()) => eprintln!("session: closed {id}"),
                Err(reason) => eprintln!("session: not closed ({reason})"),
            },
            None => eprintln!("approval: yes (no pending action)"),
        },
        OverlayOutcome::Approval(false) => eprintln!("approval: declined (nothing deleted)"),
    }
}

/// Repaint: banner + transcript + status line + input line.
fn render(app: &mut App, stdout: &mut impl Write, input: &str) -> io::Result<()> {
    execute!(stdout, Clear(ClearType::All))?;
    // Move cursor to top so we overwrite the previous frame.
    for row in app.render() {
        writeln!(stdout, "{row}")?;
    }
    // Pasted multi-line text renders with indented continuation lines.
    let mut lines = input.split('\n');
    if let Some(first) = lines.next() {
        write!(stdout, "> {first}")?;
    }
    for line in lines {
        writeln!(stdout)?;
        write!(stdout, "  {line}")?;
    }
    stdout.flush()
}