//! Personality slot: built-in presets, `PERSONALITY.md` override, and the
//! session-level overlay.
//!
//! omp model (spec §omp 3): the default prompt renders a personality block
//! from a preset; a user-level `PERSONALITY.md` **replaces** the preset text
//! verbatim (no template variables). An empty or unreadable file falls back
//! to the preset. Hermes adds the session-level `/personality` overlay on
//! top: a temporary mode-switch that never modifies the stored files.

use std::fs;
use std::path::Path;

/// Built-in personality presets (omp set: default / friendly / pragmatic;
/// `none` is expressed by [`Overlay::none`], not by a preset).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PersonalityPreset {
    Default,
    Friendly,
    Pragmatic,
}

impl PersonalityPreset {
    /// Parse a preset by its CLI/config name.
    pub fn from_name(name: &str) -> Option<Self> {
        match name.trim().to_lowercase().as_str() {
            "default" => Some(PersonalityPreset::Default),
            "friendly" => Some(PersonalityPreset::Friendly),
            "pragmatic" => Some(PersonalityPreset::Pragmatic),
            _ => None,
        }
    }

    /// The preset's config name.
    pub fn name(&self) -> &'static str {
        match self {
            PersonalityPreset::Default => "default",
            PersonalityPreset::Friendly => "friendly",
            PersonalityPreset::Pragmatic => "pragmatic",
        }
    }

    /// The personality text rendered into the prompt.
    pub fn text(&self) -> &'static str {
        match self {
            PersonalityPreset::Default => {
                "\
Respond in a clear, neutral, helpful tone. Prefer precision over flourish; \
state conclusions first and justify them after."
            }
            PersonalityPreset::Friendly => {
                "\
Be warm and encouraging. Celebrate progress, keep the tone light, and never \
let friendliness dilute correctness."
            }
            PersonalityPreset::Pragmatic => {
                "\
Be terse and outcome-focused: shortest correct answer first, caveats only \
when they change the decision."
            }
        }
    }
}

/// Session-level personality overlay (Hermes `/personality`): applied on top
/// of the persistent personality resolution and never written back to disk.
///
/// Priority inside the overlay: `custom` text, then `preset`, then neither
/// ([`Overlay::none`]) which drops the personality block for the session.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Overlay {
    /// Preset selected for this session.
    pub preset: Option<PersonalityPreset>,
    /// Free-form personality text for this session.
    pub custom: Option<String>,
}

impl Overlay {
    /// Overlay switching the session to a built-in preset.
    pub fn preset(preset: PersonalityPreset) -> Self {
        Overlay {
            preset: Some(preset),
            custom: None,
        }
    }

    /// Overlay with free-form session text.
    pub fn custom(text: impl Into<String>) -> Self {
        Overlay {
            preset: None,
            custom: Some(text.into()),
        }
    }

    /// Overlay that drops the personality block (`/personality none`).
    pub fn none() -> Self {
        Overlay {
            preset: None,
            custom: None,
        }
    }
}

/// Read `<agent_dir>/PERSONALITY.md`. `None` when the file is missing, empty,
/// or unreadable — the caller falls back to the preset (omp behaviour).
pub fn load(agent_dir: &Path) -> Option<String> {
    let text = fs::read_to_string(agent_dir.join("PERSONALITY.md")).ok()?;
    if text.trim().is_empty() {
        return None;
    }
    Some(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preset_round_trip_through_name() {
        for preset in [
            PersonalityPreset::Default,
            PersonalityPreset::Friendly,
            PersonalityPreset::Pragmatic,
        ] {
            assert_eq!(PersonalityPreset::from_name(preset.name()), Some(preset));
        }
        assert_eq!(
            PersonalityPreset::from_name("  PRAGMATIC "),
            Some(PersonalityPreset::Pragmatic)
        );
        assert_eq!(PersonalityPreset::from_name("kawaii"), None);
    }

    #[test]
    fn preset_texts_scan_clean() {
        for preset in [
            PersonalityPreset::Default,
            PersonalityPreset::Friendly,
            PersonalityPreset::Pragmatic,
        ] {
            assert!(crate::scan::scan(preset.text()).is_clean());
        }
    }

    #[test]
    fn personality_file_replaces_nothing_when_missing() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(load(dir.path()), None);
    }

    #[test]
    fn personality_file_read_verbatim() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("PERSONALITY.md"), "Speak like a pirate.\n").unwrap();
        assert_eq!(load(dir.path()).as_deref(), Some("Speak like a pirate.\n"));
    }

    #[test]
    fn blank_or_unreadable_personality_file_falls_back() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("PERSONALITY.md"), "   \n").unwrap();
        assert_eq!(load(dir.path()), None);

        let dir2 = tempfile::tempdir().unwrap();
        fs::write(dir2.path().join("PERSONALITY.md"), [0xff, 0x00]).unwrap();
        assert_eq!(load(dir2.path()), None);
    }

    #[test]
    fn overlay_constructors_set_priority_fields() {
        assert_eq!(
            Overlay::preset(PersonalityPreset::Friendly).preset,
            Some(PersonalityPreset::Friendly)
        );
        assert_eq!(Overlay::custom("brief").custom.as_deref(), Some("brief"));
        assert_eq!(Overlay::none(), Overlay::default());
    }
}
