//! Color conversion and ANSI escape generation for the theme module.
//!
//! All parsing is hex-based (`#RGB`, `#RRGGBB`, bare `RRGGBB`); numeric
//! values are terminal 256-color palette indices handled at the escape
//! layer. Conversions between RGB and HSV drive theme hue/saturation
//! adjustments; WCAG 2.x relative luminance drives contrast decisions.

use serde_json::Value;

/// Terminal color capability.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorMode {
    /// 24-bit RGB escape sequences (`\x1b[38;2;R;G;Bm`).
    Truecolor,
    /// 256-color palette escape sequences (`\x1b[38;5;Nm`).
    Color256,
}

/// An RGB triple with 8-bit channels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rgb {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

/// Basic ANSI 16-color RGB values, indices 0-15 of the 256-color palette.
const ANSI_16: [Rgb; 16] = [
    Rgb { r: 0, g: 0, b: 0 },   // 0 black
    Rgb { r: 128, g: 0, b: 0 }, // 1 maroon
    Rgb { r: 0, g: 128, b: 0 }, // 2 green
    Rgb {
        r: 128,
        g: 128,
        b: 0,
    }, // 3 olive
    Rgb { r: 0, g: 0, b: 128 }, // 4 navy
    Rgb {
        r: 128,
        g: 0,
        b: 128,
    }, // 5 purple
    Rgb {
        r: 0,
        g: 128,
        b: 128,
    }, // 6 teal
    Rgb {
        r: 192,
        g: 192,
        b: 192,
    }, // 7 silver
    Rgb {
        r: 128,
        g: 128,
        b: 128,
    }, // 8 gray
    Rgb { r: 255, g: 0, b: 0 }, // 9 red
    Rgb { r: 0, g: 255, b: 0 }, // 10 lime
    Rgb {
        r: 255,
        g: 255,
        b: 0,
    }, // 11 yellow
    Rgb { r: 0, g: 0, b: 255 }, // 12 blue
    Rgb {
        b: 255,
        r: 255,
        g: 0,
    }, // 13 fuchsia
    Rgb {
        r: 0,
        g: 255,
        b: 255,
    }, // 14 aqua
    Rgb {
        r: 255,
        g: 255,
        b: 255,
    }, // 15 white
];

/// Cube steps for 256-color palette entries 16-231.
const CUBE_STEPS: [u8; 6] = [0, 95, 135, 175, 215, 255];

/// Convert a hex string (`#RGB`, `#RRGGBB`, `RGB`, `RRGGBB`) to [`Rgb`].
///
/// Returns `None` for invalid input: wrong length, non-hex digits, or the
/// 4/5/7/8-digit forms which are not supported here.
pub fn hex_to_rgb(hex: &str) -> Option<Rgb> {
    let digits = hex.strip_prefix('#').unwrap_or(hex);
    match digits.len() {
        3 => {
            let mut channels = [0u8; 3];
            for (slot, ch) in digits.chars().enumerate() {
                let v = u8::from_str_radix(&ch.to_string(), 16).ok()?;
                channels[slot] = v * 17; // 0xF -> 0xFF
            }
            Some(Rgb {
                r: channels[0],
                g: channels[1],
                b: channels[2],
            })
        }
        6 => {
            let (pairs, _) = digits.as_bytes().as_chunks::<2>();
            let mut channels = [0u8; 3];
            for (slot, pair) in pairs.iter().enumerate() {
                let hi = u8::from_str_radix(std::str::from_utf8(&pair[..1]).ok()?, 16).ok()?;
                let lo = u8::from_str_radix(std::str::from_utf8(&pair[1..]).ok()?, 16).ok()?;
                channels[slot] = hi * 16 + lo;
            }
            Some(Rgb {
                r: channels[0],
                g: channels[1],
                b: channels[2],
            })
        }
        _ => None,
    }
}

/// Format [`Rgb`] as `#RRGGBB` (lowercase hex).
pub fn rgb_to_hex(rgb: Rgb) -> String {
    format!("#{:02x}{:02x}{:02x}", rgb.r, rgb.g, rgb.b)
}

/// Convert [`Rgb`] to HSV: `(h in 0..=360, s in 0..=1, v in 0..=1)`.
pub fn rgb_to_hsv(rgb: Rgb) -> (f64, f64, f64) {
    let r = f64::from(rgb.r) / 255.0;
    let g = f64::from(rgb.g) / 255.0;
    let b = f64::from(rgb.b) / 255.0;
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let delta = max - min;

    let h = if delta == 0.0 {
        0.0
    } else if max == r {
        60.0 * (((g - b) / delta) % 6.0)
    } else if max == g {
        60.0 * (((b - r) / delta) + 2.0)
    } else {
        60.0 * (((r - g) / delta) + 4.0)
    };
    let h = if h < 0.0 { h + 360.0 } else { h };

    let s = if max == 0.0 { 0.0 } else { delta / max };
    (h, s, max)
}

/// Convert HSV to [`Rgb`], normalizing `h` into `0..=360` and clamping
/// `s`/`v` to `0..=1`.
pub fn hsv_to_rgb(h: f64, s: f64, v: f64) -> Rgb {
    let h = h.rem_euclid(360.0);
    let s = s.clamp(0.0, 1.0);
    let v = v.clamp(0.0, 1.0);

    let c = v * s;
    let x = c * (1.0 - ((h / 60.0) % 2.0 - 1.0).abs());
    let m = v - c;
    let (r1, g1, b1) = match (h / 60.0) as u8 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    Rgb {
        r: ((r1 + m) * 255.0).round() as u8,
        g: ((g1 + m) * 255.0).round() as u8,
        b: ((b1 + m) * 255.0).round() as u8,
    }
}

/// Shift a hex color in HSV space: hue by `dh` degrees, saturation and
/// value multiplied by `ds`/`dv`. Returns the new `#RRGGBB` string, or
/// `None` if `hex` is unparseable.
pub fn adjust_hsv(hex: &str, dh: f64, ds: f64, dv: f64) -> Option<String> {
    let rgb = hex_to_rgb(hex)?;
    let (h, s, v) = rgb_to_hsv(rgb);
    let (nh, ns, nv) = (h + dh, (s * ds).clamp(0.0, 1.0), (v * dv).clamp(0.0, 1.0));
    Some(rgb_to_hex(hsv_to_rgb(nh, ns, nv)))
}

/// Look up the RGB value for a 256-color palette index.
///
/// 0-15 are the basic ANSI colors, 16-231 the 6x6x6 color cube,
/// 232-255 the grayscale ramp (`8 + (index - 232) * 10`).
pub fn palette_to_rgb(index: u8) -> Rgb {
    match index {
        0..=15 => ANSI_16[usize::from(index)],
        16..=231 => {
            let idx = usize::from(index - 16);
            let r = CUBE_STEPS[idx / 36];
            let g = CUBE_STEPS[(idx / 6) % 6];
            let b = CUBE_STEPS[idx % 6];
            Rgb { r, g, b }
        }
        _ => {
            let gray = 8 + (u16::from(index - 232)) * 10;
            Rgb {
                r: gray as u8,
                g: gray as u8,
                b: gray as u8,
            }
        }
    }
}

/// Look up a 256-color palette index and format it as `#RRGGBB`.
pub fn ansi256_to_hex(index: u8) -> String {
    rgb_to_hex(palette_to_rgb(index))
}

/// BT.709 gamma-encoded luma, normalized to `0.0..=1.0`.
///
/// Returns `None` for non-hex input.
pub fn color_luma(value: &str) -> Option<f64> {
    let rgb = hex_to_rgb(value)?;
    Some(
        (0.2126 * f64::from(rgb.r) + 0.7152 * f64::from(rgb.g) + 0.0722 * f64::from(rgb.b)) / 255.0,
    )
}

/// WCAG 2.x relative luminance, normalized to `0.0..=1.0`.
///
/// Linearizes each channel against the sRGB transfer function, then
/// applies BT.709 weights. Returns `None` for non-hex input.
pub fn relative_luminance(value: &str) -> Option<f64> {
    let rgb = hex_to_rgb(value)?;
    let lin = |c: u8| -> f64 {
        let c = f64::from(c) / 255.0;
        if c <= 0.04045 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    };
    Some(0.2126 * lin(rgb.r) + 0.7152 * lin(rgb.g) + 0.0722 * lin(rgb.b))
}

/// Find the 256-color palette index nearest to `hex` by squared RGB
/// distance. Returns `None` for unparseable input.
pub fn nearest_256_color(hex: &str) -> Option<u8> {
    let rgb = hex_to_rgb(hex)?;
    let dr = |p: Rgb| i32::from(p.r) - i32::from(rgb.r);
    let dg = |p: Rgb| i32::from(p.g) - i32::from(rgb.g);
    let db = |p: Rgb| i32::from(p.b) - i32::from(rgb.b);
    Some(
        (0u8..=255)
            .min_by_key(|&i| {
                let p = palette_to_rgb(i);
                dr(p) * dr(p) + dg(p) * dg(p) + db(p) * db(p)
            })
            .unwrap_or(0), // unreachable: the 0..=255 range is non-empty
    )
}

/// Build a foreground ANSI escape from a hex color.
///
/// In [`ColorMode::Truecolor`] the escape is `\x1b[38;2;R;G;Bm`; in
/// [`ColorMode::Color256`] the nearest palette entry is used, producing
/// `\x1b[38;5;Nm`. Returns `None` for unparseable input.
pub fn color_to_ansi(color: &str, mode: ColorMode) -> Option<String> {
    let rgb = hex_to_rgb(color)?;
    Some(match mode {
        ColorMode::Truecolor => format!("\x1b[38;2;{};{};{}m", rgb.r, rgb.g, rgb.b),
        ColorMode::Color256 => format!("\x1b[38;5;{}m", nearest_256_color(color).unwrap_or(0)),
    })
}

/// Foreground escape for a theme color value.
///
/// Empty string -> reset (`\x1b[39m`). A bare decimal number is used
/// directly as a 256-color index. Otherwise the value is parsed as hex;
/// unparseable input falls back to reset.
pub fn fg_ansi(color: &str, mode: ColorMode) -> String {
    if color.is_empty() {
        return "\x1b[39m".to_string();
    }
    if let Ok(n) = color.parse::<u8>() {
        return format!("\x1b[38;5;{n}m");
    }
    color_to_ansi(color, mode).unwrap_or_else(|| "\x1b[39m".to_string())
}

/// Background escape for a theme color value.
///
/// Same rules as [`fg_ansi`], with reset `\x1b[49m` and 256/truecolor
/// prefixes `\x1b[48;...`.
pub fn bg_ansi(color: &str, mode: ColorMode) -> String {
    if color.is_empty() {
        return "\x1b[49m".to_string();
    }
    if let Ok(n) = color.parse::<u8>() {
        return format!("\x1b[48;5;{n}m");
    }
    match color_to_ansi(color, mode) {
        Some(ansi) => ansi.replacen("\x1b[38;", "\x1b[48;", 1),
        None => "\x1b[49m".to_string(),
    }
}

/// Foreground escape from a JSON value (hex string or 256-color number).
pub fn fg_ansi_from_value(value: &Value, mode: ColorMode) -> String {
    match value {
        Value::String(s) => fg_ansi(s, mode),
        Value::Number(n) => n
            .as_u64()
            .filter(|&n| n <= 255)
            .map(|n| format!("\x1b[38;5;{n}m"))
            .unwrap_or_else(|| "\x1b[39m".to_string()),
        _ => "\x1b[39m".to_string(),
    }
}

/// Background escape from a JSON value (hex string or 256-color number).
pub fn bg_ansi_from_value(value: &Value, mode: ColorMode) -> String {
    match value {
        Value::String(s) => bg_ansi(s, mode),
        Value::Number(n) => n
            .as_u64()
            .filter(|&n| n <= 255)
            .map(|n| format!("\x1b[48;5;{n}m"))
            .unwrap_or_else(|| "\x1b[49m".to_string()),
        _ => "\x1b[49m".to_string(),
    }
}

pub fn detect_color_mode() -> ColorMode {
    let wt_session = std::env::var_os("WT_SESSION").is_some();
    let colorterm = std::env::var("COLORTERM").ok();
    let term = std::env::var("TERM").ok();
    detect_color_mode_from(wt_session, colorterm.as_deref(), term.as_deref())
}

/// Pure decision table behind [`detect_color_mode`], factored out so the
/// env-dependent logic is testable without mutating process environment.
fn detect_color_mode_from(
    wt_session: bool,
    colorterm: Option<&str>,
    term: Option<&str>,
) -> ColorMode {
    if wt_session {
        return ColorMode::Truecolor;
    }
    if let Some(colorterm) = colorterm
        && (colorterm == "truecolor" || colorterm == "24bit")
    {
        return ColorMode::Truecolor;
    }
    // After COLORTERM / WT_SESSION: dumb, linux, and empty TERM are 256-color
    // (`omp://theme.md` detectColorMode table). Other terms default truecolor,
    // except the explicit `*-256color` suffix.
    match term {
        None | Some("") | Some("dumb") | Some("linux") => ColorMode::Color256,
        Some(t) if t.ends_with("-256color") => ColorMode::Color256,
        _ => ColorMode::Truecolor,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_to_rgb_three_digit() {
        assert_eq!(
            hex_to_rgb("#f0a"),
            Some(Rgb {
                r: 255,
                g: 0,
                b: 170
            })
        );
        assert_eq!(
            hex_to_rgb("f0a"),
            Some(Rgb {
                r: 255,
                g: 0,
                b: 170
            })
        );
        assert_eq!(
            hex_to_rgb("#abc"),
            Some(Rgb {
                r: 170,
                g: 187,
                b: 204
            })
        );
    }

    #[test]
    fn hex_to_rgb_six_digit() {
        assert_eq!(hex_to_rgb("#ff0000"), Some(Rgb { r: 255, g: 0, b: 0 }));
        assert_eq!(
            hex_to_rgb("00ff88"),
            Some(Rgb {
                r: 0,
                g: 255,
                b: 136
            })
        );
        assert_eq!(
            hex_to_rgb("#AbCdEf"),
            Some(Rgb {
                r: 0xab,
                g: 0xcd,
                b: 0xef
            })
        );
    }

    #[test]
    fn hex_to_rgb_invalid() {
        assert_eq!(hex_to_rgb(""), None);
        assert_eq!(hex_to_rgb("#"), None);
        assert_eq!(hex_to_rgb("#ff"), None);
        assert_eq!(hex_to_rgb("#fffffff"), None);
        assert_eq!(hex_to_rgb("#gggggg"), None);
        assert_eq!(hex_to_rgb("#ff00 zz"), None);
    }

    #[test]
    fn rgb_to_hex_round_trip() {
        let rgb = Rgb {
            r: 0xfe,
            g: 0xbc,
            b: 0x38,
        };
        assert_eq!(rgb_to_hex(rgb), "#febc38");
        assert_eq!(hex_to_rgb(&rgb_to_hex(rgb)), Some(rgb));
        assert_eq!(rgb_to_hex(Rgb { r: 0, g: 0, b: 0 }), "#000000");
        assert_eq!(
            rgb_to_hex(Rgb {
                r: 255,
                g: 255,
                b: 255
            }),
            "#ffffff"
        );
    }

    #[test]
    fn hsv_round_trip_pure_colors() {
        for (rgb, want_h) in [
            (
                Rgb {
                    r: 255,
                    g: 255,
                    b: 255,
                },
                None,
            ),
            (Rgb { r: 0, g: 0, b: 0 }, None),
            (Rgb { r: 255, g: 0, b: 0 }, Some(0.0)),
            (Rgb { r: 0, g: 255, b: 0 }, Some(120.0)),
            (Rgb { r: 0, g: 0, b: 255 }, Some(240.0)),
        ] {
            let (h, s, v) = rgb_to_hsv(rgb);
            if let Some(want) = want_h {
                assert!((h - want).abs() < 1e-9, "hue {h} != {want} for {rgb:?}");
            }
            assert!((0.0..=1.0).contains(&s) && (0.0..=1.0).contains(&v));
            let back = hsv_to_rgb(h, s, v);
            assert_eq!(back, rgb, "round trip failed for {rgb:?}");
        }
    }

    #[test]
    fn hsv_to_rgb_normalizes() {
        // h out of range wraps; s/v clamp
        assert_eq!(hsv_to_rgb(361.0, 1.0, 1.0), hsv_to_rgb(1.0, 1.0, 1.0));
        assert_eq!(hsv_to_rgb(0.0, 2.0, 5.0), hsv_to_rgb(0.0, 1.0, 1.0));
        assert_eq!(hsv_to_rgb(0.0, -1.0, -1.0), hsv_to_rgb(0.0, 0.0, 0.0));
    }

    #[test]
    fn nearest_256_color_known_values() {
        assert_eq!(nearest_256_color("#000000"), Some(0));
        assert_eq!(nearest_256_color("#ffffff"), Some(15));
        assert_eq!(nearest_256_color("#ff0000"), Some(9));
        assert_eq!(nearest_256_color("#00ff00"), Some(10));
        assert_eq!(nearest_256_color("#0000ff"), Some(12));
        assert_eq!(nearest_256_color("#00ffff"), Some(14));
        assert_eq!(nearest_256_color("#ffff00"), Some(11));
        assert_eq!(nearest_256_color("#800000"), Some(1));
        // cube corner 5,5,5 -> 231 is white; 255 entry is 244 gray
        assert_eq!(nearest_256_color("#080808"), Some(232));
        assert_eq!(nearest_256_color("#eeeeee"), Some(255));
        assert_eq!(nearest_256_color("bogus"), None);
    }

    #[test]
    fn color_to_ansi_formats() {
        assert_eq!(
            color_to_ansi("#febc38", ColorMode::Truecolor),
            Some("\x1b[38;2;254;188;56m".to_string())
        );
        let a256 = color_to_ansi("#febc38", ColorMode::Color256).unwrap();
        assert!(a256.starts_with("\x1b[38;5;"), "got {a256:?}");
        assert!(a256.ends_with('m'));
        assert!(a256[7..a256.len() - 1].parse::<u8>().is_ok());
        assert_eq!(color_to_ansi("nope", ColorMode::Truecolor), None);
    }

    #[test]
    fn fg_ansi_forms() {
        assert_eq!(fg_ansi("", ColorMode::Truecolor), "\x1b[39m");
        assert_eq!(fg_ansi("244", ColorMode::Truecolor), "\x1b[38;5;244m");
        assert_eq!(fg_ansi("0", ColorMode::Truecolor), "\x1b[38;5;0m");
        assert_eq!(
            fg_ansi("#ff0000", ColorMode::Truecolor),
            "\x1b[38;2;255;0;0m"
        );
        assert_eq!(fg_ansi("zzz", ColorMode::Truecolor), "\x1b[39m");
    }

    #[test]
    fn bg_ansi_forms() {
        assert_eq!(bg_ansi("", ColorMode::Truecolor), "\x1b[49m");
        assert_eq!(bg_ansi("244", ColorMode::Truecolor), "\x1b[48;5;244m");
        assert_eq!(
            bg_ansi("#ff0000", ColorMode::Truecolor),
            "\x1b[48;2;255;0;0m"
        );
        assert_eq!(bg_ansi("zzz", ColorMode::Truecolor), "\x1b[49m");
    }

    #[test]
    fn ansi_from_value() {
        let num = serde_json::json!(244);
        assert_eq!(
            fg_ansi_from_value(&num, ColorMode::Truecolor),
            "\x1b[38;5;244m"
        );
        assert_eq!(
            bg_ansi_from_value(&num, ColorMode::Truecolor),
            "\x1b[48;5;244m"
        );
        let hex = serde_json::json!("#ff0000");
        assert_eq!(
            fg_ansi_from_value(&hex, ColorMode::Truecolor),
            "\x1b[38;2;255;0;0m"
        );
        assert_eq!(
            bg_ansi_from_value(&hex, ColorMode::Truecolor),
            "\x1b[48;2;255;0;0m"
        );
        let empty = serde_json::json!("");
        assert_eq!(fg_ansi_from_value(&empty, ColorMode::Truecolor), "\x1b[39m");
        assert_eq!(bg_ansi_from_value(&empty, ColorMode::Truecolor), "\x1b[49m");
        let bad = serde_json::json!(999);
        assert_eq!(fg_ansi_from_value(&bad, ColorMode::Truecolor), "\x1b[39m");
        assert_eq!(bg_ansi_from_value(&bad, ColorMode::Truecolor), "\x1b[49m");
    }

    #[test]
    fn color_luma_known_values() {
        assert_eq!(color_luma("#000000"), Some(0.0));
        let white = color_luma("#ffffff").unwrap();
        assert!((white - 1.0).abs() < 1e-9, "white {white}");
        let mid = color_luma("#808080").unwrap();
        assert!((mid - 128.0 / 255.0).abs() < 1e-9, "mid {mid}");
        assert_eq!(color_luma("bogus"), None);
    }

    #[test]
    fn relative_luminance_known_values() {
        assert_eq!(relative_luminance("#000000"), Some(0.0));
        assert_eq!(relative_luminance("#ffffff"), Some(1.0));
        // WCAG reference: #7f7f7f linearizes to ~0.212
        let half = relative_luminance("#7f7f7f").unwrap();
        assert!((half - 0.212).abs() < 0.01, "half {half}");
        assert_eq!(relative_luminance("bogus"), None);
    }

    #[test]
    fn detect_color_mode_env_dependent() {
        // WT_SESSION always wins.
        assert_eq!(
            detect_color_mode_from(true, Some("dumb"), Some("xterm-256color")),
            ColorMode::Truecolor
        );
        assert_eq!(
            detect_color_mode_from(true, None, None),
            ColorMode::Truecolor
        );
        // COLORTERM truecolor / 24bit.
        assert_eq!(
            detect_color_mode_from(false, Some("truecolor"), Some("dumb")),
            ColorMode::Truecolor
        );
        assert_eq!(
            detect_color_mode_from(false, Some("24bit"), Some("dumb")),
            ColorMode::Truecolor
        );
        // TERM suffix -> 256 color.
        assert_eq!(
            detect_color_mode_from(false, None, Some("xterm-256color")),
            ColorMode::Color256
        );
        // No / empty / dumb / linux TERM -> 256color.
        assert_eq!(
            detect_color_mode_from(false, None, None),
            ColorMode::Color256
        );
        assert_eq!(
            detect_color_mode_from(false, None, Some("")),
            ColorMode::Color256
        );
        assert_eq!(
            detect_color_mode_from(false, Some("dumb"), Some("dumb")),
            ColorMode::Color256
        );
        assert_eq!(
            detect_color_mode_from(false, None, Some("linux")),
            ColorMode::Color256
        );
        assert_eq!(
            detect_color_mode_from(false, None, Some("xterm")),
            ColorMode::Truecolor
        );
        // The env wrapper still returns one of the two modes.
        match detect_color_mode() {
            ColorMode::Truecolor | ColorMode::Color256 => {}
        }
    }

    #[test]
    fn adjust_hsv_shifts() {
        // red +120 hue -> green family
        let green = adjust_hsv("#ff0000", 120.0, 1.0, 1.0).unwrap();
        let (h, s, v) = rgb_to_hsv(hex_to_rgb(&green).unwrap());
        assert!(((h - 120.0).abs() < 1e-6), "hue {h}");
        assert!((s - 1.0).abs() < 1e-9 && (v - 1.0).abs() < 1e-9);
        // desaturate to zero -> gray, value preserved
        let gray = adjust_hsv("#ff0000", 0.0, 0.0, 1.0).unwrap();
        let (_, s2, _) = rgb_to_hsv(hex_to_rgb(&gray).unwrap());
        assert!(s2 < 1e-9, "s {s2}");
        // dim: value halves (u8 quantization: 0.5*255 -> 128 -> 128/255)
        let dim = adjust_hsv("#ff0000", 0.0, 1.0, 0.5).unwrap();
        let (_, _, v3) = rgb_to_hsv(hex_to_rgb(&dim).unwrap());
        assert!((v3 - 0.5).abs() < 0.01, "v {v3}");
        // unparseable input
        assert_eq!(adjust_hsv("nope", 1.0, 1.0, 1.0), None);
    }

    #[test]
    fn ansi256_to_hex_palette() {
        assert_eq!(ansi256_to_hex(0), "#000000");
        assert_eq!(ansi256_to_hex(15), "#ffffff");
        assert_eq!(ansi256_to_hex(16), "#000000");
        assert_eq!(ansi256_to_hex(231), "#ffffff");
        assert_eq!(ansi256_to_hex(232), "#080808");
        assert_eq!(ansi256_to_hex(255), "#eeeeee");
        assert_eq!(ansi256_to_hex(196), "#ff0000");
    }

    #[test]
    fn palette_to_rgb_grays_and_cube() {
        assert_eq!(palette_to_rgb(16), Rgb { r: 0, g: 0, b: 0 });
        assert_eq!(
            palette_to_rgb(231),
            Rgb {
                r: 255,
                g: 255,
                b: 255
            }
        );
        assert_eq!(
            palette_to_rgb(60),
            Rgb {
                r: 95,
                g: 95,
                b: 135
            }
        );
        assert_eq!(
            palette_to_rgb(250),
            Rgb {
                r: 188,
                g: 188,
                b: 188
            }
        );
    }
}
