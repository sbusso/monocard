//! A color photo as one film sees it: the grey a black and white film
//! makes of each color. The tables are mono-tone's (`film::color`).

use std::sync::Arc;

/// A film's response to color on a chromaticity grid, and the filter on
/// the lens in front of it.
#[derive(Debug, Clone, PartialEq)]
pub struct ColorResponse {
    /// Linear Rec.2020 to CIE XYZ with the white of the film's data.
    pub to_xyz: [[f32; 3]; 3],
    /// What the lens filter lets through of red, green and blue, scaled
    /// so that a grey stays the same grey.
    through: [f32; 3],
    /// `size` by `size`: over `(1 - x)^2` then `y / (1 - x)`, 0 to 1.
    size: usize,
    table: Arc<[f32]>,
}

impl ColorResponse {
    /// `table` holds `size * size` values. Panics when it does not: the
    /// tables are built in.
    pub fn new(to_xyz: [[f32; 3]; 3], through: [f32; 3], size: usize, table: Arc<[f32]>) -> Self {
        assert!(
            size >= 2 && table.len() == size * size,
            "a color table is square"
        );
        let mut r = Self {
            to_xyz,
            through: [1.0; 3],
            size,
            table,
        };
        let raw = r.grey([through[0], through[1], through[2]]);
        let k = if raw > 0.0 {
            r.grey([1.0; 3]) / raw
        } else {
            1.0
        };
        r.through = through.map(|t| t * k);
        r
    }

    /// The grey of linear Rec.2020 `rgb`: its luminance where it is
    /// neutral.
    pub fn grey(&self, rgb: [f32; 3]) -> f32 {
        let c: [f32; 3] = std::array::from_fn(|i| rgb[i].max(0.0) * self.through[i]);
        let xyz: [f32; 3] = std::array::from_fn(|r| (0..3).map(|k| self.to_xyz[r][k] * c[k]).sum());
        let b = xyz[0] + xyz[1] + xyz[2];
        if b <= 1e-12 {
            return 0.0;
        }
        let (x, y) = (xyz[0] / b, xyz[1] / b);
        let u = ((1.0 - x) * (1.0 - x)).clamp(0.0, 1.0);
        let v = (y / (1.0 - x).max(1e-10)).clamp(0.0, 1.0);
        let n = self.size - 1;
        let (pu, pv) = (u * n as f32, v * n as f32);
        let (i, j) = ((pu as usize).min(n - 1), (pv as usize).min(n - 1));
        let (tu, tv) = (pu - i as f32, pv - j as f32);
        let at = |a: usize, b: usize| self.table[a * self.size + b];
        let low = at(i, j) + (at(i, j + 1) - at(i, j)) * tv;
        let high = at(i + 1, j) + (at(i + 1, j + 1) - at(i + 1, j)) * tv;
        (b * (low + (high - low) * tu)).max(0.0)
    }
}

/// A LUT's mix of a color photo to grey, where no filter's mix fits it
/// (docs/notes/742): a cube of greys over display-referred sRGB, read
/// between the scene and a display transform that `display` gives. On
/// the grey axis it is the identity, so a grey stays the same grey and
/// the LUT's tone is the curve's to hold.
#[derive(Debug, Clone, PartialEq)]
pub struct GreyLut {
    /// Nodes an axis.
    size: usize,
    /// `size^3` greys, sRGB encoded display light, red fastest.
    greys: Arc<[f32]>,
    /// The display transform at [`Self::STEPS`] steps of 1/2.2 encoded
    /// scene light from 0 to 1, and its inverse on display light.
    to_display: Arc<[f32]>,
    from_display: Arc<[f32]>,
}

/// Linear Rec.2020 to linear sRGB, rows.
pub const REC2020_TO_SRGB: [[f32; 3]; 3] = [
    [1.660_491, -0.587_641_1, -0.072_849_9],
    [-0.124_550_5, 1.132_899_9, -0.008_349_4],
    [-0.018_150_8, -0.100_578_9, 1.118_729_7],
];

/// Linear sRGB to linear Rec.2020, rows.
pub const SRGB_TO_REC2020: [[f32; 3]; 3] = [
    [0.627_404, 0.329_283, 0.043_313],
    [0.069_097, 0.919_54, 0.011_362],
    [0.016_391, 0.088_013, 0.895_595],
];

/// `m` times `v`.
pub fn mul(m: &[[f32; 3]; 3], v: [f32; 3]) -> [f32; 3] {
    std::array::from_fn(|r| (0..3).map(|k| m[r][k] * v[k]).sum())
}

impl GreyLut {
    /// Steps of the two tables of the display transform.
    pub const STEPS: usize = 1024;
    /// The most nodes an axis a cube may have.
    pub const MAX_SIZE: usize = 33;

    /// From `greys` (`size^3`, red fastest) and the display transform:
    /// scene light to display light, rising, 1 at 1.
    pub fn new(size: usize, greys: Vec<f32>, display: impl Fn(f32) -> f32) -> Option<Self> {
        if !(2..=Self::MAX_SIZE).contains(&size) || greys.len() != size.pow(3) {
            return None;
        }
        let n = Self::STEPS - 1;
        let to: Vec<f32> = (0..=n)
            .map(|i| display((i as f32 / n as f32).powf(2.2)).clamp(0.0, 1.0))
            .collect();
        // The inverse by halves on the table, on 1/2.2 encoded light.
        let from: Vec<f32> = (0..=n)
            .map(|i| {
                let y = (i as f32 / n as f32).powf(2.2);
                let (mut lo, mut hi) = (0.0f32, 1.0f32);
                for _ in 0..30 {
                    let mid = 0.5 * (lo + hi);
                    if Self::read(&to, mid) < y {
                        lo = mid;
                    } else {
                        hi = mid;
                    }
                }
                hi.powf(2.2)
            })
            .collect();
        Some(Self {
            size,
            greys: greys.into(),
            to_display: to.into(),
            from_display: from.into(),
        })
    }

    /// The greys as their text: four hex digits each, sRGB encoded display
    /// light times 65535, red fastest.
    pub fn to_hex(greys: &[f32]) -> String {
        greys
            .iter()
            .map(|g| format!("{:04x}", (g.clamp(0.0, 1.0) * 65535.0).round() as u16))
            .collect()
    }

    /// [`Self::to_hex`] read back; none for a text that is not a cube of
    /// 2 to [`Self::MAX_SIZE`] nodes an axis.
    pub fn from_hex(text: &str, display: impl Fn(f32) -> f32) -> Option<Self> {
        if !text.is_ascii() || !text.len().is_multiple_of(4) {
            return None;
        }
        let greys: Vec<f32> = text
            .as_bytes()
            .chunks(4)
            .map(|c| {
                let digits = std::str::from_utf8(c).ok()?;
                u16::from_str_radix(digits, 16).ok()
            })
            .map(|v| v.map(|v| f32::from(v) / 65535.0))
            .collect::<Option<_>>()?;
        let size = (2..=Self::MAX_SIZE).find(|n| n.pow(3) == greys.len())?;
        Self::new(size, greys, display)
    }

    /// `table` at 1/2.2 encoded light `g`, 0 to 1.
    fn read(table: &[f32], g: f32) -> f32 {
        let n = table.len() - 1;
        let p = g.clamp(0.0, 1.0) * n as f32;
        let i = (p as usize).min(n - 1);
        table[i] + (table[i + 1] - table[i]) * (p - i as f32)
    }

    /// The cube at sRGB encoded `rgb`, tetrahedral, as a `.cube` is read
    /// (`greycard_core::lut`): exact on the grey axis. An sRGB encoded grey.
    pub fn at(&self, rgb: [f32; 3]) -> f32 {
        let n = self.size - 1;
        let p = rgb.map(|v| v.clamp(0.0, 1.0) * n as f32);
        let i = p.map(|v| (v as usize).min(n - 1));
        let [fr, fg, fb]: [f32; 3] = std::array::from_fn(|k| p[k] - i[k] as f32);
        let at = |r: usize, g: usize, b: usize| {
            self.greys[((i[2] + b) * self.size + i[1] + g) * self.size + i[0] + r]
        };
        // The two corners passed on the way from black to white of the
        // cell, and the fractions in falling order.
        let (a, b, w) = if fr > fg {
            if fg > fb {
                (at(1, 0, 0), at(1, 1, 0), [fr, fg, fb])
            } else if fr > fb {
                (at(1, 0, 0), at(1, 0, 1), [fr, fb, fg])
            } else {
                (at(0, 0, 1), at(1, 0, 1), [fb, fr, fg])
            }
        } else if fb > fg {
            (at(0, 0, 1), at(0, 1, 1), [fb, fg, fr])
        } else if fb > fr {
            (at(0, 1, 0), at(0, 1, 1), [fg, fb, fr])
        } else {
            (at(0, 1, 0), at(1, 1, 0), [fg, fr, fb])
        };
        let (c0, c1) = (at(0, 0, 0), at(1, 1, 1));
        c0 + w[0] * (a - c0) + w[1] * (b - a) + w[2] * (c1 - b)
    }

    /// The grey of linear Rec.2020 scene light `rgb`: through the display
    /// transform by channel, the cube, and back through its inverse.
    // Ceiling, draft: light over 1 is scaled under it and back, so a LUT
    // made for display light leaves the highlights' ratios as they are.
    pub fn grey(&self, rgb: [f32; 3]) -> f32 {
        let srgb = mul(&REC2020_TO_SRGB, rgb).map(|v| v.max(0.0));
        let top = srgb.iter().copied().fold(1.0f32, f32::max);
        let encoded = srgb.map(|v| {
            let y = Self::read(&self.to_display, (v / top).powf(1.0 / 2.2));
            srgb_encode(y)
        });
        let y = srgb_decode(self.at(encoded));
        top * Self::read(&self.from_display, y.max(0.0).powf(1.0 / 2.2))
    }
}

/// Linear light to sRGB's curve.
pub fn srgb_encode(v: f32) -> f32 {
    let v = v.clamp(0.0, 1.0);
    if v <= 0.003_130_8 {
        12.92 * v
    } else {
        1.055 * v.powf(1.0 / 2.4) - 0.055
    }
}

/// sRGB's curve to linear light.
pub fn srgb_decode(v: f32) -> f32 {
    let v = v.clamp(0.0, 1.0);
    if v <= 0.040_45 {
        v / 12.92
    } else {
        ((v + 0.055) / 1.055).powf(2.4)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A cube that is the identity on the grey axis keeps a grey of any
    /// light, and its text reads back.
    #[test]
    fn a_grey_lut_keeps_a_grey_and_reads_back() {
        let n = 9;
        let mut greys = Vec::new();
        for _b in 0..n {
            for _g in 0..n {
                for r in 0..n {
                    // Red's light: on the grey axis the grey itself.
                    greys.push(r as f32 / (n - 1) as f32);
                }
            }
        }
        let display = |x: f32| x.clamp(0.0, 1.0).powf(0.8);
        let lut = GreyLut::new(n, greys.clone(), display).unwrap();
        for x in [0.01, 0.18, 0.5, 0.9, 2.0] {
            let y = lut.grey([x; 3]);
            assert!((y - x).abs() < 0.02 * x + 1e-3, "{x}: {y}");
        }
        // Rec.2020 green holds a little sRGB red, no more.
        assert!(lut.grey([0.0, 0.5, 0.0]) < 0.05);
        let back = GreyLut::from_hex(&GreyLut::to_hex(&greys), display).unwrap();
        assert!((back.grey([0.3, 0.2, 0.1]) - lut.grey([0.3, 0.2, 0.1])).abs() < 1e-3);
        assert!(GreyLut::from_hex("00ff", display).is_none());
        assert!(GreyLut::from_hex("zzzzzzzz", display).is_none());
    }
}
