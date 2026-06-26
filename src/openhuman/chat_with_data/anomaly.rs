//! Statistical anomaly detection for chat-with-data.
//!
//! Provides z-score and IQR-based outlier detection on numeric time series.
//! No external dependencies — pure Rust math.
//!
//! ## Log prefix
//!
//! `[chat-with-data-anomaly]`

use serde::{Deserialize, Serialize};
use tracing::{debug, info};

/// A detected anomaly in a time series.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Anomaly {
    pub index: usize,
    pub value: f64,
    pub score: f64,
    pub method: AnomalyMethod,
    pub direction: AnomalyDirection,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum AnomalyMethod {
    ZScore,
    Iqr,
    Combined,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum AnomalyDirection {
    High,
    Low,
}

/// Result of anomaly detection on a series.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnomalyReport {
    pub anomalies: Vec<Anomaly>,
    pub mean: f64,
    pub std_dev: f64,
    pub q1: f64,
    pub q3: f64,
    pub iqr: f64,
    pub series_length: usize,
}

/// Detect anomalies using z-score method.
///
/// Points with |z-score| > threshold are flagged as anomalies.
/// Default threshold is 2.5 (covers ~99% of normal distribution).
pub fn detect_zscore(data: &[f64], threshold: f64) -> Vec<Anomaly> {
    if !threshold.is_finite() || threshold <= 0.0 {
        return vec![];
    }
    if data.len() < 3 {
        return vec![];
    }

    let mean = mean(data);
    let std = std_dev(data, mean);

    if std == 0.0 {
        return vec![];
    }

    let mut anomalies = Vec::new();
    for (idx, &val) in data.iter().enumerate() {
        let z = (val - mean) / std;
        if z.abs() > threshold {
            anomalies.push(Anomaly {
                index: idx,
                value: val,
                score: z.abs(),
                method: AnomalyMethod::ZScore,
                direction: if z > 0.0 {
                    AnomalyDirection::High
                } else {
                    AnomalyDirection::Low
                },
            });
        }
    }

    debug!(
        count = anomalies.len(),
        threshold = threshold,
        "[chat-with-data-anomaly] z-score detection complete"
    );
    anomalies
}

/// Detect anomalies using IQR (Interquartile Range) method.
///
/// Points outside [Q1 - k*IQR, Q3 + k*IQR] are flagged.
/// Default k is 1.5 (standard Tukey fence).
pub fn detect_iqr(data: &[f64], k: f64) -> Vec<Anomaly> {
    if !k.is_finite() || k <= 0.0 {
        return vec![];
    }
    if data.len() < 4 {
        return vec![];
    }

    let mut sorted = data.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

    let q1 = percentile(&sorted, 25.0);
    let q3 = percentile(&sorted, 75.0);
    let iqr = q3 - q1;

    if iqr == 0.0 {
        return vec![];
    }

    let lower_fence = q1 - k * iqr;
    let upper_fence = q3 + k * iqr;

    let mut anomalies = Vec::new();
    for (idx, &val) in data.iter().enumerate() {
        if val < lower_fence {
            let score = (q1 - val) / iqr;
            anomalies.push(Anomaly {
                index: idx,
                value: val,
                score,
                method: AnomalyMethod::Iqr,
                direction: AnomalyDirection::Low,
            });
        } else if val > upper_fence {
            let score = (val - q3) / iqr;
            anomalies.push(Anomaly {
                index: idx,
                value: val,
                score,
                method: AnomalyMethod::Iqr,
                direction: AnomalyDirection::High,
            });
        }
    }

    debug!(
        count = anomalies.len(),
        iqr = iqr,
        "[chat-with-data-anomaly] IQR detection complete"
    );
    anomalies
}

/// Run both z-score and IQR detection, merge results.
///
/// Points flagged by BOTH methods get higher confidence.
pub fn detect_combined(data: &[f64], z_threshold: f64, iqr_k: f64) -> AnomalyReport {
    let z_anomalies = detect_zscore(data, z_threshold);
    let iqr_anomalies = detect_iqr(data, iqr_k);

    // Merge: if an index appears in both, mark as Combined with boosted score.
    let mut combined: Vec<Anomaly> = Vec::new();
    let iqr_indices: std::collections::HashSet<usize> =
        iqr_anomalies.iter().map(|a| a.index).collect();

    for mut z_anom in z_anomalies {
        if iqr_indices.contains(&z_anom.index) {
            z_anom.method = AnomalyMethod::Combined;
            z_anom.score *= 1.5; // Boost confidence for dual-detection.
        }
        combined.push(z_anom);
    }

    // Add IQR-only anomalies not already in z-score results.
    let z_indices: std::collections::HashSet<usize> = combined.iter().map(|a| a.index).collect();
    for iqr_anom in iqr_anomalies {
        if !z_indices.contains(&iqr_anom.index) {
            combined.push(iqr_anom);
        }
    }

    // Sort by score descending.
    combined.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let m = mean(data);
    let s = std_dev(data, m);
    let mut sorted = data.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

    info!(
        anomaly_count = combined.len(),
        series_len = data.len(),
        "[chat-with-data-anomaly] combined detection complete"
    );

    AnomalyReport {
        anomalies: combined,
        mean: m,
        std_dev: s,
        q1: percentile(&sorted, 25.0),
        q3: percentile(&sorted, 75.0),
        iqr: percentile(&sorted, 75.0) - percentile(&sorted, 25.0),
        series_length: data.len(),
    }
}

/// Compute mean of a slice.
fn mean(data: &[f64]) -> f64 {
    if data.is_empty() {
        return 0.0;
    }
    data.iter().sum::<f64>() / data.len() as f64
}

/// Compute standard deviation.
fn std_dev(data: &[f64], mean: f64) -> f64 {
    if data.len() < 2 {
        return 0.0;
    }
    let variance = data.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / (data.len() - 1) as f64;
    variance.sqrt()
}

/// Compute percentile using linear interpolation.
fn percentile(sorted: &[f64], p: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    if sorted.len() == 1 {
        return sorted[0];
    }
    let rank = (p / 100.0) * (sorted.len() - 1) as f64;
    let lower = rank.floor() as usize;
    let upper = rank.ceil() as usize;
    let frac = rank - lower as f64;

    if upper >= sorted.len() {
        sorted[sorted.len() - 1]
    } else {
        sorted[lower] * (1.0 - frac) + sorted[upper] * frac
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mean_basic() {
        assert!((mean(&[1.0, 2.0, 3.0, 4.0, 5.0]) - 3.0).abs() < 1e-10);
    }

    #[test]
    fn mean_empty() {
        assert_eq!(mean(&[]), 0.0);
    }

    #[test]
    fn std_dev_basic() {
        let data = [2.0, 4.0, 4.0, 4.0, 5.0, 5.0, 7.0, 9.0];
        let m = mean(&data);
        let s = std_dev(&data, m);
        assert!((s - 2.138).abs() < 0.01);
    }

    #[test]
    fn percentile_median() {
        let sorted = vec![1.0, 2.0, 3.0, 4.0, 5.0];
        assert!((percentile(&sorted, 50.0) - 3.0).abs() < 1e-10);
    }

    #[test]
    fn percentile_q1_q3() {
        let sorted = vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0];
        let q1 = percentile(&sorted, 25.0);
        let q3 = percentile(&sorted, 75.0);
        assert!(q1 > 1.0 && q1 < 4.0);
        assert!(q3 > 5.0 && q3 < 8.0);
    }

    #[test]
    fn zscore_detects_outlier() {
        let mut data = vec![10.0; 100];
        data[50] = 100.0; // Clear outlier.
        let anomalies = detect_zscore(&data, 2.5);
        assert!(!anomalies.is_empty());
        assert!(anomalies.iter().any(|a| a.index == 50));
        assert_eq!(anomalies[0].direction, AnomalyDirection::High);
    }

    #[test]
    fn zscore_no_anomalies_in_uniform() {
        let data: Vec<f64> = (0..100).map(|i| i as f64).collect();
        let anomalies = detect_zscore(&data, 3.0);
        assert!(anomalies.is_empty());
    }

    #[test]
    fn zscore_too_few_points() {
        assert!(detect_zscore(&[1.0, 2.0], 2.5).is_empty());
    }

    #[test]
    fn zscore_constant_series() {
        let data = vec![5.0; 50];
        assert!(detect_zscore(&data, 2.5).is_empty());
    }

    #[test]
    fn iqr_detects_outlier() {
        let mut data: Vec<f64> = (1..=20).map(|i| i as f64).collect();
        data.push(100.0); // Clear outlier.
        let anomalies = detect_iqr(&data, 1.5);
        assert!(!anomalies.is_empty());
        assert!(anomalies.iter().any(|a| a.value == 100.0));
    }

    #[test]
    fn iqr_detects_low_outlier() {
        let mut data: Vec<f64> = (10..=30).map(|i| i as f64).collect();
        data.push(-50.0); // Low outlier.
        let anomalies = detect_iqr(&data, 1.5);
        assert!(!anomalies.is_empty());
        assert!(anomalies.iter().any(|a| a.value == -50.0));
        assert_eq!(
            anomalies
                .iter()
                .find(|a| a.value == -50.0)
                .unwrap()
                .direction,
            AnomalyDirection::Low
        );
    }

    #[test]
    fn iqr_too_few_points() {
        assert!(detect_iqr(&[1.0, 2.0, 3.0], 1.5).is_empty());
    }

    #[test]
    fn zscore_rejects_invalid_threshold() {
        let mut data = vec![10.0; 100];
        data[50] = 100.0;
        assert!(detect_zscore(&data, 0.0).is_empty());
        assert!(detect_zscore(&data, -1.0).is_empty());
        assert!(detect_zscore(&data, f64::NAN).is_empty());
        assert!(detect_zscore(&data, f64::INFINITY).is_empty());
    }

    #[test]
    fn iqr_rejects_invalid_k() {
        let mut data: Vec<f64> = (1..=20).map(|i| i as f64).collect();
        data.push(100.0);
        assert!(detect_iqr(&data, 0.0).is_empty());
        assert!(detect_iqr(&data, -1.0).is_empty());
        assert!(detect_iqr(&data, f64::NAN).is_empty());
        assert!(detect_iqr(&data, f64::INFINITY).is_empty());
    }

    #[test]
    fn combined_boosts_dual_detection() {
        let mut data: Vec<f64> = (1..=50).map(|i| i as f64).collect();
        data.push(500.0); // Extreme outlier — both methods should catch it.
        let report = detect_combined(&data, 2.5, 1.5);
        assert!(!report.anomalies.is_empty());
        // The extreme outlier should be detected by both methods.
        let extreme = report.anomalies.iter().find(|a| a.value == 500.0).unwrap();
        assert_eq!(extreme.method, AnomalyMethod::Combined);
        assert!(extreme.score > 3.0); // Boosted score.
    }

    #[test]
    fn combined_report_has_stats() {
        let data: Vec<f64> = (1..=100).map(|i| i as f64).collect();
        let report = detect_combined(&data, 3.0, 1.5);
        assert!((report.mean - 50.5).abs() < 0.1);
        assert!(report.std_dev > 0.0);
        assert!(report.q1 < report.q3);
        assert!(report.iqr > 0.0);
        assert_eq!(report.series_length, 100);
    }

    #[test]
    fn combined_empty_data() {
        let report = detect_combined(&[], 2.5, 1.5);
        assert!(report.anomalies.is_empty());
        assert_eq!(report.series_length, 0);
    }

    #[test]
    fn anomaly_serializes() {
        let a = Anomaly {
            index: 5,
            value: 100.0,
            score: 3.5,
            method: AnomalyMethod::Combined,
            direction: AnomalyDirection::High,
        };
        let json = serde_json::to_string(&a).unwrap();
        let back: Anomaly = serde_json::from_str(&json).unwrap();
        assert_eq!(back.method, AnomalyMethod::Combined);
        assert_eq!(back.direction, AnomalyDirection::High);
    }
}
