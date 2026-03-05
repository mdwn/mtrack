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
    #[cfg(test)]
    pub fn new(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b, w: None }
    }

    #[cfg(test)]
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
}
