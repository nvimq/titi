//! Keyboard input parsing and matching (contract: `omp://tui` — keybindings,
//! spec: `docs/research/tui-renderer/input-capabilities-graphics.md`).
//!
//! Mirrors omp's `packages/tui/src/keys.ts` (+ native normalization): raw
//! terminal input bytes are parsed into canonical key ids like `"ctrl+p"` or
//! `"shift+tab"`, and bindings match against those ids. Kitty CSI-u and xterm
//! `modifyOtherKeys` sequences are decoded so text-entry components can treat
//! keypad digits, keypad operators and shifted symbols as character input.

use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};

// ---------------------------------------------------------------------------
// Kitty keyboard protocol state
// ---------------------------------------------------------------------------

static KITTY_PROTOCOL_ACTIVE: AtomicBool = AtomicBool::new(false);

/// Set the global Kitty keyboard protocol state.
/// Called by the terminal layer after detecting protocol support.
pub fn set_kitty_protocol_active(active: bool) {
    KITTY_PROTOCOL_ACTIVE.store(active, Ordering::Relaxed);
}

/// Whether the Kitty keyboard protocol is currently active.
pub fn is_kitty_protocol_active() -> bool {
    KITTY_PROTOCOL_ACTIVE.load(Ordering::Relaxed)
}

// ---------------------------------------------------------------------------
// Canonical key ids
// ---------------------------------------------------------------------------

const MODIFIER_ORDER: [&str; 4] = ["ctrl", "shift", "alt", "super"];

/// Symbol keys typed with Shift on a US layout; `shift+<symbol>` is a
/// canonical alias of the symbol itself.
const SHIFTED_SYMBOL_KEYS: &[&str] = &[
    "!", "@", "#", "$", "%", "^", "&", "*", "(", ")", "_", "+", "{", "}", "|", ":", "<",
    ">", "?", "~",
];

fn starts_with_modifier(key: &str, offset: usize, modifier: &str) -> bool {
    let Some(rest) = key.get(offset..) else { return false };
    let Some(after) = rest.get(modifier.len()..) else { return false };
    if !after.starts_with('+') {
        return false;
    }
    rest[..modifier.len()].eq_ignore_ascii_case(modifier)
}

fn is_ascii_uppercase_letter(c: char) -> bool {
    c.is_ascii_uppercase()
}

/// Normalize a key id string to its canonical form: modifiers sorted
/// `ctrl < shift < alt < super`, base lowercased, `esc`→`escape`,
/// `return`→`enter`, and an uppercase letter base implies `shift`.
///
/// `"Ctrl+P"` → `"ctrl+p"`; `"A"` → `"shift+a"`; `"esc"` → `"escape"`.
pub fn canonical_key_id(key: &str) -> String {
    let mut offset = 0;
    let mut modifiers: Vec<&str> = Vec::new();
    loop {
        let mut found = false;
        for &modifier in &MODIFIER_ORDER {
            if starts_with_modifier(key, offset, modifier) {
                modifiers.push(modifier);
                offset += modifier.len() + 1;
                found = true;
                break;
            }
        }
        if !found {
            break;
        }
    }
    let raw_base = &key[offset..];
    let lower = raw_base.to_ascii_lowercase();
    let base = match lower.as_str() {
        "esc" => "escape".to_owned(),
        "return" => "enter".to_owned(),
        other => other.to_owned(),
    };
    if raw_base.chars().count() == 1
        && is_ascii_uppercase_letter(raw_base.chars().next().unwrap_or(' '))
        && !modifiers.contains(&"shift")
    {
        modifiers.push("shift");
    }
    if modifiers.is_empty() {
        return base;
    }
    modifiers.sort_by_key(|m| MODIFIER_ORDER.iter().position(|x| x == m).unwrap_or(usize::MAX));
    format!("{}+{base}", modifiers.join("+"))
}

/// Add a key id and its canonical aliases to `keys`.
///
/// Aliases: the canonical form plus `shift+<symbol>` for shifted symbol keys.
/// Used to build match sets so `?` also matches input reported as `shift+?`.
pub fn add_key_aliases(keys: &mut HashSet<String>, key: &str) {
    let canonical = canonical_key_id(key);
    keys.insert(canonical.clone());
    if SHIFTED_SYMBOL_KEYS.contains(&canonical.as_str()) {
        keys.insert(format!("shift+{canonical}"));
    }
}

// ---------------------------------------------------------------------------
// Kitty CSI-u and modifyOtherKeys wire parsing
// ---------------------------------------------------------------------------

/// Modifier bits in the Kitty protocol and xterm `modifyOtherKeys` encoding.
const KITTY_MOD_SHIFT: u32 = 1;
const KITTY_MOD_ALT: u32 = 2;
const KITTY_MOD_CTRL: u32 = 4;
const KITTY_MOD_SUPER: u32 = 8;
/// Caps Lock (64) + Num Lock (128): layout locks, not modifiers.
const KITTY_LOCK_MASK: u32 = 64 + 128;
const SUPPORTED_MODIFIER_MASK: u32 = KITTY_MOD_SHIFT | KITTY_MOD_ALT | KITTY_MOD_CTRL | KITTY_MOD_SUPER;

/// Keypad operator keys in Kitty CSI-u encoding.
const KITTY_KEYPAD_OPERATORS: &[(u32, &str)] = &[
    (57410, "/"),
    (57411, "*"),
    (57412, "-"),
    (57413, "+"),
    (57415, "="),
];

/// Numpad digit keys in Kitty CSI-u encoding.
const KITTY_NUMPAD: &[(u32, &str)] = &[
    (57399, "0"),
    (57400, "1"),
    (57401, "2"),
    (57402, "3"),
    (57403, "4"),
    (57404, "5"),
    (57405, "6"),
    (57406, "7"),
    (57407, "8"),
    (57408, "9"),
    (57409, "."),
];

/// Navigation name for a keypad codepoint when NumLock is off / a modifier is
/// held (the classic keypad-arrows mapping).
const KITTY_KEYPAD_NAV: &[(u32, &str)] = &[
    (57400, "end"),
    (57401, "down"),
    (57402, "pagedown"),
    (57403, "left"),
    (57404, "clear"),
    (57405, "right"),
    (57406, "home"),
    (57407, "up"),
    (57408, "pageup"),
    (57409, "delete"),
];

/// Named keys whose CSI-u / modifyOtherKeys codepoint resolves to a canonical
/// identifier instead of a printable character.
const NAMED_KEYS: &[(u32, &str)] = &[
    (9, "tab"),
    (13, "enter"),
    (27, "escape"),
    (32, "space"),
    (127, "backspace"),
];

struct ParsedKittySequence {
    codepoint: u32,
    shifted_key: Option<u32>,
    base_layout_key: Option<u32>,
    /// 0-based modifier bitmask.
    modifier: u32,
    /// `None` for press, `2` repeat, `3` release.
    event_type: Option<u32>,
    /// `;`-separated text codepoints (Kitty text field), if any.
    text_field: Option<Vec<u32>>,
}

fn parse_kitty_sequence(data: &str) -> Option<ParsedKittySequence> {
    let rest = data.strip_prefix("\x1b[")?.strip_suffix('u')?;
    if rest.is_empty() || !rest.bytes().all(|b| b.is_ascii_digit() || b == b':' || b == b';') {
        return None;
    }
    let mut fields = rest.split(';');
    // First field: `<code>(:shifted)?(:base)?` — the second `:` group is
    // optional and empty for the shifted slot (e.g. `107::118`).
    let first = fields.next()?;
    let mut code_parts = first.splitn(3, ':');
    let codepoint: u32 = code_parts.next()?.parse().ok()?;
    let shifted_raw = code_parts.next();
    let base_raw = code_parts.next();
    let shifted_key = shifted_raw
        .filter(|s| !s.is_empty())
        .and_then(|s| s.parse::<u32>().ok());
    let base_layout_key = base_raw.and_then(|s| s.parse::<u32>().ok());

    let mut modifier = 1u32; // wire default: no modifiers (1-indexed)
    let mut event_type: Option<u32> = None;
    let mut text_field: Option<Vec<u32>> = None;
    // Second field: `<mod>(:event)?` — e.g. `5` or `1:2` (repeat).
    if let Some(second) = fields.next() {
        let mut parts = second.split(':');
        if let Some(mod_str) = parts.next() {
            if !mod_str.is_empty() {
                if let Ok(v) = mod_str.parse::<u32>() {
                    modifier = v;
                }
            }
        }
        if let Some(ev) = parts.next() {
            if !ev.is_empty() {
                event_type = ev.parse::<u32>().ok();
            }
        }
        // Any extra `:` parts after the event belong to the text field.
        let extra: Vec<u32> = parts.filter_map(|p| p.parse::<u32>().ok()).collect();
        if !extra.is_empty() {
            text_field = Some(extra);
        }
    }
    // Remaining `;`-separated fields are the text field (`;229` or `;229:230`).
    let mut text = text_field.take().unwrap_or_default();
    for field in fields {
        if !field.is_empty() {
            text.extend(field.split(':').filter_map(|p| p.parse::<u32>().ok()));
        }
    }
    if !text.is_empty() {
        text_field = Some(text);
    }
    Some(ParsedKittySequence {
        codepoint,
        shifted_key,
        base_layout_key,
        modifier: modifier.saturating_sub(1),
        event_type,
        text_field,
    })
}

/// Decode a Kitty CSI-u sequence into the printable character it represents.
///
/// Returns `None` for release events, modified keys, named keys (space,
/// backspace, …), and keypad navigation with held modifiers — those are
/// handled by [`parse_key`]/[`matches_key`] normalization.
fn decode_kitty_printable(data: &str) -> Option<String> {
    let parsed = parse_kitty_sequence(data)?;
    if parsed.event_type == Some(3) {
        return None;
    }
    let effective_mod = parsed.modifier & !KITTY_LOCK_MASK;
    if effective_mod & !SUPPORTED_MODIFIER_MASK != 0 {
        return None;
    }
    // Ctrl/Alt/Super produce bindings, not text.
    if effective_mod & (KITTY_MOD_ALT | KITTY_MOD_CTRL | KITTY_MOD_SUPER) != 0 {
        return None;
    }
    // Text field carries the actual produced codepoints when present.
    if let Some(text) = &parsed.text_field {
        let printable: Vec<char> = text
            .iter()
            .copied()
            .filter(|&cp| cp >= 32 && cp != 127)
            .filter_map(char::from_u32)
            .collect();
        if !printable.is_empty() {
            return Some(printable.into_iter().collect());
        }
    }
    if let Some((_, op)) = KITTY_KEYPAD_OPERATORS.iter().find(|(cp, _)| *cp == parsed.codepoint) {
        return Some((*op).to_owned());
    }
    if effective_mod == 0 {
        if let Some((_, digit)) = KITTY_NUMPAD.iter().find(|(cp, _)| *cp == parsed.codepoint) {
            return Some((*digit).to_owned());
        }
    }
    let mut effective_codepoint = parsed.codepoint;
    if effective_mod & KITTY_MOD_SHIFT != 0 {
        if let Some(shifted) = parsed.shifted_key {
            effective_codepoint = shifted;
        }
    }
    // Private-use-area keys (keypad nav with locks/modifiers) are not text.
    if (0xe000..=0xf8ff).contains(&effective_codepoint) {
        return None;
    }
    if effective_codepoint < 32 || effective_codepoint == 127 {
        return None;
    }
    char::from_u32(effective_codepoint).map(|c| c.to_string())
}

/// xterm `modifyOtherKeys` sequence: `ESC [ 27 ; modifiers ; keycode ~`.
/// Modifier values are 1-indexed on the wire; normalized to a 0-based bitmask.
fn parse_modify_other_keys(data: &str) -> Option<(u32, u32)> {
    let rest = data.strip_prefix("\x1b[27;")?.strip_suffix('~')?;
    let (mod_str, code_str) = rest.split_once(';')?;
    let mod_value: u32 = mod_str.parse().ok()?;
    let codepoint: u32 = code_str.parse().ok()?;
    Some((mod_value.saturating_sub(1), codepoint))
}

/// Decode an xterm `modifyOtherKeys` sequence into the printable character.
/// Only no-modifier or Shift-only sequences produce text.
fn decode_modify_other_keys_printable(data: &str) -> Option<String> {
    let (modifier, codepoint) = parse_modify_other_keys(data)?;
    let effective_mod = modifier & !KITTY_LOCK_MASK;
    if effective_mod & !KITTY_MOD_SHIFT != 0 {
        return None;
    }
    if codepoint < 32 || codepoint == 127 {
        return None;
    }
    char::from_u32(codepoint).map(|c| c.to_string())
}

/// Decode terminal input into the printable character it represents.
/// Tries Kitty CSI-u first, then xterm `modifyOtherKeys`.
pub fn decode_printable_key(data: &str) -> Option<String> {
    decode_kitty_printable(data).or_else(|| decode_modify_other_keys_printable(data))
}

/// Decode a Kitty keypad sequence (digits/operators) into its text, or `None`
/// for any non-keypad sequence. Keeps keypad digits text-like even when the
/// terminal reports NumLock; navigation with held modifiers stays canonical.
fn decode_kitty_keypad_text(data: &str) -> Option<String> {
    let parsed = parse_kitty_sequence(data)?;
    let is_keypad = KITTY_NUMPAD
        .iter()
        .any(|(cp, _)| *cp == parsed.codepoint)
        || KITTY_KEYPAD_OPERATORS.iter().any(|(cp, _)| *cp == parsed.codepoint);
    if !is_keypad {
        return None;
    }
    decode_kitty_printable(data)
}

/// Extract printable text from raw terminal input. Handles Kitty CSI-u
/// keypad digits/operators and plain text; returns `None` for control
/// sequences and modifier-only events.
pub fn extract_printable_text(data: &str) -> Option<String> {
    let printable = decode_printable_key(data);
    if printable.is_some() {
        return printable;
    }
    if data.is_empty() || data.chars().any(|c| {
        let code = c as u32;
        code < 32 || code == 0x7f || (0x80..=0x9f).contains(&code)
    }) {
        return None;
    }
    Some(data.to_owned())
}

// ---------------------------------------------------------------------------
// Native-style parsing to canonical key ids
// ---------------------------------------------------------------------------

fn named_key(codepoint: u32) -> Option<&'static str> {
    NAMED_KEYS.iter().find(|(cp, _)| *cp == codepoint).map(|(_, name)| *name)
}

fn keypad_nav_name(codepoint: u32) -> Option<&'static str> {
    KITTY_KEYPAD_NAV.iter().find(|(cp, _)| *cp == codepoint).map(|(_, name)| *name)
}

/// Build a canonical key id from a base name and a 0-based modifier bitmask.
fn with_modifiers(base: &str, modifier: u32) -> String {
    let mut mods = Vec::new();
    if modifier & KITTY_MOD_CTRL != 0 {
        mods.push("ctrl");
    }
    if modifier & KITTY_MOD_SHIFT != 0 {
        mods.push("shift");
    }
    if modifier & KITTY_MOD_ALT != 0 {
        mods.push("alt");
    }
    if modifier & KITTY_MOD_SUPER != 0 {
        mods.push("super");
    }
    if mods.is_empty() {
        base.to_owned()
    } else {
        format!("{}+{base}", mods.join("+"))
    }
}

/// Parse a Kitty CSI-u sequence into a canonical key id.
fn parse_kitty_key(data: &str) -> Option<String> {
    let parsed = parse_kitty_sequence(data)?;
    if parsed.event_type == Some(3) {
        return None; // release events never match
    }
    let effective_mod = parsed.modifier & !KITTY_LOCK_MASK;
    if effective_mod & !SUPPORTED_MODIFIER_MASK != 0 {
        return None; // hyper/meta are not surfaced
    }

    // Named keys first: space, backspace, tab, enter, escape.
    if let Some(name) = named_key(parsed.codepoint) {
        return Some(with_modifiers(name, effective_mod));
    }

    // Keypad: bare/NumLock → digit text; held modifiers → navigation key.
    if keypad_nav_name(parsed.codepoint).is_some() {
        if effective_mod == 0 {
            if let Some((_, digit)) = KITTY_NUMPAD.iter().find(|(cp, _)| *cp == parsed.codepoint) {
                return Some((*digit).to_owned());
            }
        }
        return Some(with_modifiers(keypad_nav_name(parsed.codepoint)?, effective_mod));
    }
    if let Some((_, op)) = KITTY_KEYPAD_OPERATORS.iter().find(|(cp, _)| *cp == parsed.codepoint) {
        return Some(with_modifiers(op, effective_mod));
    }

    // Letters/symbols: prefer the codepoint for ASCII (Latin letters and
    // symbols, so Dvorak Ctrl+K stays ctrl+k); prefer the base-layout key
    // for non-Latin shortcuts (Cyrillic Ctrl+C is still ctrl+c).
    let shifted_letter = if effective_mod & KITTY_MOD_SHIFT != 0 {
        parsed.shifted_key
    } else {
        None
    };
    let base_codepoint = if let Some(shifted) = shifted_letter {
        shifted
    } else if parsed.codepoint < 128 {
        parsed.codepoint
    } else if let Some(base) = parsed.base_layout_key {
        base
    } else {
        parsed.codepoint
    };
    let base_char = char::from_u32(base_codepoint)?;
    if base_codepoint < 32 || base_codepoint == 127 {
        return None;
    }
    Some(with_modifiers(&base_char.to_string(), effective_mod))
}

/// Parse an xterm `modifyOtherKeys` sequence into a canonical key id.
fn parse_modify_other_keys_key(data: &str) -> Option<String> {
    let (modifier, codepoint) = parse_modify_other_keys(data)?;
    let effective_mod = modifier & !KITTY_LOCK_MASK;
    if effective_mod & !SUPPORTED_MODIFIER_MASK != 0 {
        return None;
    }
    if let Some(name) = named_key(codepoint) {
        return Some(with_modifiers(name, effective_mod));
    }
    let base = char::from_u32(codepoint)?;
    if codepoint < 32 || codepoint == 127 {
        return None;
    }
    Some(with_modifiers(&base.to_string(), effective_mod))
}

/// Parse a legacy CSI/SS3 sequence (arrows, function keys, modifier-prefixed
/// variants) into a canonical key id.
fn parse_legacy_sequence(data: &str) -> Option<String> {
    // SS3: ESC O A/B/C/D = up/down/right/left; H/F = home/end; P-S = F1-F4.
    if let Some(ch) = data.strip_prefix("\x1bO") {
        if ch.len() == 1 {
            let c = ch.chars().next()?;
            let base = match c {
                'A' => "up",
                'B' => "down",
                'C' => "right",
                'D' => "left",
                'H' => "home",
                'F' => "end",
                'P' => "f1",
                'Q' => "f2",
                'R' => "f3",
                'S' => "f4",
                _ => return None,
            };
            return Some(base.to_owned());
        }
        return None;
    }
    // CSI.
    let rest = data.strip_prefix("\x1b[")?;
    // shift+tab
    if rest == "Z" {
        return Some("shift+tab".to_owned());
    }
    // F1-F4 via CSI 1;mod P etc. — rare; handled below by generic numeric.
    if let Some(mod_rest) = rest.strip_prefix("1;") {
        let (mod_str, term) = split_terminal(mod_rest)?;
        let modifier: u32 = mod_str.parse().ok()?;
        let effective = modifier.saturating_sub(1) & !KITTY_LOCK_MASK;
        let base = match term.as_str() {
            "A" => "up",
            "B" => "down",
            "C" => "right",
            "D" => "left",
            "H" => "home",
            "F" => "end",
            "P" => "f1",
            "Q" => "f2",
            "R" => "f3",
            "S" => "f4",
            _ => return None,
        };
        return Some(with_modifiers(base, effective));
    }
    // Bare arrow/home/end: ESC [ A / H / F
    if rest.len() == 1 {
        let base = match rest {
            "A" => "up",
            "B" => "down",
            "C" => "right",
            "D" => "left",
            "H" => "home",
            "F" => "end",
            _ => return None,
        };
        return Some(base.to_owned());
    }
    // Numeric: ESC [ N~ or ESC [ N;mod~
    let (num_str, term) = split_terminal(rest)?;
    let num: u32 = num_str.parse().ok()?;
    let modifier = term
        .split(';')
        .nth(1)
        .and_then(|m| m.parse::<u32>().ok())
        .map(|m| (m.saturating_sub(1)) & !KITTY_LOCK_MASK)
        .unwrap_or(0);
    let base = match (num, term.as_str()) {
        (1, "~") => "home",
        (2, "~") => "insert",
        (3, "~") => "delete",
        (4, "~") => "end",
        (5, "~") => "pageup",
        (6, "~") => "pagedown",
        (7, "~") => "home",
        (8, "~") => "end",
        (15, "~") => "f5",
        (17, "~") => "f6",
        (18, "~") => "f7",
        (19, "~") => "f8",
        (20, "~") => "f9",
        (21, "~") => "f10",
        (23, "~") => "f11",
        (24, "~") => "f12",
        (25, "~") => "f13",
        (26, "~") => "f14",
        (28, "~") => "f15",
        (29, "~") => "f16",
        (31, "~") => "f17",
        (32, "~") => "f18",
        (33, "~") => "f19",
        (34, "~") => "f20",
        _ => return None,
    };
    if modifier == 0 {
        Some(base.to_owned())
    } else {
        Some(with_modifiers(base, modifier))
    }
}

/// Split `rest` into the leading number and the trailing terminator
/// (which may carry `;mod` for modifier-prefixed CSI sequences).
fn split_terminal(rest: &str) -> Option<(&str, String)> {
    let digits_end = rest
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(rest.len());
    let (num_str, tail) = rest.split_at(digits_end);
    if num_str.is_empty() {
        return None;
    }
    // tail like `~`, `;5~`, `A`, `;5A`
    let term = if tail.is_empty() {
        return None;
    } else {
        tail.to_owned()
    };
    Some((num_str, term))
}

/// True when `data` is the raw 0x08 backspace byte and the current session
/// should map it to `ctrl+backspace` (Windows Terminal heuristic).
pub fn matches_raw_backspace(data: &str, expected_modifier: u32, windows_terminal: bool) -> bool {
    if data != "\x08" {
        return false;
    }
    match expected_modifier {
        0 => !windows_terminal,
        4 => windows_terminal,
        _ => false,
    }
}

/// Whether the process is running in a genuine Windows Terminal session:
/// `WT_SESSION` set, no SSH forwarding, no multiplexer.
pub fn is_windows_terminal_session(env: &impl Fn(&str) -> Option<String>) -> bool {
    if env("WT_SESSION").is_none() {
        return false;
    }
    if ["SSH_CONNECTION", "SSH_CLIENT", "SSH_TTY"]
        .iter()
        .any(|k| env(k).is_some())
    {
        return false;
    }
    if ["TMUX", "STY", "ZELLIJ"].iter().any(|k| env(k).is_some()) {
        return false;
    }
    if let Some(term) = env("TERM") {
        let t = term.to_ascii_lowercase();
        if t.starts_with("tmux-") || t.starts_with("screen-") {
            return false;
        }
    }
    true
}

/// Parse terminal input and return the canonical key id, or `None` when the
/// input is not a recognized key sequence.
pub fn parse_key(data: &str) -> Option<String> {
    // Raw 0x08 in a Windows Terminal session is Ctrl+Backspace.
    if matches_raw_backspace(data, 4, is_windows_terminal_session(&|k| std::env::var(k).ok())) {
        return Some("ctrl+backspace".to_owned());
    }
    // Keypad digits/operators decode to their printable text first.
    if let Some(text) = decode_kitty_keypad_text(data) {
        return Some(canonical_key_id(&text));
    }
    parse_key_native(data)
}

/// Core parsing shared by [`parse_key`] and matching: control characters,
/// plain text, ESC-prefixed sequences, CSI-u, modifyOtherKeys.
pub fn parse_key_native(data: &str) -> Option<String> {
    // Single control/plain characters.
    if data.is_empty() {
        return None;
    }
    let bytes = data.as_bytes();
    if bytes.len() == 1 {
        let b = bytes[0];
        return match b {
            // Order matters: specific control bytes before the ctrl-letter range.
            0x00 => Some("ctrl+space".to_owned()),
            0x09 => Some("tab".to_owned()),
            0x0d | 0x0a => Some("enter".to_owned()),
            0x1b => Some("escape".to_owned()),
            0x7f | 0x08 => Some("backspace".to_owned()),
            28 => Some("ctrl+\\".to_owned()),
            29 => Some("ctrl+]".to_owned()),
            30 => Some("ctrl+^".to_owned()),
            31 => Some("ctrl+_".to_owned()),
            0x01..=0x1a => {
                let letter = (b'a' + b - 1) as char;
                Some(format!("ctrl+{letter}"))
            }
            _ => {
                let c = data.chars().next()?;
                Some(canonical_key_id(&c.to_string()))
            }
        };
    }
    // Escape-prefixed sequences.
    if let Some(rest) = data.strip_prefix("\x1b") {
        if rest.is_empty() {
            return Some("escape".to_owned());
        }
        // Legacy Alt+letter: ESC followed by a single printable char.
        if !rest.starts_with('[') && !rest.starts_with('O') {
            if rest.chars().count() == 1 && !rest.chars().next()?.is_ascii_control() {
                let c = rest.chars().next()?;
                // omp static tables: lowercase → "alt+p", uppercase → "alt+shift+p".
                if c.is_ascii_uppercase() {
                    return Some(format!("alt+shift+{}", c.to_ascii_lowercase()));
                }
                return Some(format!("alt+{c}"));
            }
            return None;
        }
        if let Some(k) = parse_kitty_key(data) {
            return Some(k);
        }
        if let Some(k) = parse_modify_other_keys_key(data) {
            return Some(k);
        }
        return parse_legacy_sequence(data);
    }

    // Multi-byte plain text.
    if data.chars().count() == 1 {
        return Some(canonical_key_id(data));
    }
    None
}

/// Match input data against a key id string.
///
/// Supported key ids: `"escape"`, `"tab"`, `"enter"`, `"backspace"`,
/// `"delete"`, `"home"`, `"end"`, `"space"`, arrows, `"ctrl+c"`,
/// `"shift+tab"`, `"alt+enter"`, `"ctrl+alt+x"`, … Case-insensitive on
/// modifiers and base.
pub fn matches_key(data: &str, key_id: &str) -> bool {
    if matches_raw_backspace(data, 4, is_windows_terminal_session(&|k| std::env::var(k).ok())) {
        return canonical_key_id(key_id) == "ctrl+backspace";
    }
    // Keypad fast path: digits/operators compare as their printable text.
    if let Some(text) = decode_kitty_keypad_text(data) {
        return canonical_key_id(key_id) == canonical_key_id(&text);
    }
    match parse_key_native(data) {
        Some(parsed) => matches_canonical(&parsed, &key_id.to_ascii_lowercase()),
        None => false,
    }
}

/// Match a canonical key id against a binding key id (with aliases).
pub fn matches_canonical(parsed: &str, key_id: &str) -> bool {
    let mut aliases = HashSet::new();
    add_key_aliases(&mut aliases, key_id);
    aliases.contains(&canonical_key_id(parsed))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env_map<'a>(pairs: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
        move |k: &str| {
            pairs
                .iter()
                .find(|(key, _)| *key == k)
                .map(|(_, v)| v.to_string())
        }
    }

    #[test]
    fn canonicalizes_modifiers_and_aliases() {
        // omp canonicalKeyId: uppercase single-letter base implies shift even
        // with other modifiers present.
        assert_eq!(canonical_key_id("Ctrl+P"), "ctrl+shift+p");
        assert_eq!(canonical_key_id("ctrl+p"), "ctrl+p");
        assert_eq!(canonical_key_id("ctrl+shift+p"), "ctrl+shift+p");
        assert_eq!(canonical_key_id("A"), "shift+a");
        assert_eq!(canonical_key_id("shift+A"), "shift+a");
        assert_eq!(canonical_key_id("esc"), "escape");
        assert_eq!(canonical_key_id("ESC"), "escape");
        assert_eq!(canonical_key_id("return"), "enter");
        assert_eq!(canonical_key_id("pageUp"), "pageup");
        assert_eq!(canonical_key_id("shift+?"), "shift+?");
        assert_eq!(canonical_key_id("alt+super+backspace"), "alt+super+backspace");
        // Canonical order: ctrl < shift < alt < super.
        assert_eq!(canonical_key_id("super+ctrl+alt+shift+x"), "ctrl+shift+alt+super+x");
    }

    #[test]
    fn aliases_include_shifted_symbols_and_enter() {
        let mut keys = HashSet::new();
        for key in ["esc", "return", "?", "shift+a"] {
            add_key_aliases(&mut keys, key);
        }
        let mut sorted: Vec<String> = keys.into_iter().collect();
        sorted.sort();
        assert_eq!(
            sorted,
            vec!["?", "enter", "escape", "shift+?", "shift+a"]
        );
    }

    #[test]
    fn matches_ctrl_letter() {
        set_kitty_protocol_active(false);
        assert!(matches_key("\u{3}", "ctrl+c"));
        assert!(matches_key("\u{10}", "ctrl+p"));
        assert!(!matches_key("\u{3}", "ctrl+p"));
        assert!(matches_key("\u{3}", "CTRL+C"));
    }

    #[test]
    fn matches_shifted_tab() {
        assert!(matches_key("\x1b[Z", "shift+tab"));
        assert!(!matches_key("\x1b[Z", "tab"));
    }

    #[test]
    fn matches_pageup_mixed_case() {
        assert!(matches_key("\x1b[5~", "pageUp"));
        assert!(matches_key("\x1b[5~", "pageup"));
        assert!(matches_key("\x1b[6~", "pageDown"));
    }

    #[test]
    fn matches_legacy_alt_letter_pairs() {
        set_kitty_protocol_active(true);
        assert!(matches_key("\x1bp", "alt+p"));
        assert!(matches_key("\x1bh", "alt+h"));
        assert!(matches_key("\x1bP", "alt+shift+p"));
        assert!(!matches_key("\x1bp", "alt+shift+p"));
        assert!(matches_key("\x1b[1;3A", "alt+up"));
        assert!(!matches_key("\x1bp", "alt+up"));
        set_kitty_protocol_active(false);
    }

    #[test]
    fn prefers_codepoint_for_latin_letters() {
        set_kitty_protocol_active(true);
        // Dvorak Ctrl+K: codepoint 'k' (107), base layout 'v' (118).
        let dvorak_ctrl_k = "\x1b[107::118;5u";
        assert!(matches_key(dvorak_ctrl_k, "ctrl+k"));
        assert!(!matches_key(dvorak_ctrl_k, "ctrl+v"));
        assert_eq!(parse_key(dvorak_ctrl_k), Some("ctrl+k".to_owned()));
        set_kitty_protocol_active(false);
    }

    #[test]
    fn prefers_codepoint_for_symbol_keys() {
        set_kitty_protocol_active(true);
        // Dvorak Ctrl+/: codepoint '/' (47), base layout '[' (91).
        let dvorak_ctrl_slash = "\x1b[47::91;5u";
        assert!(matches_key(dvorak_ctrl_slash, "ctrl+/"));
        assert!(!matches_key(dvorak_ctrl_slash, "ctrl+["));
        set_kitty_protocol_active(false);
    }

    #[test]
    fn matches_non_latin_shortcuts_by_base_layout_key() {
        set_kitty_protocol_active(true);
        assert!(matches_key("\x1b[1089::99;5u", "ctrl+c"));
        assert!(matches_key("\x1b[1079::112;5u", "ctrl+p"));
        assert!(matches_key("\x1b[1057::99;6u", "ctrl+shift+c"));
        set_kitty_protocol_active(false);
    }

    #[test]
    fn ignores_release_events_matches_repeats() {
        set_kitty_protocol_active(true);
        assert!(matches_key("\x1b[127u", "backspace"));
        assert!(matches_key("\x1b[127;1:2u", "backspace"));
        assert!(!matches_key("\x1b[127;1:3u", "backspace"));
        assert_eq!(parse_key("\x1b[127u"), Some("backspace".to_owned()));
        assert_eq!(parse_key("\x1b[127;1:2u"), Some("backspace".to_owned()));
        assert_eq!(parse_key("\x1b[127;1:3u"), None);
        set_kitty_protocol_active(false);
    }

    #[test]
    fn keypad_digits_are_text_with_or_without_numlock() {
        set_kitty_protocol_active(true);
        for data in ["\x1b[57400u", "\x1b[57400;129u"] {
            assert!(matches_key(data, "1"));
            assert!(!matches_key(data, "end"));
        }
        assert!(matches_key("\x1b[57404u", "5"));
        assert!(!matches_key("\x1b[57404u", "clear"));
        set_kitty_protocol_active(false);
    }

    #[test]
    fn csi_u_named_keys_resolve_to_canonical_names() {
        set_kitty_protocol_active(true);
        assert!(matches_key("\x1b[32u", "space"));
        assert!(matches_key("\x1b[97:65;2u", "shift+a"));
        assert_eq!(parse_key("\x1b[32u"), Some("space".to_owned()));
        set_kitty_protocol_active(false);
    }

    #[test]
    fn keypad_operators_match_printable_symbols() {
        set_kitty_protocol_active(true);
        assert!(matches_key("\x1b[57410u", "/"));
        assert!(matches_key("\x1b[57413;5u", "ctrl++"));
        assert_eq!(parse_key("\x1b[57410u"), Some("/".to_owned()));
        assert_eq!(parse_key("\x1b[57413;5u"), Some("ctrl++".to_owned()));
        set_kitty_protocol_active(false);
    }

    #[test]
    fn modified_keypad_navigation_stays_navigation() {
        set_kitty_protocol_active(true);
        assert!(matches_key("\x1b[57400;133u", "ctrl+end"));
        assert!(!matches_key("\x1b[57400;133u", "1"));
        assert_eq!(parse_key("\x1b[57400;133u"), Some("ctrl+end".to_owned()));
        set_kitty_protocol_active(false);
    }

    #[test]
    fn ghostty_option_backspace_is_super_alt_backspace() {
        set_kitty_protocol_active(true);
        // Modifier 11 (wire) = 10 (mask) = super(8)|alt(2).
        assert!(matches_key("\x1b[127;11u", "super+alt+backspace"));
        assert!(matches_key("\x1b[127;11u", "alt+super+backspace"));
        assert!(!matches_key("\x1b[127;11u", "alt+backspace"));
        assert!(!matches_key("\x1b[127;11u", "backspace"));
        assert_eq!(parse_key("\x1b[127;11u"), Some("alt+super+backspace".to_owned()));
        set_kitty_protocol_active(false);
    }

    #[test]
    fn unsupported_kitty_modifiers_are_ignored() {
        set_kitty_protocol_active(true);
        assert_eq!(parse_key("\x1b[99;17u"), None); // hyper-only
        assert_eq!(parse_key("\x1b[99;33u"), None); // meta-only
        set_kitty_protocol_active(false);
    }

    #[test]
    fn modify_other_keys_normalizes_named_keys() {
        set_kitty_protocol_active(true);
        assert_eq!(parse_key("\x1b[27;1;32~"), Some("space".to_owned()));
        assert_eq!(parse_key("\x1b[27;1;127~"), Some("backspace".to_owned()));
        // omp mod_value is 1-indexed: `;1;` → 0-based modifier 0 → plain "a".
        assert_eq!(parse_key("\x1b[27;1;97~"), Some("a".to_owned()));
        set_kitty_protocol_active(false);
    }

    #[test]
    fn raw_0x08_backspace_disambiguation() {
        // Genuine Windows Terminal session: 0x08 is Ctrl+Backspace.
        let env = env_map(&[("WT_SESSION", "1")]);
        assert!(is_windows_terminal_session(&env));
        assert!(matches_raw_backspace("\x08", 4, true));
        assert!(!matches_raw_backspace("\x08", 0, true));
        // Outside Windows Terminal: plain backspace.
        let env_empty = env_map(&[]);
        assert!(!is_windows_terminal_session(&env_empty));
        assert!(matches_raw_backspace("\x08", 0, false));
        // 0x7f is always plain backspace.
        assert!(matches_raw_backspace("\x7f", 0, true) == false);
        assert!(matches_raw_backspace("\x7f", 4, true) == false);
    }

    #[test]
    fn windows_terminal_session_detection_ignores_ssh_and_multiplexers() {
        let ssh = env_map(&[("WT_SESSION", "1"), ("SSH_CONNECTION", "1.2.3.4 5 6.7.8.9 22")]);
        assert!(!is_windows_terminal_session(&ssh));
        for pairs in [
            vec![("WT_SESSION", "1"), ("TMUX", "/tmp/tmux-1000/default,1,0")],
            vec![("WT_SESSION", "1"), ("STY", "1234.pts-0")],
            vec![("WT_SESSION", "1"), ("ZELLIJ", "0")],
            vec![("WT_SESSION", "1"), ("TERM", "tmux-256color")],
            vec![("WT_SESSION", "1"), ("TERM", "screen-256color")],
        ] {
            let env = env_map(&pairs);
            assert!(!is_windows_terminal_session(&env));
        }
    }

    #[test]
    fn parse_legacy_and_plain_keys() {
        assert_eq!(parse_key("\r"), Some("enter".to_owned()));
        assert_eq!(parse_key("\t"), Some("tab".to_owned()));
        assert_eq!(parse_key("\x1b"), Some("escape".to_owned()));
        assert_eq!(parse_key("\x7f"), Some("backspace".to_owned()));
        assert_eq!(parse_key("a"), Some("a".to_owned()));
        assert_eq!(parse_key("A"), Some("shift+a".to_owned()));
        assert_eq!(parse_key("?"), Some("?".to_owned()));
        assert_eq!(parse_key("\x1b[A"), Some("up".to_owned()));
        assert_eq!(parse_key("\x1b[B"), Some("down".to_owned()));
        assert_eq!(parse_key("\x1b[C"), Some("right".to_owned()));
        assert_eq!(parse_key("\x1b[D"), Some("left".to_owned()));
        assert_eq!(parse_key("\x1b[H"), Some("home".to_owned()));
        assert_eq!(parse_key("\x1b[F"), Some("end".to_owned()));
        assert_eq!(parse_key("\x1b[2~"), Some("insert".to_owned()));
        assert_eq!(parse_key("\x1b[3~"), Some("delete".to_owned()));
        assert_eq!(parse_key("\x1b[1;5A"), Some("ctrl+up".to_owned()));
        assert_eq!(parse_key("\x1b[1;2A"), Some("shift+up".to_owned()));
        assert_eq!(parse_key("\x1bOA"), Some("up".to_owned()));
        assert_eq!(parse_key("\x1bOP"), Some("f1".to_owned()));
        assert_eq!(parse_key("\x1b[15~"), Some("f5".to_owned()));
        assert_eq!(parse_key("\x1b[24~"), Some("f12".to_owned()));
        assert_eq!(parse_key("\x1bP"), Some("alt+shift+p".to_owned()));
        assert_eq!(parse_key("\x1bp"), Some("alt+p".to_owned()));
        assert_eq!(parse_key(""), None);
        assert_eq!(parse_key("\x1b["), None);
    }

    #[test]
    fn extract_printable_text_cases() {
        assert_eq!(extract_printable_text("\x1b[57407u"), Some("8".to_owned()));
        assert_eq!(extract_printable_text("\x1b[57407;129u"), Some("8".to_owned()));
        assert_eq!(extract_printable_text("\x1b[57404u"), Some("5".to_owned()));
        assert_eq!(extract_printable_text("\x1b[57410u"), Some("/".to_owned()));
        assert_eq!(extract_printable_text("\x1b[57413u"), Some("+".to_owned()));
        assert_eq!(extract_printable_text("\x1b[57400;133u"), None);
        assert_eq!(extract_printable_text("\x1b[27;1;127~"), None);
        assert_eq!(extract_printable_text("\x1b[99;9u"), None);
        assert_eq!(extract_printable_text("\x1b[97;9;229u"), None);
        assert_eq!(extract_printable_text("\x1b[97;1;229u"), Some("å".to_owned()));
        assert_eq!(extract_printable_text("hello"), Some("hello".to_owned()));
        assert_eq!(extract_printable_text("\u{3}"), None);
    }
}
