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
