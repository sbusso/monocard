//! The border of a clipped area, smoothed.

use mono_core::Plane;

/// Smooth the `radius` pixels (2 to 6) where clipped meets unclipped, half
/// of them on each side, so a blown light has no hard ring. The clipped
/// pixels further in and the unclipped ones further out are not touched:
/// nothing is recovered.
///
/// A pixel counts as clipped where `clip` says so and the plane still
/// holds light: a hot pixel that was repaired is clipped in the map and
/// dark in the plane, and has no ring to smooth. `white` is where the plane
/// clips.
pub fn inpaint_edges(plane: &mut Plane, clip: &Plane, radius: u32, white: f32) {
    let (w, h) = (plane.width, plane.height);
    if (clip.width, clip.height) != (w, h) || clip.data.len() != plane.data.len() {
        log::warn!("the clip map is not the picture's size; edges left as they are");
        return;
    }
    // The least light a clipped pixel holds after denoise and resampling.
    let held = 0.5 * white;
    let clipped: Vec<bool> = plane
        .data
        .iter()
        .zip(&clip.data)
        .map(|(v, c)| *c >= 0.5 && *v >= held)
        .collect();
    if !clipped.contains(&true) {
        return;
    }
    // How far from the seam, in pixels past the one that touches it.
    let reach = (radius.clamp(2, 6) - 2) as f32 / 2.0;
    let r = reach as isize;
    let mut away = vec![f32::INFINITY; w * h];
    let mut band: Vec<usize> = Vec::new();
    for y in 0..h {
        for x in 0..w {
            let i = y * w + x;
            let seam = (x + 1 < w && clipped[i + 1] != clipped[i])
                || (x > 0 && clipped[i - 1] != clipped[i])
                || (y + 1 < h && clipped[i + w] != clipped[i])
                || (y > 0 && clipped[i - w] != clipped[i]);
            if !seam {
                continue;
            }
            for dy in -r..=r {
                for dx in -r..=r {
                    let d = ((dx * dx + dy * dy) as f32).sqrt();
                    let (nx, ny) = (x as isize + dx, y as isize + dy);
                    let inside = nx >= 0 && ny >= 0 && nx < w as isize && ny < h as isize;
                    if d <= reach && inside {
                        let n = ny as usize * w + nx as usize;
                        if away[n].is_infinite() {
                            band.push(n);
                        }
                        away[n] = away[n].min(d);
                    }
                }
            }
        }
    }
    // Relaxed toward the mean of the four around it, the pixels outside
    // the band held: a smooth fill between the two sides.
    let first = plane.clone();
    for _ in 0..4 * radius.clamp(2, 6) {
        let next: Vec<f32> = band
            .iter()
            .map(|&i| {
                let (x, y) = ((i % w) as isize, (i / w) as isize);
                (plane.clamped(x - 1, y)
                    + plane.clamped(x + 1, y)
                    + plane.clamped(x, y - 1)
                    + plane.clamped(x, y + 1))
                    / 4.0
            })
            .collect();
        for (&i, v) in band.iter().zip(next) {
            plane.data[i] = v;
        }
    }
    for &i in &band {
        let k = 1.0 - away[i] / (reach + 1.0);
        plane.data[i] = first.data[i] + k * (plane.data[i] - first.data[i]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A disc of light over a dark wall, and its clip map.
    fn disc() -> (Plane, Plane) {
        let r = |x: usize, y: usize| ((x as f32 - 32.0).powi(2) + (y as f32 - 32.0).powi(2)).sqrt();
        (
            Plane::from_fn(64, 64, |x, y| {
                if r(x, y) < 12.0 {
                    1.0
                } else {
                    0.2 + 0.001 * x as f32
                }
            }),
            Plane::from_fn(64, 64, |x, y| f32::from(r(x, y) < 12.0)),
        )
    }

    #[test]
    fn only_the_border_changes() {
        for radius in 2..=6u32 {
            let (mut p, clip) = disc();
            let before = p.clone();
            inpaint_edges(&mut p, &clip, radius, 1.0);
            let (mut changed, mut steepest) = (0, 0.0f32);
            for y in 0..64usize {
                for x in 0..64usize {
                    let r = ((x as f32 - 32.0).powi(2) + (y as f32 - 32.0).powi(2)).sqrt();
                    let same = p.get(x, y) == before.get(x, y);
                    changed += usize::from(!same);
                    if (r - 12.0).abs() > radius as f32 / 2.0 + 1.5 {
                        assert!(same, "radius {radius}: {x},{y}");
                    }
                    if x > 0 {
                        steepest = steepest.max((p.get(x, y) - p.get(x - 1, y)).abs());
                    }
                }
            }
            assert!(changed > 0);
            assert!(steepest < 0.7, "radius {radius}: {steepest}");
        }
    }

    #[test]
    fn a_repaired_hot_pixel_and_a_wrong_size_change_nothing() {
        let mut p = Plane::filled(16, 16, 0.1);
        let mut clip = Plane::filled(16, 16, 0.0);
        clip.data[100] = 1.0;
        let before = p.clone();
        inpaint_edges(&mut p, &clip, 4, 1.0);
        assert_eq!(p, before);
        inpaint_edges(&mut p, &Plane::filled(3, 3, 1.0), 4, 1.0);
        assert_eq!(p, before);
        // Nor has one of a file with a baseline exposure, in stops of light.
        for gain in [0.25, 4.0] {
            let (mut p, mut clip) = disc();
            p.data.iter_mut().for_each(|v| *v *= gain);
            let mut plain = p.clone();
            inpaint_edges(&mut plain, &clip, 4, gain);
            // A repaired hot pixel at the brightness of the wall.
            clip.data[64 * 4 + 4] = 1.0;
            let wall = p.data[64 * 4 + 4];
            inpaint_edges(&mut p, &clip, 4, gain);
            assert_eq!(p.data[64 * 4 + 4], wall, "gain {gain}");
            assert_eq!(p, plain);
            assert_ne!(p.get(32, 20), gain, "the disc's border is smoothed");
        }
        // A picture clipped whole has no border.
        let mut white = Plane::filled(16, 16, 1.0);
        inpaint_edges(&mut white, &Plane::filled(16, 16, 1.0), 6, 1.0);
        assert_eq!(white, Plane::filled(16, 16, 1.0));
    }
}
