//! Snapshot test for `SlashRegistry::route` + `complete`.
//!
//! Contract: `docs/research/agent-ux/README.md` — "снапшот-тест
//! SlashRegistry::route + complete" with floating autocomplete showing
//! descriptions.

use titi_tui::slash::{Route, SlashRegistry};

/// A registry with a representative set of builtins and file commands,
/// mirroring the omp/Hermes command surface.
fn sample_registry() -> SlashRegistry {
    let mut reg = SlashRegistry::new();
    for (name, desc) in [
        ("help", "Show help and available commands"),
        ("model", "Switch the active model"),
        ("sessions", "Open the session switcher"),
        ("mouse", "Configure mouse tracking (off|wheel|buttons|all)"),
        ("details", "Toggle transcript section visibility"),
        ("pause", "Pause the agent"),
        ("skin", "Change the visual theme with live preview"),
    ] {
        reg.register_builtin(name, desc);
    }
    reg.register_file("translate", "Translate the following to English:\n$ARGUMENTS");
    reg.register_file("say", "Respond with: $1");
    reg
}

fn render_routes(reg: &SlashRegistry, inputs: &[&str]) -> Vec<String> {
    inputs
        .iter()
        .map(|input| {
            let route = reg.route(input);
            let text = match &route {
                Route::Builtin(name) => format!("Builtin({name})"),
                Route::Expanded(text) => format!("Expanded({text:?})"),
                Route::Passthrough => "Passthrough".to_owned(),
            };
            format!("{input: <30} -> {text}")
        })
        .collect()
}

fn render_completions(reg: &SlashRegistry, prefix: &str) -> Vec<String> {
    reg.complete(prefix)
        .into_iter()
        .map(|c| {
            if c.description.is_empty() {
                format!("{}/", c.name)
            } else {
                format!("{:<24} {}", c.name, c.description)
            }
        })
        .collect()
}

#[test]
fn route_snapshot() {
    let reg = sample_registry();
    let inputs = [
        "/help",
        "/help verbose",
        "/model claude",
        "/sessions",
        "/unknown-xyz",
        "/translate hello world",
        "/say hello world",
        "/skin dark",
        "/details thinking expanded",
        "no slash here",
        "/",
    ];
    let rendered = render_routes(&reg, &inputs).join("\n");
    let expected = "\
/help                          -> Builtin(help)
/help verbose                  -> Builtin(help)
/model claude                  -> Builtin(model)
/sessions                      -> Builtin(sessions)
/unknown-xyz                   -> Passthrough
/translate hello world         -> Expanded(\"Translate the following to English:\\nhello world\")
/say hello world               -> Expanded(\"Respond with: hello\")
/skin dark                     -> Builtin(skin)
/details thinking expanded     -> Builtin(details)
no slash here                  -> Passthrough
/                              -> Passthrough";
    assert_eq!(
        rendered, expected,
        "route snapshot mismatch. Update expected if intentional.\n--- got ---\n{rendered}\n--- want ---\n{expected}"
    );
}

#[test]
fn completion_snapshot() {
    let reg = sample_registry();
    let prefixes = ["/h", "/s", "/m", "/", "/det"];
    let rendered = prefixes
        .iter()
        .map(|p| {
            let completions = render_completions(&reg, p);
            format!("[{p}]\n{}", completions.join("\n"))
        })
        .collect::<Vec<_>>()
        .join("\n\n");

    let expected = "\
[/h]
/help                    Show help and available commands

[/s]
/sessions                Open the session switcher
/skin                    Change the visual theme with live preview
/say/

[/m]
/model                   Switch the active model
/mouse                   Configure mouse tracking (off|wheel|buttons|all)

[/]
/help                    Show help and available commands
/model                   Switch the active model
/sessions                Open the session switcher
/mouse                   Configure mouse tracking (off|wheel|buttons|all)
/details                 Toggle transcript section visibility
/pause                   Pause the agent
/skin                    Change the visual theme with live preview
/translate/
/say/

[/det]
/details                 Toggle transcript section visibility";
    assert_eq!(
        rendered, expected,
        "completion snapshot mismatch. Update expected if intentional.\n--- got ---\n{rendered}\n--- want ---\n{expected}"
    );
}
