//! Symbol glyph access — presets, spinner frames, override normalization.

use crate::theme::symbols_data::{ASCII_SYMBOLS, NERD_SYMBOLS, UNICODE_SYMBOLS};

/// Symbol key type.
pub type SymbolKey = String;

/// Symbol preset identifier.
pub type SymbolPreset = crate::theme::symbols_data::SymbolPreset;

/// Re-export the SpinnerFrames struct.
pub use crate::theme::symbols_data::SpinnerFrames;

/// Re-export the static spinner frames.
pub use crate::theme::symbols_data::{SPINNER_ASCII, SPINNER_NERD, SPINNER_UNICODE};

/// Get the symbol table for a preset.
pub fn symbols(preset: SymbolPreset) -> &'static [(&'static str, &'static str)] {
    match preset {
        SymbolPreset::Unicode => UNICODE_SYMBOLS,
        SymbolPreset::Nerd => NERD_SYMBOLS,
        SymbolPreset::Ascii => ASCII_SYMBOLS,
    }
}

/// Get spinner frames for a preset.
pub fn spinner_frames(preset: SymbolPreset) -> &'static SpinnerFrames {
    match preset {
        SymbolPreset::Unicode => &SPINNER_UNICODE,
        SymbolPreset::Nerd => &SPINNER_NERD,
        SymbolPreset::Ascii => &SPINNER_ASCII,
    }
}

/// Normalize a spinner-frames override into (status, activity) optionals.
pub fn normalize_spinner_frames_override(
    value: &Option<super::schema::SpinnerFramesOverride>,
) -> (Option<Vec<String>>, Option<Vec<String>>) {
    match value {
        None => (None, None),
        Some(super::schema::SpinnerFramesOverride::Both(arr)) => {
            (Some(arr.clone()), Some(arr.clone()))
        }
        Some(super::schema::SpinnerFramesOverride::Separate { status, activity }) => {
            (status.clone(), activity.clone())
        }
    }
}

/// Get available symbol preset names.
pub fn get_available_symbol_presets() -> &'static [&'static str] {
    crate::theme::symbols_data::SYMBOL_PRESET_NAMES
}

/// Check if a name is a valid symbol preset.
pub fn is_valid_symbol_preset(name: &str) -> bool {
    SymbolPreset::parse(name).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_unicode_has_keys() {
        let s = symbols(SymbolPreset::Unicode);
        assert!(s.len() > 200);
        assert!(s.iter().any(|(k, _)| *k == "status.success"));
    }

    #[test]
    fn test_nerd_has_keys() {
        let s = symbols(SymbolPreset::Nerd);
        assert!(s.len() > 200);
        assert!(s.iter().any(|(k, _)| *k == "status.success"));
    }

    #[test]
    fn test_ascii_has_keys() {
        let s = symbols(SymbolPreset::Ascii);
        assert!(s.len() > 200);
        assert!(s.iter().any(|(k, _)| *k == "status.success"));
    }

    #[test]
    fn test_all_presets_have_same_keys() {
        let u: Vec<&str> = symbols(SymbolPreset::Unicode)
            .iter()
            .map(|(k, _)| *k)
            .collect();
        let n: Vec<&str> = symbols(SymbolPreset::Nerd)
            .iter()
            .map(|(k, _)| *k)
            .collect();
        let a: Vec<&str> = symbols(SymbolPreset::Ascii)
            .iter()
            .map(|(k, _)| *k)
            .collect();
        assert_eq!(u, n);
        assert_eq!(u, a);
    }

    #[test]
    fn test_spinner_frames_unicode() {
        let f = spinner_frames(SymbolPreset::Unicode);
        assert!(!f.status.is_empty());
        assert!(!f.activity.is_empty());
    }

    #[test]
    fn test_spinner_frames_nerd() {
        let f = spinner_frames(SymbolPreset::Nerd);
        assert!(!f.status.is_empty());
        assert!(!f.activity.is_empty());
    }

    #[test]
    fn test_spinner_frames_ascii() {
        let f = spinner_frames(SymbolPreset::Ascii);
        assert_eq!(f.status.len(), 4);
        assert_eq!(f.activity.len(), 4);
    }

    #[test]
    fn test_parse_preset() {
        assert_eq!(SymbolPreset::parse("unicode"), Some(SymbolPreset::Unicode));
        assert_eq!(SymbolPreset::parse("nerd"), Some(SymbolPreset::Nerd));
        assert_eq!(SymbolPreset::parse("ascii"), Some(SymbolPreset::Ascii));
        assert_eq!(SymbolPreset::parse("unknown"), None);
    }

    #[test]
    fn test_is_valid_preset() {
        assert!(is_valid_symbol_preset("unicode"));
        assert!(!is_valid_symbol_preset("unknown"));
    }
}
