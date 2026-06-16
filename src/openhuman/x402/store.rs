//! x402 payment ledger — append-only JSONL persistence for payment records
//! with session/daily/monthly budget enforcement.

use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

use chrono::{DateTime, Datelike, Utc};
use log::{debug, warn};
use once_cell::sync::Lazy;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};

const LOG_PREFIX: &str = "[x402::store]";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PaymentRecord {
    pub id: String,
    pub url: String,
    pub asset: String,
    pub amount_atomic: u64,
    pub amount_display: String,
    pub recipient: String,
    pub network: String,
    pub tx_signature: Option<String>,
    pub status: PaymentStatus,
    pub timestamp: DateTime<Utc>,
    pub session_id: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PaymentStatus {
    Pending,
    Settled,
    Failed,
    Denied,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SpendingSummary {
    pub session_total_atomic: u64,
    pub daily_total_atomic: u64,
    pub monthly_total_atomic: u64,
    pub session_count: usize,
    pub daily_count: usize,
    pub monthly_count: usize,
}

impl Default for SpendingSummary {
    fn default() -> Self {
        Self {
            session_total_atomic: 0,
            daily_total_atomic: 0,
            monthly_total_atomic: 0,
            session_count: 0,
            daily_count: 0,
            monthly_count: 0,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SpendingBudget {
    pub per_request_max_atomic: u64,
    pub daily_max_atomic: u64,
    pub monthly_max_atomic: u64,
}

impl Default for SpendingBudget {
    fn default() -> Self {
        Self {
            // 1 USDC per request
            per_request_max_atomic: 1_000_000,
            // 10 USDC per day
            daily_max_atomic: 10_000_000,
            // 100 USDC per month
            monthly_max_atomic: 100_000_000,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BudgetCheck {
    Allowed,
    ExceedsPerRequest { requested: u64, cap: u64 },
    ExceedsDailyBudget { current: u64, cap: u64 },
    ExceedsMonthlyBudget { current: u64, cap: u64 },
}

pub struct PaymentLedger {
    records: Vec<PaymentRecord>,
    file_path: PathBuf,
    budget: SpendingBudget,
    session_id: String,
}

static GLOBAL_LEDGER: Lazy<Mutex<Option<PaymentLedger>>> = Lazy::new(|| Mutex::new(None));

impl PaymentLedger {
    pub fn new(workspace_dir: &Path, session_id: &str, budget: SpendingBudget) -> Self {
        let file_path = workspace_dir.join("x402").join("payments.jsonl");
        let records = Self::load_from_disk(&file_path);
        debug!(
            "{LOG_PREFIX} loaded {} existing payment records from {}",
            records.len(),
            file_path.display()
        );
        Self {
            records,
            file_path,
            budget,
            session_id: session_id.to_string(),
        }
    }

    fn load_from_disk(path: &Path) -> Vec<PaymentRecord> {
        let file = match fs::File::open(path) {
            Ok(f) => f,
            Err(_) => return Vec::new(),
        };
        let reader = BufReader::new(file);
        let mut records = Vec::new();
        for line in reader.lines() {
            let line = match line {
                Ok(l) => l,
                Err(e) => {
                    warn!("{LOG_PREFIX} read error: {e}");
                    continue;
                }
            };
            if line.trim().is_empty() {
                continue;
            }
            match serde_json::from_str::<PaymentRecord>(&line) {
                Ok(r) => records.push(r),
                Err(e) => warn!("{LOG_PREFIX} corrupt record: {e}"),
            }
        }
        records
    }

    fn append_to_disk(&self, record: &PaymentRecord) {
        if let Some(parent) = self.file_path.parent() {
            if let Err(e) = fs::create_dir_all(parent) {
                warn!("{LOG_PREFIX} mkdir failed: {e}");
                return;
            }
        }
        let mut file = match OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.file_path)
        {
            Ok(f) => f,
            Err(e) => {
                warn!("{LOG_PREFIX} open failed: {e}");
                return;
            }
        };
        match serde_json::to_string(record) {
            Ok(json) => {
                if let Err(e) = writeln!(file, "{json}") {
                    warn!("{LOG_PREFIX} write failed: {e}");
                }
            }
            Err(e) => warn!("{LOG_PREFIX} serialize failed: {e}"),
        }
    }

    pub fn check_budget(&self, amount: u64) -> BudgetCheck {
        if amount > self.budget.per_request_max_atomic {
            return BudgetCheck::ExceedsPerRequest {
                requested: amount,
                cap: self.budget.per_request_max_atomic,
            };
        }

        let now = Utc::now();
        let today = now.date_naive();
        let this_month = (now.year(), now.month());

        let daily: u64 = self
            .records
            .iter()
            .filter(|r| r.status == PaymentStatus::Settled && r.timestamp.date_naive() == today)
            .map(|r| r.amount_atomic)
            .sum();

        if daily.saturating_add(amount) > self.budget.daily_max_atomic {
            return BudgetCheck::ExceedsDailyBudget {
                current: daily,
                cap: self.budget.daily_max_atomic,
            };
        }

        let monthly: u64 = self
            .records
            .iter()
            .filter(|r| {
                r.status == PaymentStatus::Settled && {
                    let ts = r.timestamp;
                    (ts.year(), ts.month()) == this_month
                }
            })
            .map(|r| r.amount_atomic)
            .sum();

        if monthly.saturating_add(amount) > self.budget.monthly_max_atomic {
            return BudgetCheck::ExceedsMonthlyBudget {
                current: monthly,
                cap: self.budget.monthly_max_atomic,
            };
        }

        BudgetCheck::Allowed
    }

    pub fn record_payment(&mut self, record: PaymentRecord) {
        self.append_to_disk(&record);
        self.records.push(record);
    }

    pub fn summary(&self) -> SpendingSummary {
        let now = Utc::now();
        let today = now.date_naive();
        let this_month = (now.year(), now.month());

        let settled: Vec<&PaymentRecord> = self
            .records
            .iter()
            .filter(|r| r.status == PaymentStatus::Settled)
            .collect();

        let session: Vec<&&PaymentRecord> = settled
            .iter()
            .filter(|r| r.session_id == self.session_id)
            .collect();

        let daily: Vec<&&PaymentRecord> = settled
            .iter()
            .filter(|r| r.timestamp.date_naive() == today)
            .collect();

        let monthly: Vec<&&PaymentRecord> = settled
            .iter()
            .filter(|r| {
                let ts = r.timestamp;
                (ts.year(), ts.month()) == this_month
            })
            .collect();

        SpendingSummary {
            session_total_atomic: session.iter().map(|r| r.amount_atomic).sum(),
            daily_total_atomic: daily.iter().map(|r| r.amount_atomic).sum(),
            monthly_total_atomic: monthly.iter().map(|r| r.amount_atomic).sum(),
            session_count: session.len(),
            daily_count: daily.len(),
            monthly_count: monthly.len(),
        }
    }

    pub fn recent_payments(&self, limit: usize) -> Vec<PaymentRecord> {
        self.records.iter().rev().take(limit).cloned().collect()
    }

    pub fn budget(&self) -> &SpendingBudget {
        &self.budget
    }

    pub fn update_budget(&mut self, budget: SpendingBudget) {
        debug!(
            "{LOG_PREFIX} budget updated per_request={} daily={} monthly={}",
            budget.per_request_max_atomic, budget.daily_max_atomic, budget.monthly_max_atomic
        );
        self.budget = budget;
    }
}

pub fn init_global(workspace_dir: &Path, session_id: &str) {
    let budget = budget_from_env();
    let ledger = PaymentLedger::new(workspace_dir, session_id, budget);
    *GLOBAL_LEDGER.lock() = Some(ledger);
    debug!("{LOG_PREFIX} global ledger initialized");
}

fn budget_from_env() -> SpendingBudget {
    let mut budget = SpendingBudget::default();
    if let Ok(v) = std::env::var("OPENHUMAN_X402_PER_REQUEST_MAX") {
        if let Ok(n) = v.parse::<u64>() {
            debug!("{LOG_PREFIX} env override per_request_max={n}");
            budget.per_request_max_atomic = n;
        }
    }
    if let Ok(v) = std::env::var("OPENHUMAN_X402_DAILY_MAX") {
        if let Ok(n) = v.parse::<u64>() {
            debug!("{LOG_PREFIX} env override daily_max={n}");
            budget.daily_max_atomic = n;
        }
    }
    if let Ok(v) = std::env::var("OPENHUMAN_X402_MONTHLY_MAX") {
        if let Ok(n) = v.parse::<u64>() {
            debug!("{LOG_PREFIX} env override monthly_max={n}");
            budget.monthly_max_atomic = n;
        }
    }
    budget
}

pub fn with_ledger<F, R>(f: F) -> Result<R, String>
where
    F: FnOnce(&PaymentLedger) -> R,
{
    let guard = GLOBAL_LEDGER.lock();
    let ledger = guard
        .as_ref()
        .ok_or_else(|| "x402 payment ledger not initialized".to_string())?;
    Ok(f(ledger))
}

pub fn with_ledger_mut<F, R>(f: F) -> Result<R, String>
where
    F: FnOnce(&mut PaymentLedger) -> R,
{
    let mut guard = GLOBAL_LEDGER.lock();
    let ledger = guard
        .as_mut()
        .ok_or_else(|| "x402 payment ledger not initialized".to_string())?;
    Ok(f(ledger))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_budget() -> SpendingBudget {
        SpendingBudget {
            per_request_max_atomic: 500_000,
            daily_max_atomic: 2_000_000,
            monthly_max_atomic: 10_000_000,
        }
    }

    const TEST_SESSION: &str = "test";

    fn make_record(amount: u64, status: PaymentStatus) -> PaymentRecord {
        PaymentRecord {
            id: uuid::Uuid::new_v4().to_string(),
            url: "https://api.example.com/data".into(),
            asset: "USDC".into(),
            amount_atomic: amount,
            amount_display: format!("{:.6} USDC", amount as f64 / 1_000_000.0),
            recipient: "RecipientPubkey".into(),
            network: "solana:5eykt4UsFv8P8NJdTREpY1vzqKqZKvdp".into(),
            tx_signature: Some("sig123".into()),
            status,
            timestamp: Utc::now(),
            session_id: TEST_SESSION.into(),
        }
    }

    #[test]
    fn budget_check_allows_within_limits() {
        let ledger = PaymentLedger {
            records: vec![],
            file_path: PathBuf::from("/tmp/test-x402.jsonl"),
            budget: test_budget(),
            session_id: "test".into(),
        };
        assert_eq!(ledger.check_budget(100_000), BudgetCheck::Allowed);
    }

    #[test]
    fn budget_check_rejects_over_per_request() {
        let ledger = PaymentLedger {
            records: vec![],
            file_path: PathBuf::from("/tmp/test-x402.jsonl"),
            budget: test_budget(),
            session_id: "test".into(),
        };
        assert_eq!(
            ledger.check_budget(600_000),
            BudgetCheck::ExceedsPerRequest {
                requested: 600_000,
                cap: 500_000
            }
        );
    }

    #[test]
    fn budget_check_rejects_over_daily() {
        let mut ledger = PaymentLedger {
            records: vec![],
            file_path: PathBuf::from("/tmp/test-x402.jsonl"),
            budget: test_budget(),
            session_id: "test".into(),
        };
        ledger
            .records
            .push(make_record(1_800_000, PaymentStatus::Settled));
        assert_eq!(
            ledger.check_budget(400_000),
            BudgetCheck::ExceedsDailyBudget {
                current: 1_800_000,
                cap: 2_000_000
            }
        );
    }

    #[test]
    fn budget_check_ignores_failed_payments() {
        let mut ledger = PaymentLedger {
            records: vec![],
            file_path: PathBuf::from("/tmp/test-x402.jsonl"),
            budget: test_budget(),
            session_id: "test".into(),
        };
        ledger
            .records
            .push(make_record(1_800_000, PaymentStatus::Failed));
        assert_eq!(ledger.check_budget(400_000), BudgetCheck::Allowed);
    }

    #[test]
    fn summary_aggregates_correctly() {
        let mut ledger = PaymentLedger {
            records: vec![],
            file_path: PathBuf::from("/tmp/test-x402.jsonl"),
            budget: test_budget(),
            session_id: "test".into(),
        };
        ledger
            .records
            .push(make_record(100_000, PaymentStatus::Settled));
        ledger
            .records
            .push(make_record(200_000, PaymentStatus::Settled));
        ledger
            .records
            .push(make_record(50_000, PaymentStatus::Failed));

        let summary = ledger.summary();
        assert_eq!(summary.session_total_atomic, 300_000);
        assert_eq!(summary.session_count, 2);
    }
}
