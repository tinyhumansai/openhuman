use rand::Rng;
use std::f64::consts::TAU;

#[derive(Debug, Clone, Copy)]
pub struct HumanPathOptions {
    /// Total number of interpolation steps. Default 25.
    pub steps: usize,
    /// Mean dwell time between steps in milliseconds. Default 12 ms.
    pub mean_step_ms: f64,
    /// Std-dev of dwell time. Default 4 ms.
    pub stddev_step_ms: f64,
    /// Bezier control-point lateral deviation factor. Default 0.3.
    pub curvature: f64,
}

impl Default for HumanPathOptions {
    fn default() -> Self {
        Self {
            steps: 25,
            mean_step_ms: 12.0,
            stddev_step_ms: 4.0,
            curvature: 0.3,
        }
    }
}

/// Returns `(x, y, dwell_ms)` steps for a humanized cursor path.
pub fn human_path<R: Rng>(
    start: (i32, i32),
    end: (i32, i32),
    opts: &HumanPathOptions,
    rng: &mut R,
) -> Vec<(i32, i32, u64)> {
    if start == end || opts.steps == 0 {
        return vec![(end.0, end.1, 0)];
    }

    let start_f = (start.0 as f64, start.1 as f64);
    let end_f = (end.0 as f64, end.1 as f64);
    let dx = end_f.0 - start_f.0;
    let dy = end_f.1 - start_f.1;
    let dist = dx.hypot(dy);
    let steps = if dist < 5.0 {
        opts.steps.min(3)
    } else {
        opts.steps
    };
    if steps == 0 {
        return vec![(end.0, end.1, 0)];
    }

    let perp = (-dy / dist, dx / dist);
    let curvature = opts.curvature.max(0.0);
    let deviation = curvature * dist;
    let p1_offset = sample_normal(0.0, deviation, rng);
    let p2_offset = sample_normal(0.0, deviation, rng);
    let p1 = offset_perp(lerp(start_f, end_f, 0.33), perp, p1_offset);
    let p2 = offset_perp(lerp(start_f, end_f, 0.66), perp, p2_offset);

    (0..=steps)
        .map(|step| {
            let t = step as f64 / steps as f64;
            let (x, y) = cubic_bezier(start_f, p1, p2, end_f, t);
            (x.round() as i32, y.round() as i32, dwell_ms(opts, rng))
        })
        .collect()
}

fn lerp(start: (f64, f64), end: (f64, f64), t: f64) -> (f64, f64) {
    (
        start.0 + (end.0 - start.0) * t,
        start.1 + (end.1 - start.1) * t,
    )
}

fn offset_perp(point: (f64, f64), perp: (f64, f64), offset: f64) -> (f64, f64) {
    (point.0 + perp.0 * offset, point.1 + perp.1 * offset)
}

fn cubic_bezier(
    p0: (f64, f64),
    p1: (f64, f64),
    p2: (f64, f64),
    p3: (f64, f64),
    t: f64,
) -> (f64, f64) {
    let one_minus = 1.0 - t;
    let a = one_minus.powi(3);
    let b = 3.0 * one_minus.powi(2) * t;
    let c = 3.0 * one_minus * t.powi(2);
    let d = t.powi(3);
    (
        a * p0.0 + b * p1.0 + c * p2.0 + d * p3.0,
        a * p0.1 + b * p1.1 + c * p2.1 + d * p3.1,
    )
}

fn dwell_ms<R: Rng>(opts: &HumanPathOptions, rng: &mut R) -> u64 {
    let mean = finite_or_default(opts.mean_step_ms, HumanPathOptions::default().mean_step_ms);
    let stddev = finite_or_default(
        opts.stddev_step_ms,
        HumanPathOptions::default().stddev_step_ms,
    )
    .max(0.0);
    let sample = if stddev == 0.0 {
        mean
    } else {
        let raw = sample_normal(mean, stddev, rng);
        raw.clamp(mean - 3.0 * stddev, mean + 3.0 * stddev)
    };
    sample.max(1.0).round() as u64
}

fn finite_or_default(value: f64, default: f64) -> f64 {
    if value.is_finite() {
        value
    } else {
        default
    }
}

fn sample_normal<R: Rng>(mean: f64, stddev: f64, rng: &mut R) -> f64 {
    if stddev <= 0.0 {
        return mean;
    }
    let u1 = rng
        .random::<f64>()
        .clamp(f64::MIN_POSITIVE, 1.0 - f64::EPSILON);
    let u2 = rng.random::<f64>();
    let z0 = (-2.0 * u1.ln()).sqrt() * (TAU * u2).cos();
    mean + z0 * stddev
}

#[cfg(test)]
#[path = "human_path_tests.rs"]
mod tests;
