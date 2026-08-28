//! `titi` — omp port in Rust.
//!
//! First-frame: banner + status line painted before the provider finishes
//! initializing; input typed during startup is queued and flushed to the
//! agent once the provider reports ready.  Contract:
//! `docs/research/agent-ux/README.md` (DoD, first item).

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

use titi_cli::first_frame::{FirstFrame, SubmitOutcome};

/// The startup banner shown before the provider is ready.
fn banner() -> Vec<String> {
    vec![
        format!("titi v{} — omp port in Rust", titi_core::VERSION),
        String::new(),
    ]
}

/// Provider initialization.  Runs on a background thread; flips `ready` once
/// the provider can accept prompts.
///
/// Currently resolves the credential ladder — the real transport setup lands
/// with the agent loop integration.
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

    let mut app = FirstFrame::new(banner());
    let (first, ttff) = app.first_frame(80);
    for row in &first {
        writeln!(stdout, "{row}")?;
    }
    stdout.flush()?;
    eprintln!("time-to-first-frame: {} ms", ttff.as_millis());

    // Input buffer.  During startup prompts queue; after ready they flush.
    let mut input = String::new();

    loop {
        if event::poll(Duration::from_millis(50))? && let Event::Key(key) = event::read()? {
            match key.code {
                    KeyCode::Char(c) if key.modifiers.contains(KeyModifiers::CONTROL) && c == 'c' => {
                        break;
                    }
                    KeyCode::Char(c) => {
                        input.push(c);
                        render(&mut app, &mut stdout, 80, &input)?;
                    }
                    KeyCode::Enter => {
                        if !input.is_empty() {
                            let prompt = std::mem::take(&mut input);
                            match app.submit(prompt.clone()) {
                                SubmitOutcome::Queued => {
                                    eprintln!("queued (provider starting): {prompt}");
                                }
                                SubmitOutcome::Delivered => {
                                    eprintln!("delivered: {prompt}");
                                }
                            }
                        }
                        render(&mut app, &mut stdout, 80, &input)?;
                    }
                    KeyCode::Backspace => {
                        input.pop();
                        render(&mut app, &mut stdout, 80, &input)?;
                    }
                _ => {}
            }
        }

        // Flush queued prompts once the provider is ready.
        if ready.load(Ordering::SeqCst) && !app.is_ready() {
            let flushed = app.provider_ready();
            for prompt in &flushed {
                eprintln!("flushed after ready: {prompt}");
            }
            if !flushed.is_empty() {
                render(&mut app, &mut stdout, 80, &input)?;
            }
        }
    }

    execute!(stdout, Show, LeaveAlternateScreen)?;
    disable_raw_mode()?;
    Ok(())
}

/// Repaint the banner + status line and the current input line.
fn render(app: &mut FirstFrame, stdout: &mut impl Write, width: u16, input: &str) -> io::Result<()> {
    for row in app.frame(width) {
        writeln!(stdout, "{row}")?;
    }
    write!(stdout, "> {input}")?;
    stdout.flush()
}
