use super::*;
use std::collections::HashMap;

pub(super) struct RuntimeGateState {
    pub permits: Arc<Semaphore>,
    pub signed_out: bool,
}

fn map() -> &'static parking_lot::Mutex<HashMap<tokio::runtime::Id, RuntimeGateState>> {
    static M: OnceLock<parking_lot::Mutex<HashMap<tokio::runtime::Id, RuntimeGateState>>> =
        OnceLock::new();
    M.get_or_init(|| parking_lot::Mutex::new(HashMap::new()))
}

/// Current tokio runtime ID, or `None` outside any runtime (sync tests).
pub(super) fn current_id() -> Option<tokio::runtime::Id> {
    tokio::runtime::Handle::try_current().ok().map(|h| h.id())
}

pub(super) fn permits_for(id: tokio::runtime::Id) -> Arc<Semaphore> {
    let mut g = map().lock();
    g.entry(id)
        .or_insert_with(|| RuntimeGateState {
            permits: Arc::new(Semaphore::new(LLM_SLOTS)),
            signed_out: false,
        })
        .permits
        .clone()
}

pub(super) fn signed_out_for(id: tokio::runtime::Id) -> bool {
    let mut g = map().lock();
    g.entry(id)
        .or_insert_with(|| RuntimeGateState {
            permits: Arc::new(Semaphore::new(LLM_SLOTS)),
            signed_out: false,
        })
        .signed_out
}

pub(super) fn set_signed_out_for(id: tokio::runtime::Id, v: bool) -> bool {
    let mut g = map().lock();
    let entry = g.entry(id).or_insert_with(|| RuntimeGateState {
        permits: Arc::new(Semaphore::new(LLM_SLOTS)),
        signed_out: false,
    });
    let prev = entry.signed_out;
    entry.signed_out = v;
    prev
}
