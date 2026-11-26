//! The raw histogram: the sensor's samples by stops under its clip point.

use mono_core::RawHistogram;
use rayon::prelude::*;

/// `samples` are 0 at the black and 1 at the clip point, in `steps` steps,
/// and `clip` is 1.0 where one clipped.
pub fn raw_histogram(samples: &[f32], clip: &[f32], steps: usize) -> RawHistogram {
    const N: usize = RawHistogram::BINS;
    let top = steps.clamp(1, 65535);
    // The samples that did not clip, by step.
    let counts = samples
        .par_chunks(1 << 20)
        .zip(clip.par_chunks(1 << 20))
        .map(|(samples, clip)| {
            let mut counts = vec![0u32; top + 1];
            for (v, c) in samples.iter().zip(clip) {
                if *c < 0.5 {
                    counts[((v * top as f32).round() as usize).min(top)] += 1;
                }
            }
            counts
        })
        .reduce(
            || vec![0; top + 1],
            |mut a, b| {
                a.iter_mut().zip(b).for_each(|(a, b)| *a += b);
                a
            },
        );
    // How many are under each step.
    let mut under = vec![0u32];
    under.extend(counts.iter().scan(0, |sum, n| {
        *sum += n;
        Some(*sum)
    }));
    let kept = under[top + 1];
    let clipped = samples.len() as u32 - kept;
    let brightest = (kept as f32 * 0.999).ceil() as u32;
    let headroom = (kept > 0).then(|| {
        let step = under[1..].partition_point(|n| *n < brightest).max(1);
        (top as f32 / step as f32).log2()
    });

    // A step is spread over the light it stands for: in the shadows it is
    // wider than a bin.
    let below = |light: f32| {
        let x = light * top as f32 + 0.5;
        let k = (x as usize).min(top);
        f64::from(under[k]) + f64::from(counts[k]) * f64::from((x - k as f32).min(1.0))
    };
    let edge = |b: usize| ((b as f32 / N as f32 - 1.0) * RawHistogram::STOPS).exp2();
    let mut bins: Vec<u32> = (0..N)
        .map(|b| {
            let from = if b == 0 { 0.0 } else { below(edge(b)) };
            let to = if b == N - 1 {
                kept.into()
            } else {
                below(edge(b + 1))
            };
            (to - from).round().max(0.0) as u32
        })
        .collect();
    bins[N - 1] += clipped;
    RawHistogram {
        bins,
        clipped: clipped as f32 / samples.len().max(1) as f32,
        headroom,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stops_clipped_share_and_headroom() {
        let bin = |stops: f32| ((1.0 - stops / RawHistogram::STOPS) * 256.0) as usize;
        // Half the plane 1 stop under the clip point, 45 % 6 stops under,
        // and 5 % clipped, some of it repaired to less.
        let samples: Vec<f32> = (0..1000)
            .map(|i| match i {
                0..500 => 0.5,
                500..950 => 1.0 / 64.0,
                950..975 => 0.3,
                _ => 1.0,
            })
            .collect();
        let clip: Vec<f32> = (0..1000).map(|i| f32::from(u8::from(i >= 950))).collect();
        let h = raw_histogram(&samples, &clip, 16383);
        let sum = |at: usize, r: usize| h.bins[at - r..=at + r].iter().sum::<u32>();
        assert_eq!((sum(bin(1.0), 1), sum(bin(6.0), 1)), (500, 450));
        assert_eq!((h.bins[255], h.bins.iter().sum::<u32>()), (50, 1000));
        assert_eq!(h.clipped, 0.05);
        assert!((h.headroom.unwrap() - 1.0).abs() < 1e-3);

        // Under the last stop and under the black: the first bin. No sample
        // is lost, and one bright pixel in ten thousand is no headroom.
        let mut dark = vec![-0.01f32; 10_000];
        dark[..5000].fill(1.0 / 16383.0);
        dark[5000..9999].fill(0.25);
        dark[9999] = 0.99;
        let h = raw_histogram(&dark, &vec![0.0; 10_000], 16383);
        assert_eq!((h.bins[0], h.clipped), (5000, 0.0));
        assert!((h.headroom.unwrap() - 2.0).abs() < 1e-3);

        let h = raw_histogram(&[1.0; 4], &[1.0; 4], 16383);
        assert_eq!((h.clipped, h.headroom, h.bins[255]), (1.0, None, 4));
        assert_eq!(raw_histogram(&[], &[], 0).headroom, None);
    }

    /// A step of the sensor's is wider than a bin in the shadows: a ramp
    /// of them leaves no bin empty.
    #[test]
    fn the_shadows_have_no_gaps() {
        let ramp: Vec<f32> = (0..1 << 16).map(|i| (i / 4) as f32 / 16383.0).collect();
        let h = raw_histogram(&ramp, &vec![0.0; 1 << 16], 16383);
        let dark = &h.bins[43..128];
        assert!(dark.iter().all(|n| *n > 0), "{dark:?}");
    }
}
