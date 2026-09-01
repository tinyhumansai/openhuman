//! MMR selection tests.
//!
//! The regression here pins the sign-preserving contract documented on
//! [`cosine_similarity`]: an anti-correlated candidate is *more* diverse than
//! an orthogonal one, and the redundancy fold must not clamp that away.

use super::*;

/// Unit-ish vectors chosen so cosine similarity is exactly their dot product.
fn candidate<'a>(embedding: &'a [f32], relevance: f64, index: usize) -> MmrCandidate<'a> {
    MmrCandidate {
        index,
        embedding,
        relevance,
    }
}

#[test]
fn mmr_prefers_anti_correlated_over_orthogonal_at_equal_relevance() {
    // First pick is the highest-relevance candidate: `a` at [1, 0].
    // The two remaining candidates tie on relevance; `anti` at [-1, 0] has
    // similarity -1.0 to the selection, `ortho` at [0, 1] has 0.0. With the
    // fold seeded at 0.0 both reported 0.0 and the tie broke by index order,
    // selecting `ortho`; seeded at NEG_INFINITY the negative survives and the
    // MMR penalty term rewards `anti` as the more diverse pick.
    let a: [f32; 2] = [1.0, 0.0];
    let ortho: [f32; 2] = [0.0, 1.0];
    let anti: [f32; 2] = [-1.0, 0.0];
    let candidates = vec![
        candidate(&a, 1.0, 0),
        candidate(&ortho, 0.5, 1),
        candidate(&anti, 0.5, 2),
    ];

    let picked = mmr_select(&candidates, 2, 0.5);

    assert_eq!(picked.len(), 2);
    assert_eq!(picked[0].index, 0, "highest relevance is selected first");
    assert_eq!(
        picked[1].index, 2,
        "with only negative similarity to the selection, the anti-correlated \
         candidate must out-score the orthogonal one instead of tying at a \
         clamped 0.0"
    );
}

#[test]
fn mmr_first_pick_is_pure_relevance() {
    // With nothing selected yet there is no redundancy term; the seed change
    // must not leak into the first iteration.
    let a: [f32; 2] = [1.0, 0.0];
    let b: [f32; 2] = [0.0, 1.0];
    let candidates = vec![candidate(&a, 0.2, 0), candidate(&b, 0.9, 1)];

    let picked = mmr_select(&candidates, 1, 0.7);

    assert_eq!(picked.len(), 1);
    assert_eq!(picked[0].index, 1);
}
