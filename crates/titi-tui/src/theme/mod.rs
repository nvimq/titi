//! Theme system — the [`Theme`] struct, symbol/color accessors, and the
//! process-wide [`GlobalTheme`] handle.
//!
//! Ported from omp `coding-agent/src/modes/theme/`:
//! - `theme-class.ts` → [`Theme`]
//! - `theme.ts` → [`GlobalTheme`] (file watcher still deferred)
//! - `appearance.rs` → OSC 11 / COLORFGBG / macOS-Zellij auto-theme
//! - `loader.ts` → [`loader`]
//! - `schema.ts` → [`schema`]
//! - `color.ts` + `pi-utils/color.ts` → [`color`]
//! - `symbols.ts` → [`symbols`]

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, LazyLock, RwLock};

use serde_json::Value;

pub mod appearance;
pub mod builtin;
pub mod color;
pub mod loader;
pub mod schema;
pub mod symbols;
mod symbols_data;

pub use appearance::{
    AUTO_DARK_THEME, AUTO_LIGHT_THEME, Appearance, AppearanceEvent, AppearanceInputs,
    appearance_from_rgb, classify_appearance_bytes,
};
pub use color::ColorMode;
pub use schema::{ThemeBg, ThemeColor};
pub use symbols::{SpinnerFrames, SymbolPreset};

// ---- ANSI escape constants ------------------------------------------------

/// Reset sequence for the foreground color only.
const FG_RESET: &str = "\x1b[39m";
/// Reset sequence for the background color only.
const BG_RESET: &str = "\x1b[49m";

/// Foreground hex used when a token resolves to terminal default on a dark theme.
const DARK_DEFAULT_FG: &str = "#e5e5e7";
/// Foreground hex used when a token resolves to terminal default on a light theme.
const LIGHT_DEFAULT_FG: &str = "#000000";
/// Background hex used when a token resolves to terminal default on a dark theme.
const DARK_DEFAULT_BG: &str = "#000000";
/// Background hex used when a token resolves to terminal default on a light theme.
const LIGHT_DEFAULT_BG: &str = "#ffffff";

/// HSV shift applied to `toolDiffAdded` for colorblind mode.
const COLORBLIND_HUE_SHIFT: f64 = 60.0;
const COLORBLIND_SAT_MUL: f64 = 0.71;

/// Language alias → resolved symbol key.  Mirrors omp `langMap`.
const LANG_MAP: &[(&str, &str)] = &[
    ("typescript", "lang.typescript"),
    ("ts", "lang.typescript"),
    ("tsx", "lang.typescript"),
    ("javascript", "lang.javascript"),
    ("js", "lang.javascript"),
    ("jsx", "lang.javascript"),
    ("mjs", "lang.javascript"),
    ("cjs", "lang.javascript"),
    ("python", "lang.python"),
    ("py", "lang.python"),
    ("rust", "lang.rust"),
    ("rs", "lang.rust"),
    ("go", "lang.go"),
    ("java", "lang.java"),
    ("c", "lang.c"),
    ("cpp", "lang.cpp"),
    ("c++", "lang.cpp"),
    ("cc", "lang.cpp"),
    ("cxx", "lang.cpp"),
    ("csharp", "lang.csharp"),
    ("cs", "lang.csharp"),
    ("ruby", "lang.ruby"),
    ("rb", "lang.ruby"),
    ("julia", "lang.julia"),
    ("jl", "lang.julia"),
    ("php", "lang.php"),
    ("swift", "lang.swift"),
    ("kotlin", "lang.kotlin"),
    ("kt", "lang.kotlin"),
    ("bash", "lang.shell"),
    ("sh", "lang.shell"),
    ("zsh", "lang.shell"),
    ("fish", "lang.shell"),
    ("powershell", "lang.shell"),
    ("just", "lang.shell"),
    ("shell", "lang.shell"),
    ("html", "lang.html"),
    ("htm", "lang.html"),
    ("astro", "lang.html"),
    ("vue", "lang.html"),
    ("svelte", "lang.html"),
    ("css", "lang.css"),
    ("scss", "lang.css"),
    ("sass", "lang.css"),
    ("less", "lang.css"),
    ("json", "lang.json"),
    ("yaml", "lang.yaml"),
    ("yml", "lang.yaml"),
    ("markdown", "lang.markdown"),
    ("md", "lang.markdown"),
    ("sql", "lang.sql"),
    ("dockerfile", "lang.docker"),
    ("docker", "lang.docker"),
    ("lua", "lang.lua"),
    ("text", "lang.text"),
    ("txt", "lang.text"),
    ("plain", "lang.text"),
    ("log", "lang.log"),
    ("env", "lang.env"),
    ("dotenv", "lang.env"),
    ("toml", "lang.toml"),
    ("xml", "lang.xml"),
    ("ini", "lang.ini"),
    ("conf", "lang.conf"),
    ("cfg", "lang.conf"),
    ("config", "lang.conf"),
    ("properties", "lang.conf"),
    ("csv", "lang.csv"),
    ("tsv", "lang.tsv"),
    ("image", "lang.image"),
    ("img", "lang.image"),
    ("png", "lang.image"),
    ("jpg", "lang.image"),
    ("jpeg", "lang.image"),
    ("gif", "lang.image"),
    ("webp", "lang.image"),
    ("svg", "lang.image"),
    ("ico", "lang.image"),
    ("bmp", "lang.image"),
    ("tiff", "lang.image"),
    ("pdf", "lang.pdf"),
    ("zip", "lang.archive"),
    ("tar", "lang.archive"),
    ("gz", "lang.archive"),
    ("tgz", "lang.archive"),
    ("bz2", "lang.archive"),
    ("xz", "lang.archive"),
    ("7z", "lang.archive"),
    ("exe", "lang.binary"),
    ("dll", "lang.binary"),
    ("so", "lang.binary"),
    ("dylib", "lang.binary"),
    ("wasm", "lang.binary"),
    ("bin", "lang.binary"),
];

/// Brand colors for language icons, keyed by resolved `lang.*` symbol key.
const LANG_BRAND_COLORS: &[(&str, &str)] = &[
    ("lang.javascript", "#f7df1e"),
    ("lang.python", "#3776ab"),
    ("lang.ruby", "#cc342d"),
    ("lang.julia", "#9558b2"),
];

// ============================================================================
// Theme struct
// ============================================================================

/// A fully-resolved theme: ANSI escapes per color token, resolved hex values,
/// the active symbol map, and spinner frames.
///
/// Constructed via [`loader::create_theme`] / [`loader::load_theme`], or
/// directly with [`Theme::new`].
#[derive(Debug, Clone)]
pub struct Theme {
    /// Pre-rendered foreground ANSI escapes, keyed by camelCase token name.
    fg: HashMap<String, String>,
    /// Pre-rendered background ANSI escapes, keyed by camelCase token name.
    bg: HashMap<String, String>,
    /// Resolved CSS hex for each foreground token (empty = terminal default).
    hex_fg: HashMap<String, String>,
    /// Resolved CSS hex for each background token (empty = terminal default).
    hex_bg: HashMap<String, String>,
    /// Active color depth.
    mode: ColorMode,
    /// Active symbol preset.
    symbol_preset: SymbolPreset,
    /// Resolved symbol map (preset + overrides).
    symbol_map: HashMap<String, String>,
    /// Spinner frames for the `status` type.
    spinner_status: Vec<String>,
    /// Spinner frames for the `activity` type.
    spinner_activity: Vec<String>,
    /// Perceptual luma (0..1) of the status-line background.
    status_line_luminance: Option<f64>,
    /// WCAG relative luminance of the status-line background.
    status_line_contrast_luminance: Option<f64>,
}

impl Theme {
    /// Build a theme from resolved color maps.
    ///
    /// `fg_colors` / `bg_colors` map camelCase token names to JSON color
    /// values (hex string, 256-color index, or empty string for terminal
    /// default).  `symbol_overrides` patch the preset's symbol map; unknown
    /// keys are silently ignored (omp logs at debug).
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        name: String,
        fg_colors: HashMap<String, Value>,
        bg_colors: HashMap<String, Value>,
        mode: ColorMode,
        symbol_preset: SymbolPreset,
        symbol_overrides: HashMap<String, String>,
        spinner_status_override: Option<Vec<String>>,
        spinner_activity_override: Option<Vec<String>>,
    ) -> Result<Theme, String> {
        let _ = name;
        let status_line_value = bg_colors.get("statusLineBg");
        let status_line_luminance = status_line_value.and_then(color_luma_of);
        let status_line_contrast_luminance = status_line_value.and_then(relative_luminance_of);
        let sl_is_light = status_line_luminance.is_some_and(|l| l > 0.5);

        let mut fg = HashMap::new();
        let mut hex_fg = HashMap::new();
        for (key, value) in &fg_colors {
            if !schema::is_valid_theme_color(key) {
                continue;
            }
            fg.insert(key.clone(), color::fg_ansi_from_value(value, mode));
            hex_fg.insert(key.clone(), resolve_to_hex(value, sl_is_light));
        }

        let mut bg = HashMap::new();
        let mut hex_bg = HashMap::new();
        for (key, value) in &bg_colors {
            if !schema::is_valid_theme_bg(key) {
                continue;
            }
            bg.insert(key.clone(), color::bg_ansi_from_value(value, mode));
            hex_bg.insert(key.clone(), resolve_to_hex(value, sl_is_light));
        }

        // Build symbol map from preset, then apply overrides.
        let mut symbol_map = HashMap::new();
        for (k, v) in symbols::symbols(symbol_preset) {
            symbol_map.insert((*k).to_string(), (*v).to_string());
        }
        for (key, value) in &symbol_overrides {
            if symbol_map.contains_key(key) {
                symbol_map.insert(key.clone(), value.clone());
            }
        }

        // Resolve spinner frames: override or preset default.
        let spinner_status = spinner_status_override.unwrap_or_else(|| {
            symbols::spinner_frames(symbol_preset)
                .status
                .iter()
                .map(|s| s.to_string())
                .collect()
        });
        let spinner_activity = spinner_activity_override.unwrap_or_else(|| {
            symbols::spinner_frames(symbol_preset)
                .activity
                .iter()
                .map(|s| s.to_string())
                .collect()
        });

        Ok(Theme {
            fg,
            bg,
            hex_fg,
            hex_bg,
            mode,
            symbol_preset,
            symbol_map,
            spinner_status,
            spinner_activity,
            status_line_luminance,
            status_line_contrast_luminance,
        })
    }

    /// Apply the colorblind adjustment (shift `toolDiffAdded` green→blue),
    /// returning a new [`Theme`].  Unchanged when the token is absent or not
    /// a hex string.
    pub fn with_color_blind_mode(&self) -> Theme {
        let mut next = self.clone();
        if let Some(hex) = next.hex_fg.get("toolDiffAdded").cloned()
            && hex.starts_with('#')
            && let Some(adjusted) =
                color::adjust_hsv(&hex, COLORBLIND_HUE_SHIFT, COLORBLIND_SAT_MUL, 1.0)
        {
            let ansi =
                color::color_to_ansi(&adjusted, next.mode).unwrap_or_else(|| FG_RESET.to_string());
            next.fg.insert("toolDiffAdded".to_string(), ansi);
            next.hex_fg.insert("toolDiffAdded".to_string(), adjusted);
        }
        next
    }

    /// True when the active theme has a light status-line background.
    pub fn is_light(&self) -> bool {
        self.status_line_luminance.is_some_and(|l| l > 0.5)
    }

    /// Surface luminance to size session accents against on light themes;
    /// `None` on dark themes so accents stay vivid.
    pub fn accent_surface_luminance(&self) -> Option<f64> {
        if self.is_light() {
            self.status_line_contrast_luminance
        } else {
            None
        }
    }

    /// The active color depth.
    pub fn get_color_mode(&self) -> ColorMode {
        self.mode
    }

    /// The active symbol preset.
    pub fn get_symbol_preset(&self) -> SymbolPreset {
        self.symbol_preset
    }

    // ---- Color wrappers ---------------------------------------------------

    /// Wrap `text` in a foreground color token, resetting only the foreground.
    pub fn fg(&self, color: ThemeColor, text: &str) -> String {
        format!("{}{}{FG_RESET}", self.get_fg_ansi(color), text)
    }

    /// Wrap `text` in a background color token, resetting only the
    /// background.
    pub fn bg(&self, color: ThemeBg, text: &str) -> String {
        format!("{}{}{BG_RESET}", self.get_bg_ansi(color), text)
    }

    /// Apply a background fill that resumes after every nested reset.
    ///
    /// Composer rows contain styled text and cursor escapes; a plain wrapper
    /// would stop at the first nested reset.
    pub fn bg_fill(&self, color: ThemeBg, text: &str) -> String {
        let ansi = self.get_bg_ansi(color);
        format!("{}{}{BG_RESET}", ansi, reapply_after_bg_reset(text, &ansi))
    }

    /// Apply a foreground over a controlled background, choosing a
    /// contrast-safe fallback when the token requests the terminal default
    /// and re-applying it after every nested foreground reset.
    pub fn fg_on_bg(&self, color: ThemeColor, background: ThemeBg, text: &str) -> String {
        let ansi = self.get_fg_on_bg_ansi(color, background);
        format!("{}{}{FG_RESET}", ansi, reapply_after_fg_reset(text, &ansi))
    }

    /// The raw foreground ANSI escape for a token.
    pub fn get_fg_ansi(&self, color: ThemeColor) -> String {
        self.fg
            .get(color.as_str())
            .cloned()
            .unwrap_or_else(|| FG_RESET.to_string())
    }

    /// The raw background ANSI escape for a token.
    pub fn get_bg_ansi(&self, color: ThemeBg) -> String {
        self.bg
            .get(color.as_str())
            .cloned()
            .unwrap_or_else(|| BG_RESET.to_string())
    }

    /// Foreground ANSI for text rendered over a controlled theme background.
    /// Explicit theme colors win; terminal-default tokens become black or
    /// near-white based on the background's perceived luma.
    pub fn get_fg_on_bg_ansi(&self, color: ThemeColor, background: ThemeBg) -> String {
        let ansi = self.get_fg_ansi(color);
        if ansi != FG_RESET {
            return ansi;
        }
        let bg_ansi = self.get_bg_ansi(background);
        if bg_ansi == BG_RESET {
            return ansi;
        }
        let bg_hex = self.get_bg_hex(background);
        let pick = if color::color_luma(&bg_hex).is_some_and(|l| l > 0.5) {
            LIGHT_DEFAULT_FG
        } else {
            DARK_DEFAULT_FG
        };
        color::color_to_ansi(pick, self.mode).unwrap_or_else(|| ansi.clone())
    }

    /// Foreground ANSI for text drawn on top of `fill_color` used as a solid
    /// background (e.g. a powerline chip).  Picks near-black or near-white by
    /// the fill's perceived luma.  Falls back to the `Text` token when the
    /// fill is a 256-palette index (RGB unavailable).
    pub fn get_contrast_fg_ansi(&self, fill_color: ThemeColor) -> String {
        let ansi = self.get_fg_ansi(fill_color);
        match parse_truecolor_rgb(&ansi) {
            Some((r, g, b)) => {
                let luma = 0.299 * f64::from(r) + 0.587 * f64::from(g) + 0.114 * f64::from(b);
                if luma > 140.0 {
                    "\x1b[38;2;0;0;0m".to_string()
                } else {
                    "\x1b[38;2;255;255;255m".to_string()
                }
            }
            None => self.get_fg_ansi(ThemeColor::Text),
        }
    }

    /// Resolved CSS hex for a foreground token.  Empty resolves to the theme
    /// default.
    pub fn get_color_hex(&self, color: ThemeColor) -> String {
        match self.hex_fg.get(color.as_str()) {
            Some(hex) if !hex.is_empty() => hex.clone(),
            _ => {
                if self.is_light() {
                    LIGHT_DEFAULT_FG.to_string()
                } else {
                    DARK_DEFAULT_FG.to_string()
                }
            }
        }
    }

    /// Resolved CSS hex for a background token.  Empty resolves to the theme
    /// default.
    pub fn get_bg_hex(&self, color: ThemeBg) -> String {
        match self.hex_bg.get(color.as_str()) {
            Some(hex) if !hex.is_empty() => hex.clone(),
            _ => {
                if self.is_light() {
                    LIGHT_DEFAULT_BG.to_string()
                } else {
                    DARK_DEFAULT_BG.to_string()
                }
            }
        }
    }

    /// All foreground + background tokens as CSS hex strings, skipping
    /// tokens resolved to the terminal default.
    pub fn get_all_theme_color_hexes(&self) -> Vec<String> {
        let mut hexes = Vec::new();
        for hex in self.hex_fg.values() {
            if !hex.is_empty() {
                hexes.push(hex.clone());
            }
        }
        for hex in self.hex_bg.values() {
            if !hex.is_empty() {
                hexes.push(hex.clone());
            }
        }
        hexes
    }

    /// The most visually dominant foreground tokens as CSS hex strings.
    /// Skips terminal-default tokens.
    pub fn get_major_theme_color_hexes(&self) -> Vec<String> {
        const MAJOR_KEYS: &[&str] = &[
            "accent",
            "border",
            "borderAccent",
            "borderMuted",
            "success",
            "error",
            "warning",
            "mdHeading",
            "mdLink",
            "mdCode",
            "mdCodeBlock",
            "mdQuoteBorder",
            "mdListBullet",
            "toolDiffAdded",
            "toolDiffRemoved",
            "customMessageLabel",
            "thinkingText",
        ];
        let mut hexes = Vec::new();
        for key in MAJOR_KEYS {
            if let Some(hex) = self.hex_fg.get(*key).filter(|h| !h.is_empty()) {
                hexes.push(hex.clone());
            }
        }
        hexes
    }

    /// Resolved CSS hex for the accent color.
    pub fn get_accent_color_hex(&self) -> String {
        self.get_color_hex(ThemeColor::Accent)
    }

    // ---- Text style wrappers (SGR) ----------------------------------------

    /// Bold (`\x1b[1m` … `\x1b[22m`).
    pub fn bold(&self, text: &str) -> String {
        format!("\x1b[1m{text}\x1b[22m")
    }
    /// Italic (`\x1b[3m` … `\x1b[23m`).
    pub fn italic(&self, text: &str) -> String {
        format!("\x1b[3m{text}\x1b[23m")
    }
    /// Underline (`\x1b[4m` … `\x1b[24m`).
    pub fn underline(&self, text: &str) -> String {
        format!("\x1b[4m{text}\x1b[24m")
    }
    /// Strikethrough (`\x1b[9m` … `\x1b[29m`).
    pub fn strikethrough(&self, text: &str) -> String {
        format!("\x1b[9m{text}\x1b[29m")
    }
    /// Inverse (`\x1b[7m` … `\x1b[27m`).
    pub fn inverse(&self, text: &str) -> String {
        format!("\x1b[7m{text}\x1b[27m")
    }

    // ---- Symbols -----------------------------------------------------------

    /// Look up a symbol by key.  Returns an empty string for unknown keys.
    pub fn symbol(&self, key: &str) -> &str {
        self.symbol_map.get(key).map(String::as_str).unwrap_or("")
    }

    /// A symbol styled with a foreground color token.
    pub fn styled_symbol(&self, key: &str, color: ThemeColor) -> String {
        self.fg(color, self.symbol(key))
    }

    /// Spinner frames for a type (`"status"` or `"activity"`), honoring
    /// theme overrides then falling back to preset defaults.
    pub fn get_spinner_frames(&self, spinner_type: &str) -> &[String] {
        match spinner_type {
            "activity" => &self.spinner_activity,
            _ => &self.spinner_status,
        }
    }

    /// Default (status) spinner frames.
    pub fn spinner_frames(&self) -> &[String] {
        &self.spinner_status
    }

    /// Language icon glyph for a language name (case-insensitive alias
    /// lookup).
    pub fn get_lang_icon(&self, lang: Option<&str>) -> &str {
        match lang {
            None => self.symbol("lang.default"),
            Some(l) => {
                let normalized = l.to_lowercase();
                match LANG_MAP.iter().find(|(alias, _)| *alias == normalized) {
                    Some((_, key)) => self.symbol(key),
                    None => self.symbol("lang.default"),
                }
            }
        }
    }

    /// Language icon tinted with the language's brand color; falls back to
    /// the muted token, or the bare icon when the preset has none.
    pub fn get_lang_icon_styled(&self, lang: Option<&str>) -> String {
        let icon = self.symbol("lang.default");
        if icon.is_empty() {
            return String::new();
        }
        let key = lang.and_then(|l| {
            let normalized = l.to_lowercase();
            LANG_MAP
                .iter()
                .find(|(alias, _)| *alias == normalized)
                .map(|(_, k)| *k)
        });
        let hex = key.and_then(|k| {
            LANG_BRAND_COLORS
                .iter()
                .find(|(bk, _)| *bk == k)
                .map(|(_, v)| *v)
        });
        match hex {
            Some(h) => {
                let ansi = color::color_to_ansi(h, self.mode).unwrap_or_default();
                format!("{ansi}{icon}{FG_RESET}")
            }
            None => self.fg(ThemeColor::Muted, icon),
        }
    }

    // ---- Border color helpers ---------------------------------------------

    /// Foreground token for a thinking level's border.  `max` falls back to
    /// `xhigh` when the theme omits `thinkingMax`.
    pub fn get_thinking_border_color(&self, level: &str) -> ThemeColor {
        match level {
            "minimal" => ThemeColor::ThinkingMinimal,
            "low" => ThemeColor::ThinkingLow,
            "medium" => ThemeColor::ThinkingMedium,
            "high" => ThemeColor::ThinkingHigh,
            "xhigh" => ThemeColor::ThinkingXhigh,
            "max" => {
                if self.fg.contains_key("thinkingMax") {
                    ThemeColor::ThinkingMax
                } else {
                    ThemeColor::ThinkingXhigh
                }
            }
            _ => ThemeColor::ThinkingOff,
        }
    }

    /// Foreground token for the bash-mode border.
    pub fn get_bash_mode_border_color(&self) -> ThemeColor {
        ThemeColor::BashMode
    }

    /// Foreground token for the python-mode border.
    pub fn get_python_mode_border_color(&self) -> ThemeColor {
        ThemeColor::PythonMode
    }
}

// ============================================================================
// Value → color helpers (bridge serde_json::Value to color primitives)
// ============================================================================

/// Perceptual luma of a JSON color value (hex or 256 index).
fn color_luma_of(value: &Value) -> Option<f64> {
    match value {
        Value::String(s) => color::color_luma(s),
        Value::Number(n) => n.as_u64().and_then(|i| color::color_luma(&ansi256_hex(i))),
        _ => None,
    }
}

/// WCAG relative luminance of a JSON color value.
fn relative_luminance_of(value: &Value) -> Option<f64> {
    match value {
        Value::String(s) => color::relative_luminance(s),
        Value::Number(n) => n
            .as_u64()
            .and_then(|i| color::relative_luminance(&ansi256_hex(i))),
        _ => None,
    }
}

/// Convert a 256-color index (as u64) to a hex string.  Returns empty for
/// values > 255.
fn ansi256_hex(index: u64) -> String {
    if index > 255 {
        return String::new();
    }
    color::ansi256_to_hex(index as u8)
}

/// Resolve a JSON color value to a CSS hex string.  Empty → theme default.
fn resolve_to_hex(value: &Value, is_light: bool) -> String {
    match value {
        Value::Number(n) => n.as_u64().map(ansi256_hex).unwrap_or_default(),
        Value::String(s) => {
            if s.is_empty() {
                if is_light {
                    LIGHT_DEFAULT_FG.to_string()
                } else {
                    DARK_DEFAULT_FG.to_string()
                }
            } else {
                s.clone()
            }
        }
        _ => String::new(),
    }
}

/// Extract `(r, g, b)` from a truecolor escape `\x1b[38;2;R;G;Bm`.
fn parse_truecolor_rgb(ansi: &str) -> Option<(u8, u8, u8)> {
    let body = ansi.strip_prefix("\x1b[38;2;")?.strip_suffix('m')?;
    let mut parts = body.split(';');
    let r = parts.next()?.parse().ok()?;
    let g = parts.next()?.parse().ok()?;
    let b = parts.next()?.parse().ok()?;
    Some((r, g, b))
}

/// Reapply `ansi` after every nested full/background reset in `text`.
fn reapply_after_bg_reset(text: &str, ansi: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(idx) = find_reset(rest, true) {
        let end = idx + reset_end(&rest[idx..]);
        out.push_str(&rest[..end]);
        out.push_str(ansi);
        rest = &rest[end..];
    }
    out.push_str(rest);
    out
}

/// Reapply `ansi` after every nested full/foreground reset in `text`.
fn reapply_after_fg_reset(text: &str, ansi: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(idx) = find_reset(rest, false) {
        let end = idx + reset_end(&rest[idx..]);
        out.push_str(&rest[..end]);
        out.push_str(ansi);
        rest = &rest[end..];
    }
    out.push_str(rest);
    out
}

/// Find the next `\x1b[0m` or `\x1b[49m` (bg=true) / `\x1b[39m` (bg=false)
/// reset.
fn find_reset(text: &str, bg: bool) -> Option<usize> {
    let full = "\x1b[0m";
    let specific = if bg { "\x1b[49m" } else { "\x1b[39m" };
    let a = text.find(full);
    let b = text.find(specific);
    match (a, b) {
        (Some(x), Some(y)) => Some(x.min(y)),
        (Some(x), None) => Some(x),
        (None, Some(y)) => Some(y),
        (None, None) => None,
    }
}

/// Byte length of the reset escape starting at `text[0..]`.
fn reset_end(text: &str) -> usize {
    if text.starts_with("\x1b[0m") {
        4
    } else if text.starts_with("\x1b[49m") || text.starts_with("\x1b[39m") {
        5
    } else {
        0
    }
}

// ============================================================================
// GlobalTheme — process-wide handle
// ============================================================================

/// Monotonic counter bumped on any theme-affecting change so consumers can
/// key cached renders.  Mirrors omp `themeEpoch`.
static THEME_EPOCH: AtomicU64 = AtomicU64::new(0);

/// Auto-detection mapping (omp `autoDetectedTheme` / `autoDarkTheme` / `autoLightTheme`).
struct AutoTheme {
    enabled: bool,
    dark: String,
    light: String,
    reported: Option<appearance::Appearance>,
}

impl Default for AutoTheme {
    fn default() -> Self {
        Self {
            enabled: false,
            dark: appearance::AUTO_DARK_THEME.to_string(),
            light: appearance::AUTO_LIGHT_THEME.to_string(),
            reported: None,
        }
    }
}

/// The process-wide theme handle.
pub struct GlobalTheme {
    inner: RwLock<Option<Arc<Theme>>>,
    name: RwLock<Option<String>>,
    auto: RwLock<AutoTheme>,
}

/// The singleton global theme.
static GLOBAL: LazyLock<GlobalTheme> = LazyLock::new(GlobalTheme::new);

impl GlobalTheme {
    fn new() -> Self {
        GlobalTheme {
            inner: RwLock::new(None),
            name: RwLock::new(None),
            auto: RwLock::new(AutoTheme::default()),
        }
    }

    fn load_or_dark(name: &str) -> Theme {
        loader::load_theme(name, &loader::CreateThemeOptions::default())
            .or_else(|_| loader::load_theme("dark", &loader::CreateThemeOptions::default()))
            .expect("built-in dark theme must load")
    }

    fn commit(&self, name: &str, theme: Theme) {
        if let Ok(mut g) = self.inner.write() {
            *g = Some(Arc::new(theme));
        }
        if let Ok(mut n) = self.name.write() {
            *n = Some(name.to_string());
        }
        self.bump_epoch();
    }

    fn disable_auto(&self) {
        if let Ok(mut a) = self.auto.write() {
            a.enabled = false;
        }
    }

    /// Initialize the global theme from a name (built-in or custom dir).
    ///
    /// Explicit name (omp `setTheme`): disables auto-detection. On load
    /// failure, falls back to the built-in `dark` theme so the TUI always
    /// has a usable theme. Returns the active theme name.
    pub fn init(&self, name: &str) -> String {
        self.disable_auto();
        let theme = Self::load_or_dark(name);
        self.commit(name, theme);
        name.to_string()
    }

    /// Initialize from terminal appearance (omp `initThemeSync` / `configureTheme`).
    ///
    /// Dark slot defaults to [`AUTO_DARK_THEME`] (`titanium`); light to
    /// [`AUTO_LIGHT_THEME`] (`light`).
    pub fn init_auto(&self, inputs: &appearance::AppearanceInputs) -> String {
        self.init_auto_mapped(
            appearance::AUTO_DARK_THEME,
            appearance::AUTO_LIGHT_THEME,
            inputs,
        )
    }

    /// Auto-init with explicit dark/light theme names.
    pub fn init_auto_mapped(
        &self,
        dark: &str,
        light: &str,
        inputs: &appearance::AppearanceInputs,
    ) -> String {
        if let Ok(mut a) = self.auto.write() {
            a.enabled = true;
            a.dark = dark.to_string();
            a.light = light.to_string();
            a.reported = inputs.osc11_appearance;
        }
        let name = appearance::resolve_auto_theme(dark, light, inputs);
        let theme = Self::load_or_dark(&name);
        self.commit(&name, theme);
        name
    }

    /// Enable auto-detection and re-evaluate (omp `enableAutoTheme`).
    pub fn enable_auto_theme(&self, inputs: &appearance::AppearanceInputs) {
        if let Ok(mut a) = self.auto.write() {
            a.enabled = true;
        }
        self.reevaluate_auto(inputs);
    }

    /// Update the auto dark/light mapping (omp `setAutoThemeMapping`).
    pub fn set_auto_theme_mapping(
        &self,
        slot: appearance::Appearance,
        theme_name: &str,
        inputs: &appearance::AppearanceInputs,
    ) {
        if let Ok(mut a) = self.auto.write() {
            match slot {
                appearance::Appearance::Dark => a.dark = theme_name.to_string(),
                appearance::Appearance::Light => a.light = theme_name.to_string(),
            }
        }
        self.reevaluate_auto(inputs);
    }

    /// OSC 11 / Mode 2031 classified appearance (omp `onTerminalAppearanceChange`).
    ///
    /// Returns `true` when the reported slot changed (and auto-theme may have
    /// swapped). Duplicate reports are ignored.
    pub fn on_terminal_appearance_change(
        &self,
        mode: appearance::Appearance,
        inputs: &appearance::AppearanceInputs,
    ) -> bool {
        {
            let mut a = match self.auto.write() {
                Ok(guard) => guard,
                Err(_) => return false,
            };
            if a.reported == Some(mode) {
                return false;
            }
            a.reported = Some(mode);
            if !a.enabled {
                return false;
            }
        }
        let mut merged = inputs.clone();
        merged.osc11_appearance = Some(mode);
        self.reevaluate_auto(&merged);
        true
    }

    /// Re-run auto mapping against `inputs` (no-op when auto is off).
    pub fn reevaluate_auto(&self, inputs: &appearance::AppearanceInputs) {
        let (enabled, dark, light, reported) = match self.auto.read() {
            Ok(a) => (a.enabled, a.dark.clone(), a.light.clone(), a.reported),
            Err(_) => return,
        };
        if !enabled {
            return;
        }
        let mut merged = inputs.clone();
        if merged.osc11_appearance.is_none() {
            merged.osc11_appearance = reported;
        }
        let name = appearance::resolve_auto_theme(&dark, &light, &merged);
        if self.get_current_theme_name().as_deref() == Some(name.as_str()) {
            return;
        }
        let theme = Self::load_or_dark(&name);
        self.commit(&name, theme);
    }

    /// Whether auto-detection is currently enabled.
    pub fn auto_detected(&self) -> bool {
        self.auto.read().ok().is_some_and(|a| a.enabled)
    }

    /// Swap the active theme to `name`, returning an error string on failure.
    /// Disables auto-detection (omp `setTheme`).
    pub fn set(&self, name: &str) -> Result<(), String> {
        let theme = loader::load_theme(name, &loader::CreateThemeOptions::default())?;
        self.disable_auto();
        if let Ok(mut g) = self.inner.write() {
            *g = Some(Arc::new(theme));
        }
        if let Ok(mut n) = self.name.write() {
            *n = Some(name.to_string());
        }
        self.bump_epoch();
        Ok(())
    }

    /// Preview a theme without committing the name (ephemeral swap).
    /// Does not disable auto-detection (omp `previewTheme`).
    pub fn preview(&self, name: &str) -> Result<(), String> {
        let theme = loader::load_theme(name, &loader::CreateThemeOptions::default())?;
        if let Ok(mut g) = self.inner.write() {
            *g = Some(Arc::new(theme));
        }
        self.bump_epoch();
        Ok(())
    }

    /// Install an already-constructed theme instance.
    /// Disables auto-detection (omp `setThemeInstance`).
    pub fn set_instance(&self, theme: Theme) {
        self.disable_auto();
        if let Ok(mut g) = self.inner.write() {
            *g = Some(Arc::new(theme));
        }
        self.bump_epoch();
    }

    /// The current theme, if initialized.
    pub fn current(&self) -> Option<Arc<Theme>> {
        self.inner.read().ok().and_then(|g| g.clone())
    }

    /// The current epoch (bumped on every theme change).
    pub fn epoch(&self) -> u64 {
        THEME_EPOCH.load(Ordering::Relaxed)
    }

    fn bump_epoch(&self) {
        THEME_EPOCH.fetch_add(1, Ordering::Relaxed);
    }

    /// The name of the currently active theme, if set via `init` or `set`.
    pub fn get_current_theme_name(&self) -> Option<String> {
        self.name.read().ok().and_then(|n| n.clone())
    }
}

/// Access the process-wide [`GlobalTheme`] singleton.
pub fn global() -> &'static GlobalTheme {
    &GLOBAL
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests;
