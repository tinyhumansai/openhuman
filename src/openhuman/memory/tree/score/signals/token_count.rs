//! Token-count signal — penalises very short or very long chunks.
//!
//! Rationale: "+1", "lol", "👍" are usually noise; multi-page walls of text
//! are often pasted logs or attachments that overwhelm summarisation.
//! The signal is strongest in a middle band that corresponds to substantive
//! prose/discussion.
//!
//! Output is a score in `[0.0, 1.0]` shaped as a plateau between
//! `TOKEN_MIN` and `TOKEN_MAX` with linear ramps on both sides.

pub const TOKEN_MIN: u32 = 10; // below this → score 0
pub const TOKEN_RAMP_LOW: u32 = 30; // 10..30 → linear 0→1
pub const TOKEN_RAMP_HIGH: u32 = 3_000; // 3000..8000 → linear 1→0.5
pub const TOKEN_MAX: u32 = 8_000; // above → score 0.5 (not zero — still has content)

/// Score for a chunk's token count. See module docs for shape.
pub fn score(token_count: u32) -> f32 {
    if token_count < TOKEN_MIN {
        return 0.0;
    }
    if token_count <= TOKEN_RAMP_LOW {
        // linear 0..1 over [MIN, RAMP_LOW]
        let span = (TOKEN_RAMP_LOW - TOKEN_MIN) as f32;
        return (token_count - TOKEN_MIN) as f32 / span;
    }
    if token_count <= TOKEN_RAMP_HIGH {
        return 1.0;
    }
    if token_count <= TOKEN_MAX {
        // linear 1.0..0.5 over [RAMP_HIGH, MAX]
        let span = (TOKEN_MAX - TOKEN_RAMP_HIGH) as f32;
        let t = (token_count - TOKEN_RAMP_HIGH) as f32 / span;
        return 1.0 - 0.5 * t;
    }
    0.5
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tiny_is_zero() {
        assert_eq!(score(0), 0.0);
        assert_eq!(score(5), 0.0);
        assert_eq!(score(9), 0.0);
    }

    #[test]
    fn ramp_up_linear() {
        // score(MIN) = 0, score(RAMP_LOW) = 1.0
        assert!((score(TOKEN_MIN) - 0.0).abs() < 1e-4);
        assert!((score(TOKEN_RAMP_LOW) - 1.0).abs() < 1e-4);
        // midpoint ~0.5
        let mid = TOKEN_MIN + (TOKEN_RAMP_LOW - TOKEN_MIN) / 2;
        assert!((score(mid) - 0.5).abs() < 0.05);
    }

    #[test]
    fn plateau_is_one() {
        assert_eq!(score(200), 1.0);
        assert_eq!(score(1000), 1.0);
        assert_eq!(score(TOKEN_RAMP_HIGH), 1.0);
    }

    #[test]
    fn ramp_down_to_half() {
        assert!((score(TOKEN_MAX) - 0.5).abs() < 1e-4);
        assert_eq!(score(TOKEN_MAX + 10_000), 0.5);
    }

    #[test]
    fn monotonic_in_bands() {
        // Strictly increasing on the up-ramp
        assert!(score(TOKEN_MIN + 1) < score(TOKEN_RAMP_LOW - 1));
        // Strictly decreasing on the down-ramp
        assert!(score(TOKEN_RAMP_HIGH + 1) > score(TOKEN_MAX - 1));
    }
}
