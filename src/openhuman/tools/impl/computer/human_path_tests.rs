use super::*;
use rand::{rngs::StdRng, SeedableRng};

fn seeded() -> StdRng {
    StdRng::seed_from_u64(42)
}

#[test]
fn start_equals_end_returns_single_point() {
    let mut rng = seeded();
    let path = human_path((10, 20), (10, 20), &HumanPathOptions::default(), &mut rng);
    assert_eq!(path, vec![(10, 20, 0)]);
}

#[test]
fn steps_zero_returns_single_point() {
    let mut rng = seeded();
    let opts = HumanPathOptions {
        steps: 0,
        ..HumanPathOptions::default()
    };
    let path = human_path((10, 20), (30, 40), &opts, &mut rng);
    assert_eq!(path, vec![(30, 40, 0)]);
}

#[test]
fn path_starts_at_start_and_ends_at_end() {
    let mut rng = seeded();
    let path = human_path((10, 20), (210, 120), &HumanPathOptions::default(), &mut rng);
    assert_eq!((path.first().unwrap().0, path.first().unwrap().1), (10, 20));
    assert_eq!((path.last().unwrap().0, path.last().unwrap().1), (210, 120));
}

#[test]
fn path_has_expected_step_count() {
    let mut rng = seeded();
    let opts = HumanPathOptions {
        steps: 8,
        ..HumanPathOptions::default()
    };
    let path = human_path((0, 0), (100, 0), &opts, &mut rng);
    assert_eq!(path.len(), 9);
}

#[test]
fn tiny_move_caps_step_count() {
    let mut rng = seeded();
    let opts = HumanPathOptions {
        steps: 25,
        ..HumanPathOptions::default()
    };
    let path = human_path((0, 0), (4, 0), &opts, &mut rng);
    assert_eq!(path.len(), 4);
}

#[test]
fn dwell_times_within_3_sigma() {
    let mut rng = seeded();
    let opts = HumanPathOptions {
        steps: 40,
        mean_step_ms: 12.0,
        stddev_step_ms: 4.0,
        ..HumanPathOptions::default()
    };
    let path = human_path((0, 0), (100, 0), &opts, &mut rng);
    assert!(path.iter().all(|(_, _, dwell)| (0..=24).contains(dwell)));
}

#[test]
fn path_curves_off_straight_line() {
    let mut rng = seeded();
    let opts = HumanPathOptions {
        steps: 25,
        curvature: 0.8,
        ..HumanPathOptions::default()
    };
    let path = human_path((0, 0), (100, 0), &opts, &mut rng);
    assert!(path
        .iter()
        .skip(1)
        .take(path.len() - 2)
        .any(|(_, y, _)| y.abs() > 1));
}

#[test]
fn deterministic_with_seeded_rng() {
    let opts = HumanPathOptions::default();
    let mut first_rng = StdRng::seed_from_u64(7);
    let mut second_rng = StdRng::seed_from_u64(7);
    let first = human_path((5, 9), (150, 90), &opts, &mut first_rng);
    let second = human_path((5, 9), (150, 90), &opts, &mut second_rng);
    assert_eq!(first, second);
}
