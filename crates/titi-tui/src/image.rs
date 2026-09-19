//! Kitty graphics protocol — image budget with transmit-once/place-many and
//! fresh-first eviction.
//!
//! The [`ImageBudget`] manages a pool of RGBA pixel buffers.  On each frame
//! the caller provides a list of [`Placement`]s; the budget emits
//! **transmit** commands for new image IDs and **place** commands for every
//! placement, including newly transmitted ones.  Once the pixel budget is
//! exceeded the oldest entries are evicted (fresh-first).
//!
//! Contract: `docs/research/tui-renderer/input-capabilities-graphics.md`.

use base64::Engine as _;

/// Max chunk size for kitty transmit payload (base64-encoded).
const CHUNK_BYTES: usize = 4096;

/// A kitty graphics protocol command ready to write to the terminal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KittyCmd {
    /// Transmit pixel data for a new image.
    Transmit(String),
    /// Place an already-transmitted image at a position.
    Place(Placement),
    /// Delete an image and emit height-preserving text fallback.
    Delete {
        id: u32,
        /// Rows of text fallback (one per cell row the image occupied).
        fallback: Vec<String>,
    },
}

/// An image placement on the canvas.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Placement {
    /// Unique image identifier.
    pub id: u32,
    /// RGBA pixel data (required for the first transmit, `None` for
    /// subsequent frames).
    pub pixels: Option<Vec<u8>>,
    /// Pixel width of the image.
    pub pixel_w: u16,
    /// Pixel height of the image.
    pub pixel_h: u16,
    /// Cell column position (1-based).
    pub x: u16,
    /// Cell row position (1-based).
    pub y: u16,
    /// Height in terminal cells (computed from `pixel_h` / cell-height).
    pub cell_h: u16,
}

// ---------------------------------------------------------------------------
// Image budget
// ---------------------------------------------------------------------------

/// An image entry in the budget (sorted most-recently-used first).
#[derive(Debug)]
struct ImageEntry {
    id: u32,
    cell_h: u16,
    size_bytes: usize,
}

/// Manages a pool of transmitted images with fresh-first eviction.
///
/// # Invariants
///
/// - `entries` is sorted MRU-first (index 0 = most recently placed).
/// - Total pixels across all entries never exceeds `max_pixels`.
/// - An image ID that exists in the budget is never re-transmitted.
/// - After eviction, the same ID may be re-transmitted on a future frame.
#[derive(Debug)]
pub struct ImageBudget {
    entries: Vec<ImageEntry>,
    max_pixels: usize,
    /// Monotonic composition key for kitty transmit.
    comp_key: u32,
}

impl ImageBudget {
    /// Create a new budget with the given pixel cap.
    pub fn new(max_pixels: usize) -> Self {
        ImageBudget {
            entries: Vec::new(),
            max_pixels,
            comp_key: 0,
        }
    }

    /// Produce kitty commands for a frame of placements.
    ///
    /// Transmit commands are emitted **once** per new image ID; place
    /// commands are emitted **every frame** for every placement.
    /// Entries whose total pixels exceed `max_pixels` are evicted
    /// (oldest first).
    pub fn frame(&mut self, wanted: &[Placement]) -> Vec<KittyCmd> {
        let mut cmds = Vec::new();

        for p in wanted {
            let is_new = !self.entries.iter().any(|e| e.id == p.id);

            if is_new {
                // Transmit.
                let pixels = match &p.pixels {
                    Some(data) => data.clone(),
                    None => continue,
                };
                let transmit = build_transmit(
                    p.id,
                    &pixels,
                    p.pixel_w,
                    p.pixel_h,
                    &mut self.comp_key,
                );
                cmds.push(KittyCmd::Transmit(transmit));

                // Track in budget.
                let size = pixels.len();
                self.entries.insert(
                    0,
                    ImageEntry {
                        id: p.id,
                        cell_h: p.cell_h,
                        size_bytes: size,
                    },
                );
            }

            // Place every frame.
            cmds.push(KittyCmd::Place(Placement {
                pixels: None,
                ..*p
            }));

            // Move to MRU position.
            if let Some(pos) = self.entries.iter().position(|e| e.id == p.id) {
                let entry = self.entries.remove(pos);
                self.entries.insert(0, entry);
            }
        }

        // Evict oldest until under budget.
        self.evict();

        cmds
    }

    /// Demote (delete) an image by ID, returning a DELETE command and
    /// height-preserving text fallback lines.
    pub fn demote(&mut self, id: u32) -> Vec<KittyCmd> {
        let mut cmds = Vec::new();

        if let Some(pos) = self.entries.iter().position(|e| e.id == id) {
            let entry = self.entries.remove(pos);
            let fallback: Vec<String> = (0..entry.cell_h)
                .map(|_| String::new()) // empty line preserves height
                .collect();
            cmds.push(KittyCmd::Delete {
                id,
                fallback,
            });
        }

        cmds
    }

    /// Current number of entries in the budget.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the budget is empty.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Evict oldest entries until total pixels ≤ max_pixels.
    fn evict(&mut self) {
        let mut total: usize = self.entries.iter().map(|e| e.size_bytes).sum();
        // Evict from the end (oldest) until under budget.
        while total > self.max_pixels && !self.entries.is_empty() {
            if let Some(entry) = self.entries.pop() {
                total = total.saturating_sub(entry.size_bytes);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Command builders
// ---------------------------------------------------------------------------

/// Build a kitty graphics transmit command.
///
/// Format: `\x1b_Gf=32,s=<w>,v=<h>,a=T,c=<key>,m=<0/1>;<base64>\x1b\\`
/// Multiple chunks are emitted when the base64 payload exceeds `CHUNK_BYTES`.
fn build_transmit(
    id: u32,
    pixels: &[u8],
    pixel_w: u16,
    pixel_h: u16,
    comp_key: &mut u32,
) -> String {
    let b64 = base64::engine::general_purpose::STANDARD.encode(pixels);
    let mut result = String::new();
    let chunk_size = CHUNK_BYTES; // base64 chars per chunk
    let total = b64.len();
    let mut offset = 0;

    *comp_key += 1;
    let c = *comp_key;

    while offset < total {
        let end = (offset + chunk_size).min(total);
        let chunk = &b64[offset..end];
        let is_more = end < total;

        let more = if is_more { "1" } else { "0" };
        result.push_str(&format!(
            "\x1b_Gf=32,s={pixel_w},v={pixel_h},a=T,id={id},c={c},m={more};{chunk}\x1b\\"
        ));

        offset = end;
    }

    result
}

/// Build a kitty graphics place command.
///
/// Format: `\x1b_Ga=p,id=<id>[,x=<col>,y=<row>];\x1b\\`
pub fn build_place(id: u32, x: u16, y: u16) -> String {
    format!("\x1b_Ga=p,id={id},x={x},y={y};\x1b\\")
}

/// Build a kitty graphics delete command.
///
/// Format: `\x1b_Ga=d,id=<id>;\x1b\\`
pub fn build_delete(id: u32) -> String {
    format!("\x1b_Ga=d,id={id};\x1b\\")
}


// ---------------------------------------------------------------------------
// Unicode placeholders (`U=1` + U+10EEEE)
// ---------------------------------------------------------------------------

/// Kitty Unicode placeholder base character (U+10EEEE, Plane 16 PUA).
pub const KITTY_PLACEHOLDER: char = '\u{10EEEE}';

/// Row/column diacritics (Unicode combining class 230). Index `i` → codepoint.
/// Kitty `gen/rowcolumn-diacritics.txt` (Unicode 6.0.0 NSM set), 297 entries.
const ROWCOLUMN_DIACRITICS: &[u32] = &[
    0x305, 0x30D, 0x30E, 0x310, 0x312, 0x33D, 0x33E, 0x33F, 0x346, 0x34A, 0x34B, 0x34C, 0x350,
    0x351, 0x352, 0x357, 0x35B, 0x363, 0x364, 0x365, 0x366, 0x367, 0x368, 0x369, 0x36A, 0x36B,
    0x36C, 0x36D, 0x36E, 0x36F, 0x483, 0x484, 0x485, 0x486, 0x487, 0x592, 0x593, 0x594, 0x595,
    0x597, 0x598, 0x599, 0x59C, 0x59D, 0x59E, 0x59F, 0x5A0, 0x5A1, 0x5A8, 0x5A9, 0x5AB, 0x5AC,
    0x5AF, 0x5C4, 0x610, 0x611, 0x612, 0x613, 0x614, 0x615, 0x616, 0x617, 0x657, 0x658, 0x659,
    0x65A, 0x65B, 0x65D, 0x65E, 0x6D6, 0x6D7, 0x6D8, 0x6D9, 0x6DA, 0x6DB, 0x6DC, 0x6DF, 0x6E0,
    0x6E1, 0x6E2, 0x6E4, 0x6E7, 0x6E8, 0x6EB, 0x6EC, 0x730, 0x732, 0x733, 0x735, 0x736, 0x73A,
    0x73D, 0x73F, 0x740, 0x741, 0x743, 0x745, 0x747, 0x749, 0x74A, 0x7EB, 0x7EC, 0x7ED, 0x7EE,
    0x7EF, 0x7F0, 0x7F1, 0x7F3, 0x816, 0x817, 0x818, 0x819, 0x81B, 0x81C, 0x81D, 0x81E, 0x81F,
    0x820, 0x821, 0x822, 0x823, 0x825, 0x826, 0x827, 0x829, 0x82A, 0x82B, 0x82C, 0x82D, 0x951,
    0x953, 0x954, 0xF82, 0xF83, 0xF86, 0xF87, 0x135D, 0x135E, 0x135F, 0x17DD, 0x193A, 0x1A17,
    0x1A75, 0x1A76, 0x1A77, 0x1A78, 0x1A79, 0x1A7A, 0x1A7B, 0x1A7C, 0x1B6B, 0x1B6D, 0x1B6E, 0x1B6F,
    0x1B70, 0x1B71, 0x1B72, 0x1B73, 0x1CD0, 0x1CD1, 0x1CD2, 0x1CDA, 0x1CDB, 0x1CE0, 0x1DC0, 0x1DC1,
    0x1DC3, 0x1DC4, 0x1DC5, 0x1DC6, 0x1DC7, 0x1DC8, 0x1DC9, 0x1DCB, 0x1DCC, 0x1DD1, 0x1DD2, 0x1DD3,
    0x1DD4, 0x1DD5, 0x1DD6, 0x1DD7, 0x1DD8, 0x1DD9, 0x1DDA, 0x1DDB, 0x1DDC, 0x1DDD, 0x1DDE, 0x1DDF,
    0x1DE0, 0x1DE1, 0x1DE2, 0x1DE3, 0x1DE4, 0x1DE5, 0x1DE6, 0x1DFE, 0x20D0, 0x20D1, 0x20D4, 0x20D5,
    0x20D6, 0x20D7, 0x20DB, 0x20DC, 0x20E1, 0x20E7, 0x20E9, 0x20F0, 0x2CEF, 0x2CF0, 0x2CF1, 0x2DE0,
    0x2DE1, 0x2DE2, 0x2DE3, 0x2DE4, 0x2DE5, 0x2DE6, 0x2DE7, 0x2DE8, 0x2DE9, 0x2DEA, 0x2DEB, 0x2DEC,
    0x2DED, 0x2DEE, 0x2DEF, 0x2DF0, 0x2DF1, 0x2DF2, 0x2DF3, 0x2DF4, 0x2DF5, 0x2DF6, 0x2DF7, 0x2DF8,
    0x2DF9, 0x2DFA, 0x2DFB, 0x2DFC, 0x2DFD, 0x2DFE, 0x2DFF, 0xA66F, 0xA67C, 0xA67D, 0xA6F0, 0xA6F1,
    0xA8E0, 0xA8E1, 0xA8E2, 0xA8E3, 0xA8E4, 0xA8E5, 0xA8E6, 0xA8E7, 0xA8E8, 0xA8E9, 0xA8EA, 0xA8EB,
    0xA8EC, 0xA8ED, 0xA8EE, 0xA8EF, 0xA8F0, 0xA8F1, 0xAAB0, 0xAAB2, 0xAAB3, 0xAAB7, 0xAAB8, 0xAABE,
    0xAABF, 0xAAC1, 0xFE20, 0xFE21, 0xFE22, 0xFE23, 0xFE24, 0xFE25, 0xFE26, 0x10A0F, 0x10A38,
    0x1D185, 0x1D186, 0x1D187, 0x1D188, 0x1D189, 0x1D1AA, 0x1D1AB, 0x1D1AC, 0x1D1AD, 0x1D242,
    0x1D243, 0x1D244,
];

/// Largest row/column index expressible with the diacritic table.
pub const KITTY_PLACEHOLDER_MAX_CELLS: usize = 297;

/// Env + terminal identity used to detect Unicode-placeholder support.
#[derive(Debug, Clone, Default)]
pub struct PlaceholderDetect {
    /// Normalized terminal id (`kitty` / `ghostty` / other).
    pub terminal_id: String,
    /// `TMUX` is set.
    pub tmux: bool,
    /// `PI_NO_KITTY_PLACEHOLDERS` / `TITI_NO_KITTY_PLACEHOLDERS`.
    pub no_placeholders: Option<String>,
    /// `PI_KITTY_PLACEHOLDERS` / `TITI_KITTY_PLACEHOLDERS`.
    pub placeholders: Option<String>,
    /// `PI_FORCE_IMAGE_PROTOCOL` / `TITI_FORCE_IMAGE_PROTOCOL`.
    pub force_image_protocol: Option<String>,
}

impl PlaceholderDetect {
    /// Snapshot process env. `terminal_id` is normalized from `TERM_PROGRAM` / `TERM`.
    pub fn from_env() -> Self {
        let term_program = std::env::var("TERM_PROGRAM").ok();
        let term = std::env::var("TERM").ok();
        PlaceholderDetect {
            terminal_id: kitty_terminal_id(term_program.as_deref(), term.as_deref()),
            tmux: std::env::var("TMUX").is_ok(),
            no_placeholders: first_env(&["PI_NO_KITTY_PLACEHOLDERS", "TITI_NO_KITTY_PLACEHOLDERS"]),
            placeholders: first_env(&["PI_KITTY_PLACEHOLDERS", "TITI_KITTY_PLACEHOLDERS"]),
            force_image_protocol: first_env(&["PI_FORCE_IMAGE_PROTOCOL", "TITI_FORCE_IMAGE_PROTOCOL"]),
        }
    }

    /// Whether this terminal should render `U=1` + U+10EEEE grids.
    pub fn supported(&self) -> bool {
        detect_kitty_unicode_placeholders_support(self)
    }
}

fn first_env(keys: &[&str]) -> Option<String> {
    for key in keys {
        if let Ok(v) = std::env::var(key) {
            return Some(v);
        }
    }
    None
}

/// Normalize TERM / TERM_PROGRAM to OMP's `terminalId` (`kitty` / `ghostty` / raw).
pub fn kitty_terminal_id(term_program: Option<&str>, term: Option<&str>) -> String {
    let tp = term_program.unwrap_or("").to_ascii_lowercase();
    if tp.contains("kitty") {
        return "kitty".into();
    }
    if tp.contains("ghostty") {
        return "ghostty".into();
    }
    let t = term.unwrap_or("").to_ascii_lowercase();
    if t.contains("kitty") {
        return "kitty".into();
    }
    if t.contains("ghostty") {
        return "ghostty".into();
    }
    t
}

fn env_on(raw: Option<&str>) -> bool {
    matches!(
        raw.map(str::trim).map(str::to_ascii_lowercase).as_deref(),
        Some("1" | "true" | "on" | "yes" | "y")
    )
}

fn env_off(raw: Option<&str>) -> bool {
    matches!(
        raw.map(str::trim).map(str::to_ascii_lowercase).as_deref(),
        Some("0" | "false" | "off" | "no" | "n")
    )
}

/// OMP `detectKittyUnicodePlaceholdersSupport`.
pub fn detect_kitty_unicode_placeholders_support(env: &PlaceholderDetect) -> bool {
    if env_on(env.no_placeholders.as_deref()) {
        return false;
    }
    if env_on(env.placeholders.as_deref()) {
        return true;
    }
    if env_off(env.placeholders.as_deref()) {
        return false;
    }
    if env.tmux
        && env
            .force_image_protocol
            .as_deref()
            .map(str::trim)
            .map(str::to_ascii_lowercase)
            .as_deref()
            == Some("kitty")
    {
        return true;
    }
    env.terminal_id == "kitty" || env.terminal_id == "ghostty"
}

/// Whether a `columns`×`rows` placeholder grid fits the diacritic table.
pub fn kitty_placeholders_fit(columns: u16, rows: u16) -> bool {
    columns >= 1
        && rows >= 1
        && (columns as usize) <= KITTY_PLACEHOLDER_MAX_CELLS
        && (rows as usize) <= KITTY_PLACEHOLDER_MAX_CELLS
}

fn diacritic(index: usize) -> String {
    ROWCOLUMN_DIACRITICS
        .get(index)
        .and_then(|cp| char::from_u32(*cp))
        .map(String::from)
        .unwrap_or_default()
}

/// tmux DCS passthrough (`ESC P tmux ; … ST`); identity when `tmux` is false.
pub fn wrap_tmux_passthrough_if_needed(payload: &str, tmux: bool) -> String {
    if !tmux {
        return payload.to_owned();
    }
    let escaped = payload.replace('\u{1b}', "\u{1b}\u{1b}");
    format!("\x1bPtmux;{escaped}\x1b\\")
}

/// Virtual placement APC (`a=p,U=1`).
pub fn encode_kitty_virtual_placement(
    image_id: u32,
    placement_id: Option<u32>,
    columns: u16,
    rows: u16,
    tmux: bool,
) -> String {
    let mut params = format!("a=p,U=1,q=2,i={image_id}");
    if let Some(p) = placement_id {
        if p != 0 {
            params.push_str(&format!(",p={p}"));
        }
    }
    params.push_str(&format!(",c={columns},r={rows}"));
    wrap_tmux_passthrough_if_needed(&format!("\x1b_G{params}\x1b\\"), tmux)
}

/// Placeholder cell grid: one string per row (`rows` lines).
pub fn encode_kitty_placeholder_grid(
    image_id: u32,
    placement_id: Option<u32>,
    columns: u16,
    rows: u16,
) -> Vec<String> {
    let fg = format!(
        "\x1b[38;2;{};{};{}m",
        (image_id >> 16) & 0xff,
        (image_id >> 8) & 0xff,
        image_id & 0xff
    );
    let underline = match placement_id {
        Some(p) if p != 0 => format!(
            "\x1b[58:2::{}:{}:{}m",
            (p >> 16) & 0xff,
            (p >> 8) & 0xff,
            p & 0xff
        ),
        _ => String::new(),
    };
    let reset = "\x1b[39;59m";
    let lead = format!("{fg}{underline}");
    let mut out = Vec::with_capacity(rows as usize);
    for r in 0..rows as usize {
        let row_diacritic = diacritic(r);
        let mut row = lead.clone();
        for c in 0..columns as usize {
            row.push(KITTY_PLACEHOLDER);
            row.push_str(&row_diacritic);
            row.push_str(&diacritic(c));
        }
        row.push_str(reset);
        out.push(row);
    }
    out
}

/// Virtual-placement APC prefixes line 0; returns exactly `rows` lines.
pub fn render_kitty_placeholder_lines(
    image_id: u32,
    placement_id: Option<u32>,
    columns: u16,
    rows: u16,
    tmux: bool,
) -> Option<Vec<String>> {
    if !kitty_placeholders_fit(columns, rows) {
        return None;
    }
    let mut grid = encode_kitty_placeholder_grid(image_id, placement_id, columns, rows);
    if let Some(first) = grid.first_mut() {
        let apc = encode_kitty_virtual_placement(image_id, placement_id, columns, rows, tmux);
        first.insert_str(0, &apc);
    }
    Some(grid)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Create a trivial RGBA pixel buffer of `w × h` pixels.
    fn rgba(w: u16, h: u16, r: u8, g: u8, b: u8, a: u8) -> Vec<u8> {
        let mut data = Vec::with_capacity(w as usize * h as usize * 4);
        for _ in 0..(w as u32 * h as u32) {
            data.extend_from_slice(&[r, g, b, a]);
        }
        data
    }

    // ---- Transmit-once ----------------------------------------------------

    #[test]
    fn transmit_once_across_two_frames() {
        // Frame 1: place image 1 → must transmit + place.
        let mut budget = ImageBudget::new(1_000_000);
        let pixels = rgba(10, 10, 255, 0, 0, 255);
        let p = Placement {
            id: 1,
            pixels: Some(pixels),
            pixel_w: 10,
            pixel_h: 10,
            x: 1,
            y: 1,
            cell_h: 2,
        };

        let cmds = budget.frame(&[p.clone()]);
        assert_eq!(cmds.len(), 2, "frame 1: transmit + place");
        assert!(matches!(cmds[0], KittyCmd::Transmit(_)), "frame 1 cmd 0: transmit");
        assert!(matches!(cmds[1], KittyCmd::Place(_)), "frame 1 cmd 1: place");

        // Frame 2: same placement, no pixel data → place only, no transmit.
        let p2 = Placement {
            pixels: None,
            ..p
        };
        let cmds = budget.frame(&[p2]);
        assert_eq!(cmds.len(), 1, "frame 2: place only");
        assert!(matches!(cmds[0], KittyCmd::Place(_)), "frame 2 cmd 0: place");
    }

    #[test]
    fn transmit_two_images() {
        let mut budget = ImageBudget::new(1_000_000);
        let p1 = Placement {
            id: 1,
            pixels: Some(rgba(5, 5, 0, 255, 0, 255)),
            pixel_w: 5,
            pixel_h: 5,
            x: 1,
            y: 1,
            cell_h: 1,
        };
        let p2 = Placement {
            id: 2,
            pixels: Some(rgba(5, 5, 0, 0, 255, 255)),
            pixel_w: 5,
            pixel_h: 5,
            x: 10,
            y: 1,
            cell_h: 1,
        };

        let cmds = budget.frame(&[p1, p2]);
        assert_eq!(cmds.len(), 4, "transmit(1) + place(1) + transmit(2) + place(2)");
    }

    // ---- Demote -----------------------------------------------------------

    #[test]
    fn demote_emits_delete_and_fallback() {
        let mut budget = ImageBudget::new(1_000_000);
        let p = Placement {
            id: 1,
            pixels: Some(rgba(20, 10, 0, 0, 0, 255)),
            pixel_w: 20,
            pixel_h: 10,
            x: 1,
            y: 1,
            cell_h: 2,
        };
        budget.frame(&[p]);

        let cmds = budget.demote(1);
        assert_eq!(cmds.len(), 1, "demote produces one command");
        match &cmds[0] {
            KittyCmd::Delete { id, fallback } => {
                assert_eq!(*id, 1);
                assert_eq!(fallback.len(), 2, "fallback height must match cell_h");
                assert!(fallback.iter().all(|l| l.is_empty()));
            }
            other => panic!("expected Delete, got {other:?}"),
        }
    }

    #[test]
    fn demote_unknown_id_noop() {
        let mut budget = ImageBudget::new(1_000_000);
        let cmds = budget.demote(99);
        assert!(cmds.is_empty());
    }

    // ---- Eviction ---------------------------------------------------------

    #[test]
    fn eviction_fresh_first() {
        // Budget of 500 bytes → fits only one ~400 byte image.
        let mut budget = ImageBudget::new(500);
        let p1 = Placement {
            id: 1,
            pixels: Some(rgba(10, 10, 255, 0, 0, 255)), // 10*10*4 = 400 bytes
            pixel_w: 10,
            pixel_h: 10,
            x: 1,
            y: 1,
            cell_h: 2,
        };
        let p2 = Placement {
            id: 2,
            pixels: Some(rgba(10, 10, 0, 255, 0, 255)), // 400 bytes
            pixel_w: 10,
            pixel_h: 10,
            x: 10,
            y: 1,
            cell_h: 2,
        };

        // Insert both in one frame.
        let cmds = budget.frame(&[p1, p2]);
        // Both transmit + place = 4 commands, but the eviction happens
        // after frame, so we see both transmits.
        assert_eq!(cmds.len(), 4);
        // After eviction: only the last (MRU) entry should remain.
        assert_eq!(budget.len(), 1, "only one entry survives eviction");
        assert_eq!(budget.entries[0].id, 2, "MRU id=2 survives");
    }

    // ---- History not replayed ---------------------------------------------

    #[test]
    fn history_not_replayed_after_demote() {
        // Demote removes the entry; subsequent frame with the same id
        // re-transmits it (no history caching).
        let mut budget = ImageBudget::new(1_000_000);
        let p = Placement {
            id: 1,
            pixels: Some(rgba(5, 5, 0, 0, 0, 255)),
            pixel_w: 5,
            pixel_h: 5,
            x: 1,
            y: 1,
            cell_h: 1,
        };
        budget.frame(&[p.clone()]);
        budget.demote(1);
        assert!(budget.is_empty());

        // Re-insert same id — must re-transmit.
        let p2 = Placement {
            pixels: Some(rgba(5, 5, 0, 0, 0, 255)),
            ..p
        };
        let cmds = budget.frame(&[p2]);
        assert_eq!(cmds.len(), 2, "re-transmit + place");
        assert!(matches!(cmds[0], KittyCmd::Transmit(_)), "re-transmit");
    }

    // ---- Command builders -------------------------------------------------

    #[test]
    fn build_transmit_contains_base64() {
        let pixels = rgba(1, 1, 0x12, 0x34, 0x56, 0x78);
        let mut key = 0;
        let cmd = build_transmit(1, &pixels, 1, 1, &mut key);
        assert!(cmd.starts_with("\x1b_Gf=32,s=1,v=1,a=T,id=1,c=1,m=0;"));
        assert!(cmd.ends_with("\x1b\\"));
        // Base64 decodes back to 4 bytes.
        let b64 = cmd
            .split(';')
            .nth(1)
            .and_then(|s| s.strip_suffix("\x1b\\"))
            .unwrap();
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(b64)
            .unwrap();
        assert_eq!(decoded, pixels);
    }

    #[test]
    fn build_place_format() {
        let cmd = build_place(1, 5, 10);
        assert_eq!(cmd, "\x1b_Ga=p,id=1,x=5,y=10;\x1b\\");
    }

    #[test]
    fn build_delete_format() {
        let cmd = build_delete(1);
        assert_eq!(cmd, "\x1b_Ga=d,id=1;\x1b\\");
    }

    #[test]
    fn build_transmit_chunks_large_image() {
        // Create a 64×64 RGBA image = 16384 bytes → ~21845 base64 chars.
        let pixels = rgba(64, 64, 0xff, 0, 0, 0xff);
        let mut key = 0;
        let raw = build_transmit(1, &pixels, 64, 64, &mut key);
        // Chunk size is 4096 base64 chars, so we expect ceil(21845/4096) = 6 chunks.
        let chunk_count = raw.matches("\x1b_G").count();
        assert!(chunk_count >= 5, "expected ~6 chunks, got {chunk_count}");

        // Verify the chunk chain: m=1 for all but the last.
        let parts: Vec<&str> = raw.split("\x1b\\").collect();
        for (i, part) in parts.iter().enumerate() {
            if part.is_empty() {
                continue;
            }
            // Last part should have m=0 before the final \x1b\
            let is_last = i == parts.len() - 2;
            if is_last {
                assert!(part.contains(",m=0;"), "last chunk must have m=0");
            } else {
                // Non-last (before the trailing empty part) should have m=1
                // But the last element is the trailing empty string from split,
                // so the actual last part is parts.len()-2.
                if i < parts.len() - 2 {
                    assert!(part.contains(",m=1;"), "chunk {i} must have m=1");
                }
            }
        }
    }

    // ---- Unicode placeholders --------------------------------------------

    #[test]
    fn placeholder_is_plane16_pua() {
        assert_eq!(KITTY_PLACEHOLDER as u32, 0x10EEEE);
        assert_eq!(ROWCOLUMN_DIACRITICS.len(), KITTY_PLACEHOLDER_MAX_CELLS);
        assert_eq!(ROWCOLUMN_DIACRITICS[0], 0x305);
        assert_eq!(*ROWCOLUMN_DIACRITICS.last().unwrap(), 0x1D244);
    }

    #[test]
    fn placeholders_fit_bounds() {
        assert!(kitty_placeholders_fit(1, 1));
        assert!(kitty_placeholders_fit(297, 297));
        assert!(!kitty_placeholders_fit(0, 1));
        assert!(!kitty_placeholders_fit(1, 0));
        assert!(!kitty_placeholders_fit(298, 1));
        assert!(!kitty_placeholders_fit(1, 298));
    }

    #[test]
    fn virtual_placement_apc() {
        let apc = encode_kitty_virtual_placement(42, None, 3, 2, false);
        assert_eq!(apc, "\x1b_Ga=p,U=1,q=2,i=42,c=3,r=2\x1b\\");
        let with_p = encode_kitty_virtual_placement(42, Some(7), 3, 2, false);
        assert!(with_p.contains(",p=7,"));
        let tmux = encode_kitty_virtual_placement(1, None, 1, 1, true);
        assert!(tmux.starts_with("\x1bPtmux;"));
        assert!(tmux.contains("\x1b\x1b_G"));
        assert!(tmux.ends_with("\x1b\\"));
    }

    #[test]
    fn placeholder_grid_names_every_cell() {
        let grid = encode_kitty_placeholder_grid(0x010203, Some(0x0A0B0C), 2, 2);
        assert_eq!(grid.len(), 2);
        assert!(grid[0].starts_with("\x1b[38;2;1;2;3m\x1b[58:2::10:11:12m"));
        assert!(grid[0].contains(KITTY_PLACEHOLDER));
        assert!(grid[0].ends_with("\x1b[39;59m"));
        let cell0 = format!(
            "{}{}{}",
            KITTY_PLACEHOLDER,
            char::from_u32(ROWCOLUMN_DIACRITICS[0]).unwrap(),
            char::from_u32(ROWCOLUMN_DIACRITICS[0]).unwrap()
        );
        let cell1 = format!(
            "{}{}{}",
            KITTY_PLACEHOLDER,
            char::from_u32(ROWCOLUMN_DIACRITICS[0]).unwrap(),
            char::from_u32(ROWCOLUMN_DIACRITICS[1]).unwrap()
        );
        assert!(grid[0].contains(&cell0), "{}", grid[0]);
        assert!(grid[0].contains(&cell1), "{}", grid[0]);
        let cell_r1c0 = format!(
            "{}{}{}",
            KITTY_PLACEHOLDER,
            char::from_u32(ROWCOLUMN_DIACRITICS[1]).unwrap(),
            char::from_u32(ROWCOLUMN_DIACRITICS[0]).unwrap()
        );
        assert!(grid[1].contains(&cell_r1c0), "{}", grid[1]);
    }

    #[test]
    fn render_prefixes_line0_and_rejects_overflow() {
        let lines = render_kitty_placeholder_lines(9, None, 2, 2, false).unwrap();
        assert_eq!(lines.len(), 2);
        assert!(lines[0].starts_with("\x1b_Ga=p,U=1,q=2,i=9,c=2,r=2\x1b\\"));
        assert!(!lines[1].starts_with("\x1b_G"));
        assert!(render_kitty_placeholder_lines(1, None, 298, 1, false).is_none());
    }

    #[test]
    fn detect_placeholders_kitty_ghostty_and_env() {
        let kitty = PlaceholderDetect {
            terminal_id: "kitty".into(),
            ..PlaceholderDetect::default()
        };
        assert!(kitty.supported());
        let ghostty = PlaceholderDetect {
            terminal_id: "ghostty".into(),
            ..PlaceholderDetect::default()
        };
        assert!(ghostty.supported());
        let wez = PlaceholderDetect {
            terminal_id: "wezterm".into(),
            ..PlaceholderDetect::default()
        };
        assert!(!wez.supported());
        let off = PlaceholderDetect {
            terminal_id: "kitty".into(),
            no_placeholders: Some("1".into()),
            ..PlaceholderDetect::default()
        };
        assert!(!off.supported());
        let force = PlaceholderDetect {
            terminal_id: "wezterm".into(),
            placeholders: Some("true".into()),
            ..PlaceholderDetect::default()
        };
        assert!(force.supported());
        let force_off = PlaceholderDetect {
            terminal_id: "kitty".into(),
            placeholders: Some("0".into()),
            ..PlaceholderDetect::default()
        };
        assert!(!force_off.supported());
        let tmux_force = PlaceholderDetect {
            terminal_id: "screen".into(),
            tmux: true,
            force_image_protocol: Some("kitty".into()),
            ..PlaceholderDetect::default()
        };
        assert!(tmux_force.supported());
        let tmux_auto = PlaceholderDetect {
            terminal_id: "screen".into(),
            tmux: true,
            ..PlaceholderDetect::default()
        };
        assert!(!tmux_auto.supported());
        assert_eq!(kitty_terminal_id(Some("iTerm.app"), Some("xterm-kitty")), "kitty");
        assert_eq!(kitty_terminal_id(Some("ghostty"), Some("xterm-256color")), "ghostty");
    }
}
