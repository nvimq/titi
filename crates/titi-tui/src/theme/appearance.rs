//! Terminal dark/light appearance detection.
//!
//! Mirrors omp `coding-agent/src/modes/theme/theme.ts` `detectTerminalBackground`
//! and pi-tui OSC 11 luminance classification (`terminal.ts` `#handleOsc11Response`).
//!
//! Order:
//! 1. OSC 11 (unless Zellij-on-macOS, where passthrough is broken)
//! 2. `COLORFGBG` (`fg;bg`, `bg < 8` → dark)
//! 3. host macOS appearance, only for Zellij-on-macOS
//! 4. dark
//!
//! Product mapping (titi, not Hermes): auto-dark → `titanium`, auto-light → `light`.

/// Terminal / OS appearance slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Appearance {
    Dark,
    Light,
}

/// Injectable inputs for [`detect_terminal_background`].
///
/// `from_env` reads process environment; tests pass explicit values so the
/// decision table does not depend on the host terminal.
#[derive(Debug, Clone, Default)]
pub struct AppearanceInputs {
    /// Classified OSC 11 appearance, if a reply has been parsed.
    pub osc11_appearance: Option<Appearance>,
    /// `COLORFGBG` value (`fg;bg`), if set.
    pub colorfgbg: Option<String>,
    /// `std::env::consts::OS` (`"darwin"`, `"linux"`, …).
    pub platform: String,
    /// Whether `$ZELLIJ` is set.
    pub zellij: bool,
    /// Host macOS appearance, injected so tests never spawn `defaults`.
    pub macos_appearance: Option<Appearance>,
}

impl AppearanceInputs {
    /// Read detection inputs from the process environment.
    ///
    /// OSC 11 is not queried here (first paint matches omp `initThemeSync`:
    /// COLORFGBG, then a later OSC 11 report updates auto-theme). On
    /// Zellij-on-macOS, `defaults read -g AppleInterfaceStyle` is consulted.
    pub fn from_env() -> Self {
        let zellij = std::env::var_os("ZELLIJ").is_some();
        let platform = std::env::consts::OS.to_string();
        let macos_appearance = if platform == "darwin" && zellij {
            detect_macos_appearance()
        } else {
            None
        };
        Self {
            osc11_appearance: None,
            colorfgbg: std::env::var("COLORFGBG").ok().filter(|s| !s.is_empty()),
            platform,
            zellij,
            macos_appearance,
        }
    }
}

/// True when OSC 11 cannot be trusted (Zellij on macOS).
pub fn should_use_macos_appearance_fallback(platform: &str, zellij: bool) -> bool {
    platform == "darwin" && zellij
}

/// Classify BT.601 luma of an sRGB triple. `< 0.5` is dark (pi-tui OSC 11).
pub fn appearance_from_rgb(r: u8, g: u8, b: u8) -> Appearance {
    let luma = (0.299 * f64::from(r) + 0.587 * f64::from(g) + 0.114 * f64::from(b)) / 255.0;
    if luma < 0.5 {
        Appearance::Dark
    } else {
        Appearance::Light
    }
}

/// Normalize an XParseColor hex component (`0`–`ffff`, 1–4 digits) to `0.0..=1.0`.
pub fn normalize_xparse_color(hex: &str) -> f64 {
    if hex.is_empty() || hex.len() > 4 {
        return 0.0;
    }
    let value = u32::from_str_radix(hex, 16).unwrap_or(0);
    let max = 16u32.saturating_pow(hex.len() as u32).saturating_sub(1);
    if max == 0 {
        0.0
    } else {
        f64::from(value) / f64::from(max)
    }
}

/// Classify OSC 11 rgb/rgba hex components with BT.601 luma (`< 0.5` → dark).
pub fn appearance_from_osc11_hex(r: &str, g: &str, b: &str) -> Appearance {
    let luma = 0.299 * normalize_xparse_color(r)
        + 0.587 * normalize_xparse_color(g)
        + 0.114 * normalize_xparse_color(b);
    if luma < 0.5 {
        Appearance::Dark
    } else {
        Appearance::Light
    }
}

/// Parse an OSC 11 reply into the three XParseColor hex components.
///
/// Accepts `OSC 11 ; rgb:R/G/B` and `rgba:R/G/B`, ST or BEL terminated.
pub fn parse_osc11_reply(bytes: &[u8]) -> Option<(String, String, String)> {
    let s = std::str::from_utf8(bytes).ok()?;
    let body = s.strip_prefix("\x1b]11;")?;
    let spec = body
        .strip_suffix("\x1b\\")
        .or_else(|| body.strip_suffix('\x07'))?;
    let spec = spec
        .strip_prefix("rgba:")
        .or_else(|| spec.strip_prefix("rgb:"))?;
    let mut parts = spec.split('/');
    let r = parts.next()?;
    let g = parts.next()?;
    let b = parts.next()?;
    if parts.next().is_some() {
        return None;
    }
    if !is_xparse_hex(r) || !is_xparse_hex(g) || !is_xparse_hex(b) {
        return None;
    }
    Some((r.to_string(), g.to_string(), b.to_string()))
}

fn is_xparse_hex(s: &str) -> bool {
    let n = s.len();
    (1..=4).contains(&n) && s.bytes().all(|b| b.is_ascii_hexdigit())
}

/// Classified terminal appearance report.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppearanceEvent {
    /// OSC 11 classified via BT.601 luma — this is the appearance source.
    Osc11(Appearance),
    /// Mode 2031 DSR (`CSI ? 997 ; 1/2 n`). Re-query OSC 11; do not use as luma.
    Mode2031Requery,
}

/// Classify a probe-reply / stdin chunk as an appearance event.
pub fn classify_appearance_bytes(bytes: &[u8]) -> Option<AppearanceEvent> {
    if parse_mode_2031_dsr(bytes).is_some() {
        return Some(AppearanceEvent::Mode2031Requery);
    }
    let (r, g, b) = parse_osc11_reply(bytes)?;
    Some(AppearanceEvent::Osc11(appearance_from_osc11_hex(&r, &g, &b)))
}

/// Parse a Mode 2031 DSR (`CSI ? 997 ; 1/2 n`). OMP uses this as a re-query
/// trigger, not as the appearance value itself.
pub fn parse_mode_2031_dsr(bytes: &[u8]) -> Option<Appearance> {
    let s = std::str::from_utf8(bytes).ok()?;
    match s {
        "\x1b[?997;1n" => Some(Appearance::Dark),
        "\x1b[?997;2n" => Some(Appearance::Light),
        _ => None,
    }
}

/// Parse `COLORFGBG` (`fg;bg`). `bg < 8` → dark, else light.
pub fn parse_colorfgbg(value: &str) -> Option<Appearance> {
    let mut parts = value.split(';');
    let _fg = parts.next()?;
    let bg = parts.next()?;
    let bg: i32 = bg.parse().ok()?;
    Some(if bg < 8 {
        Appearance::Dark
    } else {
        Appearance::Light
    })
}

/// Host macOS appearance via `defaults read -g AppleInterfaceStyle`.
///
/// Light mode typically omits the key (`defaults` fails) → light. No new
/// native crate — this is the std-process stand-in for `@oh-my-pi/pi-natives`.
pub fn detect_macos_appearance() -> Option<Appearance> {
    let output = std::process::Command::new("defaults")
        .args(["read", "-g", "AppleInterfaceStyle"])
        .output()
        .ok()?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    if stdout.trim().eq_ignore_ascii_case("Dark") {
        Some(Appearance::Dark)
    } else {
        Some(Appearance::Light)
    }
}

/// omp `detectTerminalBackground` decision table.
pub fn detect_terminal_background(inputs: &AppearanceInputs) -> Appearance {
    let zellij_macos = should_use_macos_appearance_fallback(&inputs.platform, inputs.zellij);

    if !zellij_macos {
        if let Some(slot) = inputs.osc11_appearance {
            return slot;
        }
    }

    if let Some(value) = inputs.colorfgbg.as_deref() {
        if let Some(slot) = parse_colorfgbg(value) {
            return slot;
        }
    }

    if zellij_macos {
        if let Some(slot) = inputs.macos_appearance {
            return slot;
        }
    }

    Appearance::Dark
}

/// Default auto-dark theme name (titi product dark slot).
pub const AUTO_DARK_THEME: &str = "titanium";
/// Default auto-light theme name.
pub const AUTO_LIGHT_THEME: &str = "light";

/// Map a detected appearance onto the auto dark/light theme names.
pub fn resolve_auto_theme(dark: &str, light: &str, inputs: &AppearanceInputs) -> String {
    match detect_terminal_background(inputs) {
        Appearance::Light => light.to_string(),
        Appearance::Dark => dark.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inputs() -> AppearanceInputs {
        AppearanceInputs {
            platform: "linux".into(),
            ..AppearanceInputs::default()
        }
    }

    #[test]
    fn osc11_luma_threshold() {
        assert_eq!(appearance_from_rgb(0, 0, 0), Appearance::Dark);
        assert_eq!(appearance_from_rgb(255, 255, 255), Appearance::Light);
        // BT.601 of #7f7f7f is ~0.498 → dark; #808080 is ~0.502 → light.
        assert_eq!(appearance_from_rgb(0x7f, 0x7f, 0x7f), Appearance::Dark);
        assert_eq!(appearance_from_rgb(0x80, 0x80, 0x80), Appearance::Light);
    }

    #[test]
    fn osc11_hex_normalizes_1_to_4_digits() {
        assert_eq!(
            appearance_from_osc11_hex("0000", "0000", "0000"),
            Appearance::Dark
        );
        assert_eq!(
            appearance_from_osc11_hex("ffff", "ffff", "ffff"),
            Appearance::Light
        );
        assert_eq!(
            appearance_from_osc11_hex("f", "f", "f"),
            Appearance::Light
        );
        assert_eq!(
            appearance_from_osc11_hex("0", "0", "0"),
            Appearance::Dark
        );
    }

    #[test]
    fn osc11_reply_parse_rgb_and_rgba() {
        let st = b"\x1b]11;rgb:0000/0000/0000\x1b\\";
        assert_eq!(
            parse_osc11_reply(st),
            Some(("0000".into(), "0000".into(), "0000".into()))
        );
        let bel = b"\x1b]11;rgba:ffff/ffff/ffff\x07";
        assert_eq!(
            parse_osc11_reply(bel),
            Some(("ffff".into(), "ffff".into(), "ffff".into()))
        );
        assert!(parse_osc11_reply(b"not-osc").is_none());
    }

    #[test]
    fn colorfgbg_bg_lt_8_is_dark() {
        assert_eq!(parse_colorfgbg("15;0"), Some(Appearance::Dark));
        assert_eq!(parse_colorfgbg("0;7"), Some(Appearance::Dark));
        assert_eq!(parse_colorfgbg("0;8"), Some(Appearance::Light));
        assert_eq!(parse_colorfgbg("0;15"), Some(Appearance::Light));
        assert_eq!(parse_colorfgbg("0"), None);
        assert_eq!(parse_colorfgbg("0;x"), None);
        assert_eq!(parse_colorfgbg(""), None);
    }

    #[test]
    fn chain_prefers_osc11_over_colorfgbg() {
        let mut i = inputs();
        i.osc11_appearance = Some(Appearance::Light);
        i.colorfgbg = Some("15;0".into());
        assert_eq!(detect_terminal_background(&i), Appearance::Light);
    }

    #[test]
    fn chain_colorfgbg_when_no_osc11() {
        let mut i = inputs();
        i.colorfgbg = Some("0;15".into());
        assert_eq!(detect_terminal_background(&i), Appearance::Light);
        i.colorfgbg = Some("15;0".into());
        assert_eq!(detect_terminal_background(&i), Appearance::Dark);
    }

    #[test]
    fn zellij_macos_ignores_osc11() {
        let mut i = AppearanceInputs {
            platform: "darwin".into(),
            zellij: true,
            osc11_appearance: Some(Appearance::Light),
            colorfgbg: Some("15;0".into()),
            macos_appearance: Some(Appearance::Light),
            ..AppearanceInputs::default()
        };
        assert_eq!(detect_terminal_background(&i), Appearance::Dark);
        i.colorfgbg = None;
        assert_eq!(detect_terminal_background(&i), Appearance::Light);
        i.macos_appearance = None;
        assert_eq!(detect_terminal_background(&i), Appearance::Dark);
    }

    #[test]
    fn zellij_linux_does_not_use_macos_fallback() {
        let i = AppearanceInputs {
            platform: "linux".into(),
            zellij: true,
            osc11_appearance: Some(Appearance::Light),
            macos_appearance: Some(Appearance::Dark),
            ..AppearanceInputs::default()
        };
        assert_eq!(detect_terminal_background(&i), Appearance::Light);
    }

    #[test]
    fn empty_inputs_default_dark() {
        assert_eq!(detect_terminal_background(&inputs()), Appearance::Dark);
    }

    #[test]
    fn resolve_maps_titi_slots() {
        let mut i = inputs();
        i.colorfgbg = Some("0;15".into());
        assert_eq!(
            resolve_auto_theme(AUTO_DARK_THEME, AUTO_LIGHT_THEME, &i),
            "light"
        );
        i.colorfgbg = Some("15;0".into());
        assert_eq!(
            resolve_auto_theme(AUTO_DARK_THEME, AUTO_LIGHT_THEME, &i),
            "titanium"
        );
    }

    #[test]
    fn mode_2031_dsr() {
        assert_eq!(
            parse_mode_2031_dsr(b"\x1b[?997;1n"),
            Some(Appearance::Dark)
        );
        assert_eq!(
            parse_mode_2031_dsr(b"\x1b[?997;2n"),
            Some(Appearance::Light)
        );
        assert_eq!(parse_mode_2031_dsr(b"\x1b[?997;3n"), None);
    }

    #[test]
    fn classify_osc11_and_mode_2031() {
        assert_eq!(
            classify_appearance_bytes(b"\x1b]11;rgb:0000/0000/0000\x1b\\"),
            Some(AppearanceEvent::Osc11(Appearance::Dark))
        );
        assert_eq!(
            classify_appearance_bytes(b"\x1b]11;rgb:ffff/ffff/ffff\x07"),
            Some(AppearanceEvent::Osc11(Appearance::Light))
        );
        assert_eq!(
            classify_appearance_bytes(b"\x1b[?997;1n"),
            Some(AppearanceEvent::Mode2031Requery)
        );
        assert_eq!(
            classify_appearance_bytes(b"\x1b[?997;2n"),
            Some(AppearanceEvent::Mode2031Requery)
        );
        assert!(classify_appearance_bytes(b"\x1b[A").is_none());
    }
}
