//! Theme JSON schema types and variable resolution.
//!
//! Mirrors the omp theme-schema (camelCase token names). Theme files
//! declare `vars` (named colors), `colors` (token -> var/hex/number),
//! optional `symbols` (preset + per-key overrides + spinner frames),
//! and `export` (derived colors consumed by downstream layers).
//!
//! Values in `colors` may reference `vars` either bare (`"accent"`) or
//! prefixed (`"$accent"`); [`resolve_theme_colors`] resolves both forms
//! into concrete hex/number values.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::theme::color::hex_to_rgb;

/// Theme color tokens (the `colors` keys of a theme JSON).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ThemeColor {
    Accent,
    Border,
    BorderAccent,
    BorderMuted,
    Success,
    Error,
    Warning,
    Muted,
    Dim,
    Text,
    ThinkingText,
    UserMessageText,
    CustomMessageText,
    CustomMessageLabel,
    ToolTitle,
    ToolOutput,
    MdHeading,
    MdLink,
    MdLinkUrl,
    MdCode,
    MdCodeBlock,
    MdCodeBlockBorder,
    MdQuote,
    MdQuoteBorder,
    MdHr,
    MdListBullet,
    ToolDiffAdded,
    ToolDiffRemoved,
    ToolDiffContext,
    SyntaxComment,
    SyntaxKeyword,
    SyntaxFunction,
    SyntaxVariable,
    SyntaxString,
    SyntaxNumber,
    SyntaxType,
    SyntaxOperator,
    SyntaxPunctuation,
    ThinkingOff,
    ThinkingMinimal,
    ThinkingLow,
    ThinkingMedium,
    ThinkingHigh,
    ThinkingXhigh,
    ThinkingMax,
    BashMode,
    PythonMode,
    StatusLineSep,
    StatusLineModel,
    StatusLinePath,
    StatusLineGitClean,
    StatusLineGitDirty,
    StatusLineContext,
    StatusLineSpend,
    StatusLineStaged,
    StatusLineDirty,
    StatusLineUntracked,
    StatusLineOutput,
    StatusLineCost,
    StatusLineSubagents,
}

/// Theme background tokens (subset of `colors` used as background fills).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ThemeBg {
    SelectedBg,
    UserMessageBg,
    CustomMessageBg,
    ToolPendingBg,
    ToolSuccessBg,
    ToolErrorBg,
    StatusLineBg,
}

impl ThemeColor {
    /// The camelCase JSON key for this foreground token.
    ///
    /// Used for hot-path lookups into the theme's resolved color maps; kept
    /// hand-written to avoid per-call serde serialization.
    pub fn as_str(&self) -> &'static str {
        match self {
            ThemeColor::Accent => "accent",
            ThemeColor::Border => "border",
            ThemeColor::BorderAccent => "borderAccent",
            ThemeColor::BorderMuted => "borderMuted",
            ThemeColor::Success => "success",
            ThemeColor::Error => "error",
            ThemeColor::Warning => "warning",
            ThemeColor::Muted => "muted",
            ThemeColor::Dim => "dim",
            ThemeColor::Text => "text",
            ThemeColor::ThinkingText => "thinkingText",
            ThemeColor::UserMessageText => "userMessageText",
            ThemeColor::CustomMessageText => "customMessageText",
            ThemeColor::CustomMessageLabel => "customMessageLabel",
            ThemeColor::ToolTitle => "toolTitle",
            ThemeColor::ToolOutput => "toolOutput",
            ThemeColor::MdHeading => "mdHeading",
            ThemeColor::MdLink => "mdLink",
            ThemeColor::MdLinkUrl => "mdLinkUrl",
            ThemeColor::MdCode => "mdCode",
            ThemeColor::MdCodeBlock => "mdCodeBlock",
            ThemeColor::MdCodeBlockBorder => "mdCodeBlockBorder",
            ThemeColor::MdQuote => "mdQuote",
            ThemeColor::MdQuoteBorder => "mdQuoteBorder",
            ThemeColor::MdHr => "mdHr",
            ThemeColor::MdListBullet => "mdListBullet",
            ThemeColor::ToolDiffAdded => "toolDiffAdded",
            ThemeColor::ToolDiffRemoved => "toolDiffRemoved",
            ThemeColor::ToolDiffContext => "toolDiffContext",
            ThemeColor::SyntaxComment => "syntaxComment",
            ThemeColor::SyntaxKeyword => "syntaxKeyword",
            ThemeColor::SyntaxFunction => "syntaxFunction",
            ThemeColor::SyntaxVariable => "syntaxVariable",
            ThemeColor::SyntaxString => "syntaxString",
            ThemeColor::SyntaxNumber => "syntaxNumber",
            ThemeColor::SyntaxType => "syntaxType",
            ThemeColor::SyntaxOperator => "syntaxOperator",
            ThemeColor::SyntaxPunctuation => "syntaxPunctuation",
            ThemeColor::ThinkingOff => "thinkingOff",
            ThemeColor::ThinkingMinimal => "thinkingMinimal",
            ThemeColor::ThinkingLow => "thinkingLow",
            ThemeColor::ThinkingMedium => "thinkingMedium",
            ThemeColor::ThinkingHigh => "thinkingHigh",
            ThemeColor::ThinkingXhigh => "thinkingXhigh",
            ThemeColor::ThinkingMax => "thinkingMax",
            ThemeColor::BashMode => "bashMode",
            ThemeColor::PythonMode => "pythonMode",
            ThemeColor::StatusLineSep => "statusLineSep",
            ThemeColor::StatusLineModel => "statusLineModel",
            ThemeColor::StatusLinePath => "statusLinePath",
            ThemeColor::StatusLineGitClean => "statusLineGitClean",
            ThemeColor::StatusLineGitDirty => "statusLineGitDirty",
            ThemeColor::StatusLineContext => "statusLineContext",
            ThemeColor::StatusLineSpend => "statusLineSpend",
            ThemeColor::StatusLineStaged => "statusLineStaged",
            ThemeColor::StatusLineDirty => "statusLineDirty",
            ThemeColor::StatusLineUntracked => "statusLineUntracked",
            ThemeColor::StatusLineOutput => "statusLineOutput",
            ThemeColor::StatusLineCost => "statusLineCost",
            ThemeColor::StatusLineSubagents => "statusLineSubagents",
        }
    }

    /// Parse a camelCase JSON key into a foreground token, if valid.
    pub fn parse(s: &str) -> Option<ThemeColor> {
        serde_json::from_str::<ThemeColor>(&format!("\"{s}\"")).ok()
    }
}

impl ThemeBg {
    /// The camelCase JSON key for this background token.
    pub fn as_str(&self) -> &'static str {
        match self {
            ThemeBg::SelectedBg => "selectedBg",
            ThemeBg::UserMessageBg => "userMessageBg",
            ThemeBg::CustomMessageBg => "customMessageBg",
            ThemeBg::ToolPendingBg => "toolPendingBg",
            ThemeBg::ToolSuccessBg => "toolSuccessBg",
            ThemeBg::ToolErrorBg => "toolErrorBg",
            ThemeBg::StatusLineBg => "statusLineBg",
        }
    }

    /// Parse a camelCase JSON key into a background token, if valid.
    pub fn parse(s: &str) -> Option<ThemeBg> {
        serde_json::from_str::<ThemeBg>(&format!("\"{s}\"")).ok()
    }
}

/// Spinner frame overrides, accepting either a single shared list
/// (`{ "spinnerFrames": [ ... ] }`) or per-type lists.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum SpinnerFramesOverride {
    /// One frame list used for both status and activity spinners.
    Both(Vec<String>),
    /// Separate frame lists; either side may be absent.
    Separate {
        status: Option<Vec<String>>,
        activity: Option<Vec<String>>,
    },
}

/// Top-level theme document.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThemeJson {
    #[serde(rename = "$schema", skip_serializing_if = "Option::is_none")]
    pub schema: Option<String>,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vars: Option<HashMap<String, serde_json::Value>>,
    pub colors: HashMap<String, serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub export: Option<HashMap<String, serde_json::Value>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub symbols: Option<ThemeSymbols>,
}

/// Optional symbol configuration within a theme.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThemeSymbols {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preset: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub overrides: Option<HashMap<String, String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spinner_frames: Option<SpinnerFramesOverride>,
}

/// Recursively resolve `value` through `vars`, detecting cycles.
///
/// Strings are handled as follows:
/// - empty or starting with `#` -> returned unchanged (terminal color /
///   concrete hex);
/// - a key of `vars` (optionally `$`-prefixed) -> replaced by that var's
///   resolved value;
/// - anything else -> returned unchanged.
///
/// Non-string values pass through untouched. Returns `Err` when a cycle
/// is detected (a var transitively references itself).
pub fn resolve_var_refs(
    value: &serde_json::Value,
    vars: &HashMap<String, serde_json::Value>,
) -> Result<serde_json::Value, String> {
    fn resolve(
        value: &serde_json::Value,
        vars: &HashMap<String, serde_json::Value>,
        visiting: &mut Vec<String>,
    ) -> Result<serde_json::Value, String> {
        match value {
            serde_json::Value::String(s) => {
                if s.is_empty() || s.starts_with('#') {
                    return Ok(value.clone());
                }
                let key = s.strip_prefix('$').unwrap_or(s);
                if visiting.iter().any(|k| k == key) {
                    return Err(format!("circular variable reference: ${key}"));
                }
                match vars.get(key) {
                    None => Ok(value.clone()),
                    Some(var_value) => {
                        visiting.push(key.to_string());
                        let resolved = resolve(var_value, vars, visiting);
                        visiting.pop();
                        resolved
                    }
                }
            }
            other => Ok(other.clone()),
        }
    }
    resolve(value, vars, &mut Vec::new())
}

/// Whether a resolved value is a usable color: an empty string (terminal
/// default), a parseable hex color, or an integer 256-color index.
fn is_resolvable_color_value(value: &serde_json::Value) -> bool {
    match value {
        serde_json::Value::String(s) => s.is_empty() || hex_to_rgb(s).is_some(),
        serde_json::Value::Number(n) => n.as_u64().is_some_and(|n| n <= 255),
        _ => false,
    }
}

/// Resolve every color through `vars`, dropping entries that do not
/// resolve to a usable color value.
pub fn resolve_theme_colors(
    colors: &HashMap<String, serde_json::Value>,
    vars: &HashMap<String, serde_json::Value>,
) -> HashMap<String, serde_json::Value> {
    let mut resolved = HashMap::new();
    for (name, value) in colors {
        let Ok(value) = resolve_var_refs(value, vars) else {
            continue;
        };
        if is_resolvable_color_value(&value) {
            resolved.insert(name.clone(), value);
        }
    }
    resolved
}

/// Whether `s` matches a [`ThemeColor`] variant name (camelCase).
pub fn is_valid_theme_color(s: &str) -> bool {
    matches!(
        s,
        "accent"
            | "border"
            | "borderAccent"
            | "borderMuted"
            | "success"
            | "error"
            | "warning"
            | "muted"
            | "dim"
            | "text"
            | "thinkingText"
            | "userMessageText"
            | "customMessageText"
            | "customMessageLabel"
            | "toolTitle"
            | "toolOutput"
            | "mdHeading"
            | "mdLink"
            | "mdLinkUrl"
            | "mdCode"
            | "mdCodeBlock"
            | "mdCodeBlockBorder"
            | "mdQuote"
            | "mdQuoteBorder"
            | "mdHr"
            | "mdListBullet"
            | "toolDiffAdded"
            | "toolDiffRemoved"
            | "toolDiffContext"
            | "syntaxComment"
            | "syntaxKeyword"
            | "syntaxFunction"
            | "syntaxVariable"
            | "syntaxString"
            | "syntaxNumber"
            | "syntaxType"
            | "syntaxOperator"
            | "syntaxPunctuation"
            | "thinkingOff"
            | "thinkingMinimal"
            | "thinkingLow"
            | "thinkingMedium"
            | "thinkingHigh"
            | "thinkingXhigh"
            | "thinkingMax"
            | "bashMode"
            | "pythonMode"
            | "statusLineSep"
            | "statusLineModel"
            | "statusLinePath"
            | "statusLineGitClean"
            | "statusLineGitDirty"
            | "statusLineContext"
            | "statusLineSpend"
            | "statusLineStaged"
            | "statusLineDirty"
            | "statusLineUntracked"
            | "statusLineOutput"
            | "statusLineCost"
            | "statusLineSubagents"
    )
}

/// Whether `s` matches a [`ThemeBg`] variant name (camelCase).
pub fn is_valid_theme_bg(s: &str) -> bool {
    matches!(
        s,
        "selectedBg" | "userMessageBg" | "customMessageBg" | "toolPendingBg" | "toolSuccessBg"
            | "toolErrorBg" | "statusLineBg"
    )
}

/// Whether `s` is a supported symbol preset name.
pub fn is_valid_symbol_preset(s: &str) -> bool {
    matches!(s, "unicode" | "nerd" | "ascii")
}

/// Split a [`SpinnerFramesOverride`] into `(status, activity)` frame
/// lists. A shared list yields both; an absent override yields `None`s.
pub fn normalize_spinner_frames_override(
    value: &Option<SpinnerFramesOverride>,
) -> (Option<Vec<String>>, Option<Vec<String>>) {
    match value {
        None => (None, None),
        Some(SpinnerFramesOverride::Both(frames)) => (Some(frames.clone()), Some(frames.clone())),
        Some(SpinnerFramesOverride::Separate { status, activity }) => {
            (status.clone(), activity.clone())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// All 60 color token names (camelCase), for cross-checking against
    /// serde serialization and `is_valid_theme_color`.
    const COLOR_TOKENS: [&str; 60] = [
        "accent",
        "border",
        "borderAccent",
        "borderMuted",
        "success",
        "error",
        "warning",
        "muted",
        "dim",
        "text",
        "thinkingText",
        "userMessageText",
        "customMessageText",
        "customMessageLabel",
        "toolTitle",
        "toolOutput",
        "mdHeading",
        "mdLink",
        "mdLinkUrl",
        "mdCode",
        "mdCodeBlock",
        "mdCodeBlockBorder",
        "mdQuote",
        "mdQuoteBorder",
        "mdHr",
        "mdListBullet",
        "toolDiffAdded",
        "toolDiffRemoved",
        "toolDiffContext",
        "syntaxComment",
        "syntaxKeyword",
        "syntaxFunction",
        "syntaxVariable",
        "syntaxString",
        "syntaxNumber",
        "syntaxType",
        "syntaxOperator",
        "syntaxPunctuation",
        "thinkingOff",
        "thinkingMinimal",
        "thinkingLow",
        "thinkingMedium",
        "thinkingHigh",
        "thinkingXhigh",
        "thinkingMax",
        "bashMode",
        "pythonMode",
        "statusLineSep",
        "statusLineModel",
        "statusLinePath",
        "statusLineGitClean",
        "statusLineGitDirty",
        "statusLineContext",
        "statusLineSpend",
        "statusLineStaged",
        "statusLineDirty",
        "statusLineUntracked",
        "statusLineOutput",
        "statusLineCost",
        "statusLineSubagents",
    ];

    const BG_TOKENS: [&str; 7] = [
        "selectedBg",
        "userMessageBg",
        "customMessageBg",
        "toolPendingBg",
        "toolSuccessBg",
        "toolErrorBg",
        "statusLineBg",
    ];

    #[test]
    fn all_color_tokens_parse_and_validate() {
        for token in COLOR_TOKENS {
            let parsed: ThemeColor = serde_json::from_str(&format!("\"{token}\"")).expect(token);
            assert!(is_valid_theme_color(token), "token {token} not valid");
            // serde camelCase round-trips back to the same name.
            let back = serde_json::to_string(&parsed).unwrap();
            assert_eq!(back, format!("\"{token}\""), "round trip for {token}");
        }
        for token in BG_TOKENS {
            let parsed: ThemeBg = serde_json::from_str(&format!("\"{token}\"")).expect(token);
            assert!(is_valid_theme_bg(token), "bg token {token} not valid");
            let back = serde_json::to_string(&parsed).unwrap();
            assert_eq!(back, format!("\"{token}\""), "round trip for {token}");
        }
        assert_eq!(COLOR_TOKENS.len(), 60);
        assert_eq!(BG_TOKENS.len(), 7);
    }

    #[test]
    fn is_valid_checks_reject_garbage() {
        assert!(!is_valid_theme_color("accentt"));
        assert!(!is_valid_theme_color(""));
        assert!(!is_valid_theme_color("selectedBg"));
        assert!(is_valid_theme_color("statusLineGitClean"));
        assert!(!is_valid_theme_bg("accent"));
        assert!(is_valid_theme_bg("userMessageBg"));
        assert!(is_valid_symbol_preset("unicode"));
        assert!(is_valid_symbol_preset("nerd"));
        assert!(is_valid_symbol_preset("ascii"));
        assert!(!is_valid_symbol_preset("emoji"));
        assert!(!is_valid_symbol_preset(""));
    }

    #[test]
    fn dark_json_round_trip() {
        let raw = include_str!("../../themes/dark.json");
        let theme: ThemeJson = serde_json::from_str(raw).expect("dark.json parses");
        assert_eq!(theme.name, "dark");
        assert!(theme.schema.as_deref().is_some_and(|s| s.contains("theme-schema")));
        assert!(theme.vars.as_ref().is_some_and(|v| v.contains_key("accent")));
        assert!(theme.colors.contains_key("accent"));
        assert!(theme.export.is_some());
        assert!(theme.symbols.is_none());
        // name field survives serialization
        let round = serde_json::to_string(&theme).unwrap();
        assert!(round.contains("\"name\":\"dark\""));
    }

    #[test]
    fn onyx_json_parses_dollar_vars() {
        let raw = include_str!("../../themes/defaults/onyx.json");
        let theme: ThemeJson = serde_json::from_str(raw).expect("onyx.json parses");
        assert_eq!(theme.name, "onyx");
        // `$vein` references in the file (prefixed var syntax)
        let vars = theme.vars.as_ref().expect("vars present");
        let colors = resolve_theme_colors(&theme.colors, vars);
        let accent = colors.get("accent").expect("accent resolved");
        assert_eq!(accent.as_str(), Some("#d4a853"));
    }

    #[test]
    fn var_resolution_bare_and_prefixed() {
        let vars: HashMap<String, serde_json::Value> = serde_json::from_value(serde_json::json!({
            "brand": "#123456",
            "lighter": "$brand",
        }))
        .unwrap();

        // Bare reference
        let v = resolve_var_refs(&serde_json::json!("brand"), &vars).unwrap();
        assert_eq!(v, serde_json::json!("#123456"));

        // $ -prefixed reference
        let v = resolve_var_refs(&serde_json::json!("$brand"), &vars).unwrap();
        assert_eq!(v, serde_json::json!("#123456"));

        // Nested var -> var
        let v = resolve_var_refs(&serde_json::json!("lighter"), &vars).unwrap();
        assert_eq!(v, serde_json::json!("#123456"));

        // Hex, empty string, and numbers pass through untouched.
        assert_eq!(resolve_var_refs(&serde_json::json!("#abc"), &vars).unwrap(), serde_json::json!("#abc"));
        assert_eq!(resolve_var_refs(&serde_json::json!(""), &vars).unwrap(), serde_json::json!(""));
        assert_eq!(resolve_var_refs(&serde_json::json!(244), &vars).unwrap(), serde_json::json!(244));

        // Unknown key stays as-is.
        let v = resolve_var_refs(&serde_json::json!("nope"), &vars).unwrap();
        assert_eq!(v, serde_json::json!("nope"));
    }

    #[test]
    fn var_cycle_detection() {
        let vars: HashMap<String, serde_json::Value> = serde_json::from_value(serde_json::json!({
            "a": "$b",
            "b": "$a",
        }))
        .unwrap();
        let err = resolve_var_refs(&serde_json::json!("a"), &vars).unwrap_err();
        assert!(err.contains("circular"), "err: {err}");

        // Self-reference
        let vars: HashMap<String, serde_json::Value> = serde_json::from_value(serde_json::json!({
            "a": "$a",
        }))
        .unwrap();
        assert!(resolve_var_refs(&serde_json::json!("a"), &vars).is_err());
    }

    #[test]
    fn resolve_theme_colors_skips_invalid() {
        let vars: HashMap<String, serde_json::Value> = serde_json::from_value(serde_json::json!({
            "ok": "#abcdef",
            "badCycle": "$badCycle",
        }))
        .unwrap();
        let colors: HashMap<String, serde_json::Value> = serde_json::from_value(serde_json::json!({
            "accent": "$ok",
            "text": "",
            "statusLineSep": 244,
            "broken": "#zzzzzz",
            "missingVar": "ghost",
            "cyclic": "$badCycle",
        }))
        .unwrap();
        let resolved = resolve_theme_colors(&colors, &vars);
        assert_eq!(resolved.len(), 3, "only accent/text/statusLineSep survive");
        assert_eq!(resolved["accent"], serde_json::json!("#abcdef"));
        assert_eq!(resolved["text"], serde_json::json!(""));
        assert_eq!(resolved["statusLineSep"], serde_json::json!(244));
    }

    #[test]
    fn spinner_frames_override_forms() {
        // Shared list form
        let both: SpinnerFramesOverride =
            serde_json::from_value(serde_json::json!(["a", "b"])).unwrap();
        let (s, a) = normalize_spinner_frames_override(&Some(both));
        assert_eq!(s, Some(vec!["a".to_string(), "b".to_string()]));
        assert_eq!(a, Some(vec!["a".to_string(), "b".to_string()]));

        // Separate form, both sides
        let sep: SpinnerFramesOverride =
            serde_json::from_value(serde_json::json!({ "status": ["s1"], "activity": ["a1"] }))
                .unwrap();
        let (s, a) = normalize_spinner_frames_override(&Some(sep));
        assert_eq!(s, Some(vec!["s1".to_string()]));
        assert_eq!(a, Some(vec!["a1".to_string()]));

        // Separate form, one side absent
        let partial: SpinnerFramesOverride =
            serde_json::from_value(serde_json::json!({ "status": ["s1"] })).unwrap();
        let (s, a) = normalize_spinner_frames_override(&Some(partial));
        assert_eq!(s, Some(vec!["s1".to_string()]));
        assert_eq!(a, None);

        // None
        assert_eq!(normalize_spinner_frames_override(&None), (None, None));
    }
}
