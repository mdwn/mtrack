// Copyright (C) 2025 Michael Wilson <mike@mdwn.dev>
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

    #[cfg(test)]
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
