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
}