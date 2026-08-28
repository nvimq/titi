//! Theme loading — parse JSON, resolve vars, construct Theme instances.

use std::collections::HashMap;
use std::path::PathBuf;

use crate::theme::builtin::{get_builtin_theme, list_builtin_themes};
use crate::theme::color::{detect_color_mode, ColorMode};
use crate::theme::schema::{normalize_spinner_frames_override, resolve_theme_colors, ThemeJson};
use crate::theme::symbols::SymbolPreset;
use crate::theme::Theme;

/// Options for creating a theme.
#[derive(Debug, Clone, Default)]
pub struct CreateThemeOptions {
    pub mode: Option<ColorMode>,
    pub symbol_preset_override: Option<SymbolPreset>,
    pub color_blind_mode: bool,
}

/// Load a theme JSON by name (built-in or custom dir).
pub fn load_theme_json(name: &str) -> Result<ThemeJson, String> {
    // Try built-in
    if let Some(content) = get_builtin_theme(name) {
        return serde_json::from_str(content)
            .map_err(|e| format!("Failed to parse built-in theme '{}': {}", name, e));
    }
    // Try custom dir: ~/.titi/themes/{name}.json
    let custom_dir = custom_themes_dir();
    let theme_path = custom_dir.join(format!("{}.json", name));
    let content = std::fs::read_to_string(&theme_path)
        .map_err(|_| format!("Theme not found: {}", name))?;
    serde_json::from_str(&content)
        .map_err(|e| format!("Failed to parse theme '{}': {}", name, e))
}

/// Load a theme JSON synchronously (for first paint).
pub fn load_theme_json_sync(name: &str) -> Result<ThemeJson, String> {
    load_theme_json(name)
}

/// Get the custom themes directory.
pub fn custom_themes_dir() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
    PathBuf::from(home).join(".titi").join("themes")
}

/// Create a Theme from parsed JSON + options.
pub fn create_theme(theme_json: ThemeJson, options: &CreateThemeOptions) -> Result<Theme, String> {
    let color_mode = options.mode.unwrap_or_else(detect_color_mode);
    let vars = theme_json.vars.unwrap_or_default();
    let resolved_colors = resolve_theme_colors(&theme_json.colors, &vars);

    let symbol_preset = options
        .symbol_preset_override
        .or_else(|| {
            theme_json
                .symbols
                .as_ref()
                .and_then(|s| s.preset.as_deref().and_then(SymbolPreset::parse))
        })
        .unwrap_or(SymbolPreset::Unicode);

    let symbol_overrides = theme_json
        .symbols
        .as_ref()
        .and_then(|s| s.overrides.clone())
        .unwrap_or_default();

    let (spinner_status_override, spinner_activity_override) = normalize_spinner_frames_override(
        &theme_json
            .symbols
            .as_ref()
            .and_then(|s| s.spinner_frames.clone()),
    );

    // Partition into fg and bg colors
    let bg_keys: [&str; 7] = [
        "selectedBg",
        "userMessageBg",
        "customMessageBg",
        "toolPendingBg",
        "toolSuccessBg",
        "toolErrorBg",
        "statusLineBg",
    ];

    let mut fg_colors = HashMap::new();
    let mut bg_colors = HashMap::new();

    for (key, value) in &resolved_colors {
        if bg_keys.contains(&key.as_str()) {
            bg_colors.insert(key.clone(), value.clone());
        } else {
            fg_colors.insert(key.clone(), value.clone());
        }
    }

    Theme::new(
        theme_json.name,
        fg_colors,
        bg_colors,
        color_mode,
        symbol_preset,
        symbol_overrides,
        spinner_status_override,
        spinner_activity_override,
    )
}

/// Load a theme by name.
pub fn load_theme(name: &str, options: &CreateThemeOptions) -> Result<Theme, String> {
    let theme_json = load_theme_json(name)?;
    create_theme(theme_json, options)
}

/// Load a theme synchronously (for first paint).
pub fn load_theme_sync(name: &str, options: &CreateThemeOptions) -> Result<Theme, String> {
    load_theme(name, options)
}

/// Get all available theme names (built-in + custom).
pub fn get_available_themes() -> Vec<String> {
    let mut names: Vec<String> =
        list_builtin_themes().into_iter().map(|s| s.to_string()).collect();
    // Scan custom dir
    let custom_dir = custom_themes_dir();
    if let Ok(entries) = std::fs::read_dir(&custom_dir) {
        for entry in entries.flatten() {
            if let Some(name) = entry
                .file_name()
                .to_str()
                .and_then(|s| s.strip_suffix(".json"))
            {
                let name_str = name.to_string();
                if !names.contains(&name_str) {
                    names.push(name_str);
                }
            }
        }
    }
    names.sort();
    names
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_load_dark() {
        let theme = load_theme("dark", &CreateThemeOptions::default()).unwrap();
        assert!(!theme.is_light());
    }

    #[test]
    fn test_load_light() {
        let theme = load_theme("light", &CreateThemeOptions::default()).unwrap();
        assert!(theme.is_light());
    }

    #[test]
    fn test_unknown_theme() {
        let result = load_theme("nonexistent_theme_xyz", &CreateThemeOptions::default());
        assert!(result.is_err());
    }

    #[test]
    fn test_custom_dir_fallback() {
        // Built-in lookup wins over the custom dir.
        let theme = load_theme("dark", &CreateThemeOptions::default()).unwrap();
        assert!(!theme.is_light());
    }

    #[test]
    fn test_create_theme_with_symbol_override() {
        let theme_json = load_theme_json("dark").unwrap();
        let opts = CreateThemeOptions {
            symbol_preset_override: Some(SymbolPreset::Ascii),
            ..Default::default()
        };
        let theme = create_theme(theme_json, &opts).unwrap();
        assert_eq!(theme.get_symbol_preset(), SymbolPreset::Ascii);
    }

    #[test]
    fn test_get_available() {
        let themes = get_available_themes();
        assert!(themes.contains(&"dark".to_string()));
        assert!(themes.contains(&"light".to_string()));
        assert!(themes.len() >= 100); // 98 defaults + dark + light
    }
}
