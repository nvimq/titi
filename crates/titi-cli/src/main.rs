//! `titi` — omp port in Rust.
//!
//! First-frame: banner + status line painted before the provider finishes
//! initializing; input typed during startup is queued and flushed to the
//! agent once the provider reports ready.  Transcript accordion sections
//! render per DoD defaults: `/details <section> <mode>` switches visibility;
//! floating-alert backstop surfaces when every section is hidden.
//!
//! Contract: `docs/research/agent-ux/README.md` (DoD, first two items).

use std::io::{self, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use crossterm::cursor::{Hide, Show};
use crossterm::event::{self, Event, KeyCode, KeyModifiers};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};

use titi_cli::app::{default_theme, App};
use titi_cli::first_frame::SubmitOutcome;

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
    let ready = Arc::new(AtomicBool::new(false));
    {
        let ready = Arc::clone(&ready);
        std::thread::spawn(move || init_provider(ready));
    }

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, Hide)?;

    let theme = default_theme();
    let mut app = App::new(Arc::clone(&ready), banner(), theme);
    let rows = app.render();
    for row in &rows {
        writeln!(stdout, "{row}")?;
    }
    stdout.flush()?;
    eprintln!("time-to-first-frame: {} ms", app.time_to_first_frame().as_millis());

    let mut input = String::new();

    loop {
        if event::poll(Duration::from_millis(50))? && let Event::Key(key) = event::read()? {
            match key.code {
                KeyCode::Char(c) if key.modifiers.contains(KeyModifiers::CONTROL) && c == 'c' => {
                    break;
                }
                KeyCode::Char(c) => {
                    input.push(c);
                    render(&mut app, &mut stdout, &input)?;
                }
                KeyCode::Enter => {
                    if !input.is_empty() {
                        let cmd = std::mem::take(&mut input);
                        if cmd.starts_with("/details ") {
                            let directive = cmd.trim_start_matches("/details ");
                            app.details(directive);
                        } else {
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
                    render(&mut app, &mut stdout, &input)?;
                }
                KeyCode::Backspace => {
                    input.pop();
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

    execute!(stdout, Show, LeaveAlternateScreen)?;
    disable_raw_mode()?;
    Ok(())
}

/// Repaint: banner + transcript + status line + input line.
fn render(app: &mut App, stdout: &mut impl Write, input: &str) -> io::Result<()> {
    // Move cursor to top so we overwrite the previous frame.
    for row in app.render() {
        writeln!(stdout, "{row}")?;
    }
    write!(stdout, "> {input}")?;
    stdout.flush()
}