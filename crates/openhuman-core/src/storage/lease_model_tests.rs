//! Model-based tests: random acquire / renew / release / clock / crash runs
//! across 2–4 nodes on one `MemoryStorage`, checked step by step against a
//! small reference model of the lease rules, plus the safety invariants:
//!
//! - at most one node holds an unexpired grant at any instant;
//! - epochs only grow, and only a re-entrant acquire repeats one;
//! - a crashed holder's lease is taken by another node only after it
//!   expires, and the taker sees `previous_unclean`;
//! - a renew (or release) after the record moved on is always `Lost`.

use super::*;

use std::time::Duration;

use proptest::prelude::*;

use crate::storage::MemoryStorage;

const KEY: &str = "profile-1";
const TTL_MS: u64 = 1_000;

#[derive(Debug, Clone)]
enum Op {
    Acquire(usize),
    Renew(usize),
    Release(usize),
    Advance(u64),
    Crash(usize),
}

fn ops(nodes: usize) -> impl Strategy<Value = Op> {
    prop_oneof![
        4 => (0..nodes).prop_map(Op::Acquire),
        3 => (0..nodes).prop_map(Op::Renew),
        1 => (0..nodes).prop_map(Op::Release),
        3 => (0..1_500u64).prop_map(Op::Advance),
        1 => (0..nodes).prop_map(Op::Crash),
    ]
}

/// The model's copy of the stored record.
#[derive(Debug, Clone)]
struct Rec {
    owner: usize,
    epoch: u64,
    expires_at_ms: u64,
    released: bool,
    version: u64,
}

/// What the model expects a step to return.
#[derive(Debug, PartialEq, Eq)]
enum Expect {
    Granted { epoch: u64, unclean: bool },
    Held { owner: usize },
    Lost,
    Done,
    Skip,
}

struct Model {
    now: u64,
    rec: Option<Rec>,
    /// Per node: the epoch the current incarnation holds and the version of
    /// its grant.
    held: Vec<Option<u64>>,
    grant: Vec<Option<u64>>,
}

impl Model {
    fn new(nodes: usize) -> Self {
        Self {
            now: 0,
            rec: None,
            held: vec![None; nodes],
            grant: vec![None; nodes],
        }
    }

    fn write(&mut self, owner: usize, epoch: u64, expires_at_ms: u64, released: bool) -> u64 {
        let version = self.rec.as_ref().map_or(1, |r| r.version + 1);
        self.rec = Some(Rec {
            owner,
            epoch,
            expires_at_ms,
            released,
            version,
        });
        version
    }

    fn acquire(&mut self, n: usize) -> Expect {
        let (epoch, unclean) = match &self.rec {
            None => (1, false),
            Some(r) if r.released => (r.epoch + 1, false),
            Some(r)
                if r.owner == n && self.held[n] == Some(r.epoch) && self.now < r.expires_at_ms =>
            {
                (r.epoch, false)
            }
            Some(r) if r.owner == n => (r.epoch + 1, true),
            Some(r) if self.now >= r.expires_at_ms => (r.epoch + 1, true),
            Some(r) => return Expect::Held { owner: r.owner },
        };
        let version = self.write(n, epoch, self.now + TTL_MS, false);
        self.held[n] = Some(epoch);
        self.grant[n] = Some(version);
        Expect::Granted { epoch, unclean }
    }

    fn current(&self, n: usize) -> Option<bool> {
        let version = self.grant[n]?;
        Some(self.rec.as_ref().is_some_and(|r| r.version == version))
    }

    fn renew(&mut self, n: usize) -> Expect {
        match self.current(n) {
            None => Expect::Skip,
            Some(false) => {
                self.grant[n] = None;
                Expect::Lost
            }
            Some(true)
                if self
                    .rec
                    .as_ref()
                    .is_some_and(|r| self.now >= r.expires_at_ms) =>
            {
                self.grant[n] = None;
                Expect::Lost
            }
            Some(true) => {
                let epoch = self.rec.as_ref().map_or(0, |r| r.epoch);
                let version = self.write(n, epoch, self.now + TTL_MS, false);
                self.grant[n] = Some(version);
                Expect::Done
            }
        }
    }

    fn release(&mut self, n: usize) -> Expect {
        match self.current(n) {
            None => Expect::Skip,
            Some(false) => {
                self.grant[n] = None;
                Expect::Lost
            }
            Some(true) => {
                let (epoch, expires) = self
                    .rec
                    .as_ref()
                    .map_or((0, 0), |r| (r.epoch, r.expires_at_ms));
                self.write(n, epoch, expires, true);
                self.grant[n] = None;
                self.held[n] = None;
                Expect::Done
            }
        }
    }
}

struct System {
    storage: MemoryStorage,
    leases: Vec<DocumentLeases>,
    grants: Vec<Option<LeaseGrant>>,
}

fn name(n: usize) -> String {
    format!("node-{n}")
}

impl System {
    fn new(nodes: usize) -> Self {
        let storage = MemoryStorage::new();
        let leases = (0..nodes).map(|n| Self::boot(&storage, n)).collect();
        Self {
            storage,
            leases,
            grants: vec![None; nodes],
        }
    }

    fn boot(storage: &MemoryStorage, n: usize) -> DocumentLeases {
        DocumentLeases::cluster(storage, name(n), None, Duration::from_millis(TTL_MS)).unwrap()
    }

    fn crash(&mut self, n: usize) {
        self.leases[n] = Self::boot(&self.storage, n);
        self.grants[n] = None;
    }
}

async fn run(nodes: usize, steps: Vec<Op>) -> Result<(), TestCaseError> {
    let mut model = Model::new(nodes);
    let mut system = System::new(nodes);
    let mut max_epoch = 0u64;

    for (step, op) in steps.into_iter().enumerate() {
        let now = model.now;
        let before = model.rec.clone();
        let (expected, actual) = match op {
            Op::Advance(ms) => {
                model.now += ms;
                (Expect::Done, Expect::Done)
            }
            Op::Crash(n) => {
                model.held[n] = None;
                model.grant[n] = None;
                system.crash(n);
                (Expect::Done, Expect::Done)
            }
            Op::Acquire(n) => {
                let expected = model.acquire(n);
                let actual = match system.leases[n].acquire(KEY, now).await {
                    Ok(grant) => {
                        let got = Expect::Granted {
                            epoch: grant.epoch,
                            unclean: grant.previous_unclean,
                        };
                        let reentrant = before.as_ref().is_some_and(|r| {
                            !r.released && r.epoch == grant.epoch && now < r.expires_at_ms
                        });
                        if reentrant {
                            prop_assert_eq!(grant.epoch, max_epoch, "step {}", step);
                        } else {
                            prop_assert!(grant.epoch > max_epoch, "step {}: epoch went back", step);
                        }
                        max_epoch = max_epoch.max(grant.epoch);
                        if let Some(prev) = before.as_ref().filter(|r| !r.released && r.owner != n)
                        {
                            prop_assert!(
                                now >= prev.expires_at_ms && grant.previous_unclean,
                                "step {}: took a live or unflagged foreign lease",
                                step
                            );
                        }
                        system.grants[n] = Some(grant);
                        got
                    }
                    Err(LeaseError::Held(record)) => Expect::Held {
                        owner: record
                            .owner
                            .trim_start_matches("node-")
                            .parse()
                            .unwrap_or(usize::MAX),
                    },
                    Err(other) => return Err(TestCaseError::fail(format!("{other:?}"))),
                };
                (expected, actual)
            }
            Op::Renew(n) | Op::Release(n) => {
                let renew = matches!(op, Op::Renew(_));
                let expected = if renew {
                    model.renew(n)
                } else {
                    model.release(n)
                };
                let actual = match system.grants[n].clone() {
                    None => Expect::Skip,
                    Some(grant) => {
                        let stolen = before.as_ref().is_some_and(|r| r.owner != n);
                        let outcome = if renew {
                            system.leases[n].renew(&grant, now).await.map(Some)
                        } else {
                            system.leases[n].release(grant).await.map(|()| None)
                        };
                        match outcome {
                            Ok(next) => {
                                prop_assert!(!stolen, "step {}: renewed a stolen lease", step);
                                system.grants[n] = next;
                                Expect::Done
                            }
                            Err(LeaseError::Lost) => {
                                system.grants[n] = None;
                                Expect::Lost
                            }
                            Err(other) => return Err(TestCaseError::fail(format!("{other:?}"))),
                        }
                    }
                };
                (expected, actual)
            }
        };
        prop_assert_eq!(&actual, &expected, "step {}", step);

        let live = system
            .grants
            .iter()
            .flatten()
            .filter(|g| g.expires_at_ms > model.now)
            .count();
        prop_assert!(live <= 1, "step {}: {} live holders", step, live);

        let stored = system.leases[0]
            .holder(KEY)
            .await
            .map_err(|e| TestCaseError::fail(format!("{e:?}")))?;
        match (&stored, &model.rec) {
            (None, None) => {}
            (Some(s), Some(m)) => {
                prop_assert_eq!(&s.owner, &name(m.owner), "step {}", step);
                prop_assert_eq!(s.epoch, m.epoch, "step {}", step);
                prop_assert_eq!(s.expires_at_ms, m.expires_at_ms, "step {}", step);
                prop_assert_eq!(s.released, m.released, "step {}", step);
            }
            _ => prop_assert!(
                false,
                "step {}: stored {:?} vs model {:?}",
                step,
                stored,
                model.rec
            ),
        }
    }
    Ok(())
}

fn scenario() -> impl Strategy<Value = (usize, Vec<Op>)> {
    (2usize..=4).prop_flat_map(|nodes| (Just(nodes), prop::collection::vec(ops(nodes), 1..80)))
}

/// Preserve the saved lease-model case: the same holder reacquires at exact expiry.
#[tokio::test]
async fn reacquiring_at_expiry_advances_epoch_and_marks_previous_holder_unclean() {
    run(2, vec![Op::Acquire(0), Op::Advance(1_000), Op::Acquire(0)])
        .await
        .unwrap();
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 256,
        failure_persistence: None,
        ..ProptestConfig::default()
    })]

    #[test]
    fn leases_match_the_model_and_never_double_grant((nodes, steps) in scenario()) {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(run(nodes, steps))?;
    }
}
