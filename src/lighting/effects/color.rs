// Copyright (C) 2026 Michael Wilson <mike@mdwn.dev>
//
// This program is free software: you can redistribute it and/or modify it under
// the terms of the GNU General Public License as published by the Free Software
// Foundation, version 3.
//
// This program is distributed in the hope that it will be useful, but WITHOUT
// ANY WARRANTY; without even the implied warranty of MERCHANTABILITY or FITNESS
// FOR A PARTICULAR PURPOSE. See the GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License along with
// this program. If not, see <https://www.gnu.org/licenses/>.
//

/// Color representation
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub w: Option<u8>, // White channel for RGBW fixtures
}

impl Color {
    pub fn new(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b, w: None }
    }

    pub fn from_hex(hex: &str) -> Result<Self, Box<dyn std::error::Error>> {
        let hex = hex.trim_start_matches('#');
        if hex.len() != 6 {
            return Err("Invalid hex color format".into());
        }

        let r = u8::from_str_radix(&hex[0..2], 16)?;
        let g = u8::from_str_radix(&hex[2..4], 16)?;
        let b = u8::from_str_radix(&hex[4..6], 16)?;

        Ok(Color { r, g, b, w: None })
    }

    pub fn from_name(name: &str) -> Result<Self, Box<dyn std::error::Error>> {
        match name.to_lowercase().as_str() {
            "red" => Ok(Color {
                r: 255,
                g: 0,
                b: 0,
                w: None,
            }),
            "green" => Ok(Color {
                r: 0,
                g: 255,
                b: 0,
                w: None,
            }),
            "blue" => Ok(Color {
                r: 0,
                g: 0,
                b: 255,
                w: None,
            }),
            "white" => Ok(Color {
                r: 255,
                g: 255,
                b: 255,
                w: None,
            }),
            "black" => Ok(Color {
                r: 0,
                g: 0,
                b: 0,
                w: None,
            }),
            "yellow" => Ok(Color {
                r: 255,
                g: 255,
                b: 0,
                w: None,
            }),
            "cyan" => Ok(Color {
                r: 0,
                g: 255,
                b: 255,
                w: None,
            }),
            "magenta" => Ok(Color {
                r: 255,
                g: 0,
                b: 255,
                w: None,
            }),
            "orange" => Ok(Color {
                r: 255,
                g: 165,
                b: 0,
                w: None,
            }),
            "purple" => Ok(Color {
                r: 128,
                g: 0,
                b: 128,
                w: None,
            }),
            _ => Err(format!("Unknown color name: {}", name).into()),
        }
    }

    pub fn from_hsv(h: f64, s: f64, v: f64) -> Self {
        let c = v * s;
        let x = c * (1.0 - ((h / 60.0) % 2.0 - 1.0).abs());
        let m = v - c;

        // HSV to RGB conversion: determine which sector of the color wheel
        // Each sector is 60 degrees, with different RGB component ordering
        let sector = (h / 60.0).floor() as u8 % 6;
        let (r, g, b) = match sector {
            0 => (c, x, 0.0),
            1 => (x, c, 0.0),
            2 => (0.0, c, x),
            3 => (0.0, x, c),
            4 => (x, 0.0, c),
            _ => (c, 0.0, x), // sector 5
        };

        Self {
            r: ((r + m) * 255.0) as u8,
            g: ((g + m) * 255.0) as u8,
            b: ((b + m) * 255.0) as u8,
            w: None,
        }
    }

    /// Hue in degrees, saturation and value of the colour's RGB, each of
    /// saturation and value from 0.0 to 1.0.
    pub fn to_hsv(&self) -> (f64, f64, f64) {
        let (r, g, b) = (
            self.r as f64 / 255.0,
            self.g as f64 / 255.0,
            self.b as f64 / 255.0,
        );
        let max = r.max(g).max(b);
        let delta = max - r.min(g).min(b);
        let hue = if delta == 0.0 {
            0.0
        } else if max == r {
            60.0 * ((g - b) / delta).rem_euclid(6.0)
        } else if max == g {
            60.0 * ((b - r) / delta + 2.0)
        } else {
            60.0 * ((r - g) / delta + 4.0)
        };
        let saturation = if max == 0.0 { 0.0 } else { delta / max };
        (hue, saturation, max)
    }

    /// Interpolate between two colours around the colour wheel: the hue
    /// turns the short way from `self` to `other` while saturation and
    /// brightness blend linearly, so the midpoint keeps its colour where a
    /// straight [`Color::lerp`] goes dull: yellow to blue is grey halfway
    /// in RGB (#541). Colours opposite each other turn through red. A grey
    /// has no hue of its own and takes the other colour's, so white to red
    /// only gains saturation. The white channel blends as in `lerp`.
    pub fn lerp_hue(&self, other: &Color, t: f64) -> Self {
        let t = t.clamp(0.0, 1.0);
        let (h1, s1, v1) = self.to_hsv();
        let (h2, s2, v2) = other.to_hsv();
        let (h1, h2) = match (s1 == 0.0, s2 == 0.0) {
            (true, true) => return self.lerp(other, t),
            (true, false) => (h2, h2),
            (false, true) => (h1, h1),
            (false, false) => (h1, h2),
        };
        let turn = (h2 - h1 + 540.0).rem_euclid(360.0) - 180.0;
        let hue = (h1 + turn * t).rem_euclid(360.0);
        let blended = Color::from_hsv(hue, s1 + (s2 - s1) * t, v1 + (v2 - v1) * t);
        Self {
            w: self.lerp(other, t).w,
            ..blended
        }
    }

    /// Linearly interpolate between two colors.
    /// `t` should be between 0.0 (returns `self`) and 1.0 (returns `other`).
    pub fn lerp(&self, other: &Color, t: f64) -> Self {
        let t = t.clamp(0.0, 1.0);
        let t_inv = 1.0 - t;

        // Helper to interpolate between two u8 values
        let lerp_u8 = |a: u8, b: u8| -> u8 { (a as f64 * t_inv + b as f64 * t) as u8 };

        Self {
            r: lerp_u8(self.r, other.r),
            g: lerp_u8(self.g, other.g),
            b: lerp_u8(self.b, other.b),
            w: match (self.w, other.w) {
                (Some(w1), Some(w2)) => Some(lerp_u8(w1, w2)),
                (Some(w1), None) => Some((w1 as f64 * t_inv) as u8),
                (None, Some(w2)) => Some((w2 as f64 * t) as u8),
                (None, None) => None,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── Color::new ──────────────────────────────────────────────────

    #[test]
    fn new_rgb() {
        let c = Color::new(10, 20, 30);
        assert_eq!(c.r, 10);
        assert_eq!(c.g, 20);
        assert_eq!(c.b, 30);
        assert_eq!(c.w, None);
    }

    // ── Color::from_hex ─────────────────────────────────────────────

    #[test]
    fn from_hex_valid() {
        let c = Color::from_hex("#FF8000").unwrap();
        assert_eq!(c.r, 255);
        assert_eq!(c.g, 128);
        assert_eq!(c.b, 0);
    }

    #[test]
    fn from_hex_without_hash() {
        let c = Color::from_hex("00FF00").unwrap();
        assert_eq!(c, Color::new(0, 255, 0));
    }

    #[test]
    fn from_hex_black() {
        let c = Color::from_hex("#000000").unwrap();
        assert_eq!(c, Color::new(0, 0, 0));
    }

    #[test]
    fn from_hex_white() {
        let c = Color::from_hex("#FFFFFF").unwrap();
        assert_eq!(c, Color::new(255, 255, 255));
    }

    #[test]
    fn from_hex_lowercase() {
        let c = Color::from_hex("#ff0000").unwrap();
        assert_eq!(c, Color::new(255, 0, 0));
    }

    #[test]
    fn from_hex_invalid_length() {
        assert!(Color::from_hex("#FFF").is_err());
        assert!(Color::from_hex("#FFFFFFF").is_err());
    }

    #[test]
    fn from_hex_invalid_chars() {
        assert!(Color::from_hex("#GGHHII").is_err());
    }

    // ── Color::from_name ────────────────────────────────────────────

    #[test]
    fn from_name_all_known_colors() {
        let cases = vec![
            ("red", (255, 0, 0)),
            ("green", (0, 255, 0)),
            ("blue", (0, 0, 255)),
            ("white", (255, 255, 255)),
            ("black", (0, 0, 0)),
            ("yellow", (255, 255, 0)),
            ("cyan", (0, 255, 255)),
            ("magenta", (255, 0, 255)),
            ("orange", (255, 165, 0)),
            ("purple", (128, 0, 128)),
        ];
        for (name, (r, g, b)) in cases {
            let c = Color::from_name(name).unwrap();
            assert_eq!(c.r, r, "failed for {}", name);
            assert_eq!(c.g, g, "failed for {}", name);
            assert_eq!(c.b, b, "failed for {}", name);
        }
    }

    #[test]
    fn from_name_case_insensitive() {
        assert_eq!(
            Color::from_name("RED").unwrap(),
            Color::from_name("red").unwrap()
        );
        assert_eq!(
            Color::from_name("Blue").unwrap(),
            Color::from_name("blue").unwrap()
        );
    }

    #[test]
    fn from_name_unknown() {
        assert!(Color::from_name("chartreuse").is_err());
    }

    // ── Color::from_hsv ─────────────────────────────────────────────

    #[test]
    fn from_hsv_red() {
        let c = Color::from_hsv(0.0, 1.0, 1.0);
        assert_eq!(c.r, 255);
        assert_eq!(c.g, 0);
        assert_eq!(c.b, 0);
    }

    #[test]
    fn from_hsv_green() {
        let c = Color::from_hsv(120.0, 1.0, 1.0);
        assert_eq!(c.r, 0);
        assert_eq!(c.g, 255);
        assert_eq!(c.b, 0);
    }

    #[test]
    fn from_hsv_blue() {
        let c = Color::from_hsv(240.0, 1.0, 1.0);
        assert_eq!(c.r, 0);
        assert_eq!(c.g, 0);
        assert_eq!(c.b, 255);
    }

    #[test]
    fn from_hsv_black() {
        let c = Color::from_hsv(0.0, 0.0, 0.0);
        assert_eq!(c.r, 0);
        assert_eq!(c.g, 0);
        assert_eq!(c.b, 0);
    }

    #[test]
    fn from_hsv_white() {
        let c = Color::from_hsv(0.0, 0.0, 1.0);
        assert_eq!(c.r, 255);
        assert_eq!(c.g, 255);
        assert_eq!(c.b, 255);
    }

    #[test]
    fn from_hsv_half_brightness() {
        let c = Color::from_hsv(0.0, 1.0, 0.5);
        assert_eq!(c.r, 127);
        assert_eq!(c.g, 0);
        assert_eq!(c.b, 0);
    }

    // ── Color::lerp ─────────────────────────────────────────────────

    #[test]
    fn lerp_at_zero() {
        let a = Color::new(0, 0, 0);
        let b = Color::new(255, 255, 255);
        let result = a.lerp(&b, 0.0);
        assert_eq!(result, a);
    }

    #[test]
    fn lerp_at_one() {
        let a = Color::new(0, 0, 0);
        let b = Color::new(255, 255, 255);
        let result = a.lerp(&b, 1.0);
        assert_eq!(result, b);
    }

    #[test]
    fn lerp_at_midpoint() {
        let a = Color::new(0, 0, 0);
        let b = Color::new(254, 254, 254);
        let result = a.lerp(&b, 0.5);
        assert_eq!(result.r, 127);
        assert_eq!(result.g, 127);
        assert_eq!(result.b, 127);
    }

    #[test]
    fn lerp_clamps_below_zero() {
        let a = Color::new(100, 100, 100);
        let b = Color::new(200, 200, 200);
        let result = a.lerp(&b, -1.0);
        assert_eq!(result, a);
    }

    #[test]
    fn lerp_clamps_above_one() {
        let a = Color::new(100, 100, 100);
        let b = Color::new(200, 200, 200);
        let result = a.lerp(&b, 2.0);
        assert_eq!(result, b);
    }

    #[test]
    fn lerp_with_white_channels() {
        let a = Color {
            r: 0,
            g: 0,
            b: 0,
            w: Some(0),
        };
        let b = Color {
            r: 255,
            g: 255,
            b: 255,
            w: Some(254),
        };
        let result = a.lerp(&b, 0.5);
        assert_eq!(result.w, Some(127));
    }

    #[test]
    fn lerp_no_white_channels() {
        let a = Color::new(0, 0, 0);
        let b = Color::new(255, 255, 255);
        let result = a.lerp(&b, 0.5);
        assert_eq!(result.w, None);
    }

    // ── Color::to_hsv / lerp_hue ───────────────────────────────────

    #[test]
    fn to_hsv_primaries_and_grey() {
        assert_eq!(Color::new(255, 0, 0).to_hsv(), (0.0, 1.0, 1.0));
        assert_eq!(Color::new(0, 255, 0).to_hsv(), (120.0, 1.0, 1.0));
        assert_eq!(Color::new(0, 0, 255).to_hsv(), (240.0, 1.0, 1.0));
        let (_, s, v) = Color::new(128, 128, 128).to_hsv();
        assert_eq!(s, 0.0);
        assert!((v - 128.0 / 255.0).abs() < 1e-12);
    }

    #[test]
    fn lerp_hue_keeps_the_midpoint_saturated() {
        let yellow = Color::new(255, 255, 0);
        let blue = Color::new(0, 0, 255);
        // In RGB the midpoint is grey.
        let straight = yellow.lerp(&blue, 0.5);
        assert_eq!((straight.r, straight.g, straight.b), (127, 127, 127));
        // Around the wheel it is a full-strength colour.
        let (_, s, v) = yellow.lerp_hue(&blue, 0.5).to_hsv();
        assert_eq!((s, v), (1.0, 1.0));
    }

    #[test]
    fn lerp_hue_turns_the_short_way() {
        // Red (0°) to magenta (300°): back through 330°, not round by green.
        let mid = Color::new(255, 0, 0).lerp_hue(&Color::new(255, 0, 255), 0.5);
        assert_eq!((mid.r, mid.g), (255, 0));
        assert!((126..=128).contains(&mid.b), "{mid:?}");
    }

    #[test]
    fn lerp_hue_reaches_its_endpoints() {
        let a = Color::new(255, 128, 0);
        let b = Color::new(0, 64, 200);
        assert_eq!(a.lerp_hue(&b, 0.0), a);
        let end = a.lerp_hue(&b, 1.0);
        assert!(
            end.r.abs_diff(b.r) <= 1 && end.g.abs_diff(b.g) <= 1 && end.b.abs_diff(b.b) <= 1,
            "{end:?}"
        );
    }

    #[test]
    fn lerp_hue_from_grey_takes_the_other_hue() {
        let mid = Color::new(255, 255, 255).lerp_hue(&Color::new(255, 0, 0), 0.5);
        assert_eq!(mid.r, 255);
        assert!(mid.g == mid.b && (126..=128).contains(&mid.g), "{mid:?}");
        let greys = Color::new(0, 0, 0).lerp_hue(&Color::new(200, 200, 200), 0.5);
        assert_eq!(
            greys,
            Color::new(0, 0, 0).lerp(&Color::new(200, 200, 200), 0.5)
        );
    }
}
