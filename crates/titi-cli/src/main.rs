//! `titi` — omp port in Rust.
//!
//! First-frame: banner + OMP box composer (status in the top border) painted
//! before the provider finishes initializing; input typed during startup is
//! queued and flushed to the agent once the provider reports ready.
//! Transcript accordion sections render per DoD defaults.
//!
//! Mouse: `--mouse <off|on|wheel|buttons|all>` selects the tracking preset
//! (1000/1002/1003 + SGR 1006); drag-select paints the selection background
//! (selectedBg) instead of SGR inverse.
//!
//! Contract: `docs/research/agent-ux/README.md` (DoD).

use std::io::{self, Write};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use crossterm::cursor::{Hide, Show};
use crossterm::event::{
    self, DisableBracketedPaste, DisableFocusChange, EnableBracketedPaste, EnableFocusChange,
    Event, MouseButton, MouseEvent, MouseEventKind,
};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode, size,
};

use titi_cli::app::{
    App, Dispatch, OverlayOutcome, SubmitEffect, default_theme, delete_session, load_mouse_preset,
    save_mouse_preset,
};
use titi_cli::keys::{canonical_from_key_event, overlay_key_data};
use titi_engine::EngineCommand;
use titi_tui::caps::{MODE_2031_DISABLE, MODE_2031_ENABLE, MousePreset, OSC11_QUERY, osc52_copy};
use titi_tui::renderer::{FramePlan, FrameProvider, Renderer, ResizeScrollbackMode};

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
    let mut headless = false;
    let mut set_key: Option<(String, String)> = None;
    let mut list_keys = false;
    // `--approval <mode>`: a surface with no approval prompt (headless, or a
    // script) must say so, or a write-tier call waits forever.
    let mut approval = titi_tools::ApprovalMode::Write;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        if arg == "--mouse"
            && let Some(preset) = args.next().and_then(|v| MousePreset::parse(&v))
        {
            mouse = preset;
        } else if arg == "--headless" || arg == "-p" {
            headless = true;
        } else if arg == "--set-key" {
            match (args.next(), args.next()) {
                (Some(provider), Some(key)) => set_key = Some((provider, key)),
                _ => {
                    eprintln!("usage: titi --set-key <provider> <key>");
                    std::process::exit(2);
                }
            }
        } else if arg == "--list-keys" {
            list_keys = true;
        } else if arg == "--approval" {
            let Some(raw) = args.next() else {
                eprintln!("usage: titi --approval <always-ask|write|yolo>");
                std::process::exit(2);
            };
            match titi_cli::engine::parse_approval(&raw) {
                Ok(mode) => approval = mode,
                Err(reason) => {
                    eprintln!("{reason}");
                    std::process::exit(2);
                }
            }
        }
    }

    // Credential administration runs without starting the engine.
    if let Some((provider, key)) = set_key {
        return match titi_cli::secrets::store_key(&titi_config::agent_dir(), &provider, &key) {
            Ok(()) => {
                eprintln!("stored an API key for {provider}");
                Ok(())
            }
            Err(reason) => {
                eprintln!("not stored: {reason}");
                std::process::exit(1);
            }
        };
    }
    if list_keys {
        return match titi_cli::secrets::list_keys(&titi_config::agent_dir()) {
            Ok(keys) if keys.is_empty() => {
                eprintln!("no stored keys");
                Ok(())
            }
            Ok(keys) => {
                for key in keys {
                    eprintln!("{}  ({})", key.provider, key.kind);
                }
                Ok(())
            }
            Err(reason) => {
                eprintln!("could not read keys: {reason}");
                std::process::exit(1);
            }
        };
    }

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(io::Error::other)?;
    let _enter = runtime.enter();

    let ready = Arc::new(AtomicBool::new(false));
    {
        let ready = Arc::clone(&ready);
        std::thread::spawn(move || init_provider(ready));
    }
    let (mut engine, models, session_id) =
        titi_cli::engine::start_engine_with(approval).map_err(io::Error::other)?;
    // The transcript lives in the session store; without this the session file
    // stays empty and a resume replays nothing. Both surfaces share it.
    let session_log =
        titi_cli::session_log::SessionLog::open(&titi_config::agent_dir(), &session_id);
    if session_log.is_none() {
        eprintln!("session: transcript writes are off (store unavailable)");
    }
    if headless {
        let code = runtime.block_on(titi_cli::headless::run(engine, session_log))?;
        std::process::exit(code);
    }

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(
        stdout,
        EnterAlternateScreen,
        Hide,
        EnableBracketedPaste,
        EnableFocusChange
    )?;
    write!(stdout, "{}", mouse.enable())?;
    // OMP: Mode 2031 push + startup OSC 11 query. Replies are ProbeReply
    // bytes (`App::ingest_probe_reply`); crossterm's Event enum drops OSC,
    // so FocusGained re-queries the same way Mode 2031 would.
    write!(stdout, "{MODE_2031_ENABLE}{OSC11_QUERY}")?;

    let theme = default_theme().map_err(io::Error::other)?;
    let mut app = App::new(Arc::clone(&ready), banner(), theme);
    app.set_available_models(models);
    app.set_session_id(session_id);
    let (w, h) = size().unwrap_or((80, 24));
    app.resize(w);
    // Coding-agent default is rebuild; `PI_TUI_RESIZE_SCROLLBACK` overrides.
    let resize = Renderer::<io::Stdout>::resize_mode_from_env(ResizeScrollbackMode::Rebuild);
    let mut renderer = Renderer::new(stdout, w, h, true, resize);
    let mut input = String::new();
    paint(&mut renderer, &mut app, &input)?;

    loop {
        if event::poll(Duration::from_millis(50))? {
            match event::read()? {
                Event::Key(key) => {
                    let Some(canonical) = canonical_from_key_event(&key) else {
                        continue;
                    };
                    // Pause overlay: Esc/Enter/Space/Ctrl+C resume (OMP /pause).
                    if app.is_paused()
                        && matches!(canonical.as_str(), "escape" | "enter" | "space" | "ctrl+c")
                    {
                        app.close_overlay();
                        paint(&mut renderer, &mut app, &input)?;
                        continue;
                    }
                    if app.overlay_open() {
                        if let Some(data) = overlay_key_data(&key) {
                            if let Some(outcome) = app.overlay_input(&data) {
                                handle_outcome(&mut engine, &mut app, &mut input, outcome);
                            }
                            paint(&mut renderer, &mut app, &input)?;
                        }
                        continue;
                    }
                    match app.handle_canonical(&canonical, &mut input) {
                        Dispatch::Exit => {
                            let _ = engine.try_send(EngineCommand::Shutdown);
                            break;
                        }
                        Dispatch::Unhandled => {}
                        Dispatch::Handled(effect) => {
                            if let Some(effect) = effect {
                                apply_effect(
                                    &mut mouse,
                                    &mut renderer,
                                    &mut engine,
                                    &mut app,
                                    &mut input,
                                    effect,
                                )?;
                            } else {
                                paint(&mut renderer, &mut app, &input)?;
                            }
                        }
                    }
                }
                Event::Resize(w, h) => {
                    app.set_size(w, h);
                    renderer.on_resize(
                        w,
                        h,
                        &mut AppFrame {
                            app: &mut app,
                            input: &input,
                        },
                    )?;
                }
                Event::Mouse(mouse_event) => {
                    handle_mouse(&mut app, mouse_event);
                    paint(&mut renderer, &mut app, &input)?;
                }
                Event::Paste(text) if !app.overlay_open() => {
                    // One paste = one event: the whole block is inserted
                    // into the buffer, never executed line-by-line.
                    let appended = app.paste(&text);
                    input.push_str(&appended);
                    paint(&mut renderer, &mut app, &input)?;
                }
                Event::FocusGained => {
                    // Mode 2031 analogue under crossterm: re-query OSC 11.
                    let out = renderer.out_mut();
                    write!(out, "{OSC11_QUERY}")?;
                    out.flush()?;
                }
                _ => {}
            }
        }

        if app.poll_space_hold(Instant::now()) {
            paint(&mut renderer, &mut app, &input)?;
        }

        let flushed = app.flush_queued(&ready);
        for prompt in flushed {
            let _ = engine.try_send(EngineCommand::SubmitPrompt {
                text: prompt.into(),
            });
        }
        let mut events = false;
        while let Ok(event) = engine.try_recv() {
            app.ingest_engine_event(event);
            events = true;
        }
        if events {
            paint(&mut renderer, &mut app, &input)?;
        }
        persist_session(&mut app, session_log.as_ref());
    }

    let stdout = renderer.out_mut();
    execute!(
        stdout,
        Show,
        LeaveAlternateScreen,
        DisableBracketedPaste,
        DisableFocusChange
    )?;
    write!(stdout, "{MODE_2031_DISABLE}")?;
    write!(stdout, "{}", mouse.disable())?;
    disable_raw_mode()?;
    Ok(())
}

/// Appends everything the App has queued for the transcript. A failed append
/// is reported once per batch and never stops the CLI: losing a line of
/// history is better than dropping the session.
fn persist_session(app: &mut App, log: Option<&titi_cli::session_log::SessionLog>) {
    let writes = app.drain_session_writes();
    if writes.is_empty() {
        return;
    }
    let Some(log) = log else {
        return;
    };
    for (role, text) in writes {
        let result = match role {
            titi_core::session::Role::User => log.user(&text),
            titi_core::session::Role::Assistant => log.assistant(&text),
            titi_core::session::Role::System => log.system(&text),
        };
        if let Err(reason) = result {
            eprintln!("session: not saved ({reason})");
        }
    }
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

/// Act on a closed overlay panel's outcome.  An approval Yes is the only
/// path that deletes; Esc / No / Cancel never do.
fn handle_outcome(
    engine: &mut titi_engine::Engine,
    app: &mut App,
    input: &mut String,
    outcome: OverlayOutcome,
) {
    match outcome {
        OverlayOutcome::ModelSelected(model) => {
            app.apply_model(&model);
            let _ = engine.try_send(EngineCommand::SwitchModel {
                model: model.into(),
            });
        }
        OverlayOutcome::HistoryPicked(text) => {
            *input = text;
        }
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
        OverlayOutcome::Approval(false) => {
            let _ = app.take_pending_close();
            eprintln!("approval: declined (nothing deleted)");
        }
        OverlayOutcome::ToolApproval { call_id, approved } => {
            let _ = engine.try_send(EngineCommand::ApproveTool {
                call_id: call_id.into(),
                approved,
            });
        }
        OverlayOutcome::Dismissed => {}
        OverlayOutcome::HubSelected(id) => {
            let _ = engine.try_send(EngineCommand::FocusAgent {
                agent_id: id.into(),
            });
        }
        OverlayOutcome::HubRevive(id) => {
            let _ = engine.try_send(EngineCommand::ReviveAgent {
                agent_id: id.into(),
            });
        }
        OverlayOutcome::HubStop(id) => {
            let _ = engine.try_send(EngineCommand::StopAgent {
                agent_id: id.into(),
            });
        }
    }
}

fn apply_effect(
    mouse: &mut MousePreset,
    renderer: &mut Renderer<io::Stdout>,
    engine: &mut titi_engine::Engine,
    app: &mut App,
    input: &mut String,
    effect: SubmitEffect,
) -> io::Result<()> {
    match effect {
        SubmitEffect::DisplayReset => {
            app.request_history_replay();
            let mut provider = AppFrame { app, input };
            renderer.reset_display(&mut provider)?;
        }
        SubmitEffect::ExternalEditor => {
            let stdout = renderer.out_mut();
            execute!(stdout, Show, LeaveAlternateScreen)?;
            disable_raw_mode()?;
            *input = run_external_editor(input);
            enable_raw_mode()?;
            execute!(stdout, EnterAlternateScreen, Hide)?;
            renderer.set_size(renderer.width(), renderer.height());
            paint(renderer, app, input)?;
        }
        other => {
            apply_submit_effect(engine, mouse, renderer.out_mut(), other)?;
            paint(renderer, app, input)?;
        }
    }
    Ok(())
}

fn apply_submit_effect(
    engine: &mut titi_engine::Engine,
    mouse: &mut MousePreset,
    stdout: &mut impl Write,
    effect: SubmitEffect,
) -> io::Result<()> {
    match effect {
        SubmitEffect::None => {}
        SubmitEffect::MouseToggle => {
            let next = cycle_mouse(*mouse);
            apply_submit_effect(engine, mouse, stdout, SubmitEffect::Mouse(next))?;
        }
        SubmitEffect::Mouse(next) => {
            write!(stdout, "{}", mouse.disable())?;
            *mouse = next;
            write!(stdout, "{}", mouse.enable())?;
            match save_mouse_preset(*mouse) {
                Ok(()) => eprintln!("mouse preset: {} (saved)", mouse.name()),
                Err(reason) => {
                    eprintln!("mouse preset: {} (not saved: {reason})", mouse.name())
                }
            }
        }
        SubmitEffect::Queued(prompt) | SubmitEffect::Delivered(prompt) => {
            let _ = engine.try_send(EngineCommand::SubmitPrompt {
                text: prompt.into(),
            });
        }
        SubmitEffect::Steer(prompt) => {
            let _ = engine.try_send(EngineCommand::Steer {
                text: prompt.into(),
            });
        }
        SubmitEffect::Copy(text) => {
            write!(stdout, "{}", osc52_copy(&text))?;
            stdout.flush()?;
        }
        SubmitEffect::DisplayReset | SubmitEffect::ExternalEditor => {}
    }
    Ok(())
}

fn cycle_mouse(current: MousePreset) -> MousePreset {
    match current {
        MousePreset::Off => MousePreset::Wheel,
        MousePreset::Wheel => MousePreset::Buttons,
        MousePreset::Buttons => MousePreset::All,
        MousePreset::All => MousePreset::Off,
    }
}

/// Viewport-diff paint via [`Renderer`] — never `Clear(All)`.
fn paint(renderer: &mut Renderer<io::Stdout>, app: &mut App, input: &str) -> io::Result<()> {
    let mut provider = AppFrame { app, input };
    let plan = provider.plan((renderer.width(), renderer.height()));
    if let Some(ack) = renderer.draw(plan)? {
        provider.acknowledge(ack.id);
    }
    Ok(())
}

struct AppFrame<'a> {
    app: &'a mut App,
    input: &'a str,
}

impl FrameProvider for AppFrame<'_> {
    fn plan(&mut self, size: (u16, u16)) -> FramePlan {
        self.app.plan_frame(self.input, size.1)
    }

    fn acknowledge(&mut self, id: u64) {
        self.app.acknowledge_history(id);
    }
}

/// `$VISUAL` / `$EDITOR` (fallback `vi`) on a temp file; returns the
/// edited draft, or the original text if the editor fails.
fn run_external_editor(draft: &str) -> String {
    let editor = std::env::var("VISUAL")
        .or_else(|_| std::env::var("EDITOR"))
        .unwrap_or_else(|_| "vi".to_owned());
    let path = std::env::temp_dir().join(format!("titi-draft-{}.txt", std::process::id()));
    if std::fs::write(&path, draft).is_err() {
        return draft.to_owned();
    }
    let status = std::process::Command::new(&editor).arg(&path).status();
    let text = std::fs::read_to_string(&path).unwrap_or_else(|_| draft.to_owned());
    let _ = std::fs::remove_file(&path);
    match status {
        Ok(s) if s.success() => text,
        _ => draft.to_owned(),
    }
}
