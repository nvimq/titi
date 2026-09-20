//! Theme loading — parse JSON, resolve vars, construct Theme instances.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::theme::Theme;
use crate::theme::builtin::{get_builtin_theme, list_builtin_themes};
use crate::theme::color::{ColorMode, detect_color_mode};
use crate::theme::schema::{ThemeJson, normalize_spinner_frames_override, resolve_theme_colors};
use crate::theme::symbols::SymbolPreset;

/// Options for creating a theme.
#[derive(Debug, Clone, Default)]
pub struct CreateThemeOptions {
    pub mode: Option<ColorMode>,
    pub symbol_preset_override: Option<SymbolPreset>,
    pub color_blind_mode: bool,
}

/// Load a theme JSON by name (built-in or custom dir).
pub fn load_theme_json(name: &str) -> Result<ThemeJson, String> {
    load_theme_json_in(name, &custom_themes_dir())
}

fn load_theme_json_in(name: &str, custom_dir: &Path) -> Result<ThemeJson, String> {
    // Built-in names win, matching omp `loadThemeJson`.
    if let Some(content) = get_builtin_theme(name) {
        return serde_json::from_str(content)
            .map_err(|e| format!("Failed to parse built-in theme '{name}': {e}"));
    }
    let theme_path = custom_dir.join(format!("{name}.json"));
    let content =
        std::fs::read_to_string(&theme_path).map_err(|_| format!("Theme not found: {name}"))?;
    serde_json::from_str(&content).map_err(|e| format!("Failed to parse theme '{name}': {e}"))
}

/// Load a theme JSON synchronously (for first paint).
pub fn load_theme_json_sync(name: &str) -> Result<ThemeJson, String> {
    load_theme_json(name)
}

/// Custom themes directory (`omp://theme.md` / `getCustomThemesDir`).
///
/// `{agentDir}/themes`, where `agentDir` is `$TITI_AGENT_DIR`, else
/// `$PI_CODING_AGENT_DIR`, else `~/.titi/profiles/<name>/agent` for a named
/// profile, else `~/.titi/agent`.
pub fn custom_themes_dir() -> PathBuf {
    custom_themes_dir_from_env(|key| std::env::var(key).ok())
}

fn custom_themes_dir_from_env(get: impl Fn(&str) -> Option<String>) -> PathBuf {
    agent_dir_from_env(&get).join("themes")
}

fn agent_dir_from_env(get: &impl Fn(&str) -> Option<String>) -> PathBuf {
    for key in ["TITI_AGENT_DIR", "PI_CODING_AGENT_DIR"] {
        if let Some(dir) = get(key) {
            let dir = dir.trim();
            if !dir.is_empty() {
                return PathBuf::from(dir);
            }
        }
    }
    let home = get("HOME")
        .filter(|h| !h.is_empty())
        .unwrap_or_else(|| "/tmp".to_owned());
    let root = PathBuf::from(home).join(".titi");
    match profile_name_from_env(get) {
        Some(name) => root.join("profiles").join(name).join("agent"),
        None => root.join("agent"),
    }
}

fn profile_name_from_env(get: &impl Fn(&str) -> Option<String>) -> Option<String> {
    for key in ["TITI_PROFILE", "OMP_PROFILE", "PI_PROFILE"] {
        if let Some(v) = get(key) {
            let t = v.trim();
            if !t.is_empty() && t != "default" {
                return Some(t.to_owned());
            }
        }
    }
    None
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
    let mut names: Vec<String> = list_builtin_themes()
        .into_iter()
        .map(|s| s.to_string())
        .collect();
    scan_custom_theme_names(&mut names, &custom_themes_dir());
    names.sort();
    names
}

fn scan_custom_theme_names(names: &mut Vec<String>, custom_dir: &Path) {
    if let Ok(entries) = std::fs::read_dir(custom_dir) {
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

    fn env<'a>(pairs: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
        |key| {
            pairs
                .iter()
                .find(|(k, _)| *k == key)
                .map(|(_, v)| (*v).to_owned())
        }
    }

    #[test]
    fn custom_dir_follows_titi_agent_dir() {
        let dir = custom_themes_dir_from_env(env(&[("TITI_AGENT_DIR", "/tmp/titi-agent")]));
        assert_eq!(dir, PathBuf::from("/tmp/titi-agent/themes"));
    }

    #[test]
    fn custom_dir_follows_pi_coding_agent_dir() {
        let dir = custom_themes_dir_from_env(env(&[("PI_CODING_AGENT_DIR", "/tmp/omp-agent")]));
        assert_eq!(dir, PathBuf::from("/tmp/omp-agent/themes"));
    }

    #[test]
    fn titi_agent_dir_wins_over_pi() {
        let dir = custom_themes_dir_from_env(env(&[
            ("TITI_AGENT_DIR", "/tmp/titi-agent"),
            ("PI_CODING_AGENT_DIR", "/tmp/omp-agent"),
        ]));
        assert_eq!(dir, PathBuf::from("/tmp/titi-agent/themes"));
    }

    #[test]
    fn default_custom_dir_is_dot_titi_agent_themes() {
        let dir = custom_themes_dir_from_env(env(&[("HOME", "/Users/me")]));
        assert_eq!(dir, PathBuf::from("/Users/me/.titi/agent/themes"));
    }

    #[test]
    fn named_profile_nests_under_profiles() {
        let dir =
            custom_themes_dir_from_env(env(&[("HOME", "/Users/me"), ("TITI_PROFILE", "work")]));
        assert_eq!(
            dir,
            PathBuf::from("/Users/me/.titi/profiles/work/agent/themes")
        );
    }

    #[test]
    fn default_profile_name_is_ignored() {
        let dir =
            custom_themes_dir_from_env(env(&[("HOME", "/Users/me"), ("TITI_PROFILE", "default")]));
        assert_eq!(dir, PathBuf::from("/Users/me/.titi/agent/themes"));
    }

    #[test]
    fn loads_custom_theme_from_agent_themes_dir() {
        let tmp = std::env::temp_dir().join(format!(
            "titi-theme-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        let themes = tmp.join("themes");
        std::fs::create_dir_all(&themes).unwrap();
        let src = get_builtin_theme("dark").expect("builtin dark");
        std::fs::write(themes.join("my-custom.json"), src).unwrap();
        let json = load_theme_json_in("my-custom", &themes).unwrap();
        let theme = create_theme(json, &CreateThemeOptions::default()).unwrap();
        assert!(!theme.is_light());
        let mut names = Vec::new();
        scan_custom_theme_names(&mut names, &themes);
        assert!(names.contains(&"my-custom".to_owned()));
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
