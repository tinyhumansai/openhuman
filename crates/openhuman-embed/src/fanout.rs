//! Bounded concurrent calls with ordered results, isolated sessions and a shared budget.
//!
//! A branch can declare one level of leaf children. Leaves expose no child
//! builder. Runtime-owned agent calls keep their own agent/session policy;
//! this primitive does not enable model-directed subagent tools.
use crate::budget::{ModelBudget, SpendLimits};
use crate::complete::{Completer, CompletionRequest, CompletionResponse};
use crate::{CoreError, Runtime, Turn, TurnOutcome};
use std::num::NonZeroUsize;

/// One leaf call. It cannot declare further children.
///
/// ```compile_fail
/// use openhuman_embed::fanout::LeafCall;
/// fn recurse(leaf: LeafCall) { leaf.children(Vec::new()); }
/// ```
pub enum LeafCall {
    /// Stateless model request on its explicit route.
    Completion {
        /// The route and completion options.
        completer: Completer,
        /// The model request.
        request: Box<CompletionRequest>,
    },
    /// Independent agent turn. An unset session gets a fresh identity.
    Turn(Box<Turn>),
}
impl LeafCall {
    /// A stateless request with owned, boxed options.
    pub fn completion(completer: Completer, request: CompletionRequest) -> Self {
        Self::Completion {
            completer,
            request: Box::new(request),
        }
    }
    /// An independent agent turn with owned, boxed state.
    pub fn turn(turn: Turn) -> Self {
        Self::Turn(Box::new(turn))
    }
}
/// Either model-call result, retaining usage and answering-model metadata.
#[derive(Debug)]
pub enum CallOutcome {
    /// Completed stateless request.
    Completion(CompletionResponse),
    /// Completed agent turn.
    Turn(TurnOutcome),
}
/// A root call and an optional single level of child calls.
pub struct Branch {
    call: LeafCall,
    children: Vec<LeafCall>,
    limits: SpendLimits,
}
impl Branch {
    /// An independent branch with no children or local ceilings.
    pub fn new(call: LeafCall) -> Self {
        Self {
            call,
            children: Vec::new(),
            limits: SpendLimits::default(),
        }
    }
    /// Declare direct children. A child has no further child-building API.
    pub fn children(mut self, children: Vec<LeafCall>) -> Self {
        self.children = children;
        self
    }
    /// Apply a branch/turn ceiling in addition to the shared run ceiling.
    pub fn limits(mut self, limits: SpendLimits) -> Self {
        self.limits = limits;
        self
    }
}
/// A successful root and individually isolated, ordered child results.
#[derive(Debug)]
pub struct BranchOutcome {
    /// The root call's response.
    pub root: CallOutcome,
    /// Direct children, in declared order. One failure leaves siblings running.
    pub children: Vec<Result<CallOutcome, CoreError>>,
}
type CallFuture =
    std::pin::Pin<Box<dyn std::future::Future<Output = Result<CallOutcome, CoreError>> + Send>>;
fn call(call: LeafCall, budget: ModelBudget, leaf: bool) -> CallFuture {
    match call {
        LeafCall::Completion { completer, request } => Box::pin(async move {
            Box::pin(completer.budget(budget).complete(*request))
                .await
                .map(CallOutcome::Completion)
        }),
        LeafCall::Turn(turn) => Box::pin(async move {
            Box::pin(
                (*turn)
                    .budget(budget)
                    .with_spawn_depth_limit(if leaf { 0 } else { 1 })
                    .send(),
            )
            .await
            .map(CallOutcome::Turn)
        }),
    }
}

async fn branch(branch: Branch, budget: ModelBudget) -> Result<BranchOutcome, CoreError> {
    let budget = ModelBudget {
        ledger: budget.ledger.child(branch.limits),
        call: budget.call,
    };
    let root = call(branch.call, budget.clone(), false).await?;
    let mut children = Vec::with_capacity(branch.children.len());
    for child in branch.children {
        children.push(call(child, budget.clone(), true).await);
    }
    Ok(BranchOutcome { root, children })
}
/// Run branches concurrently without booting a runtime (useful for completers).
///
/// At most `concurrency` branches run at once; each branch processes its root
/// then its children sequentially. Input order is retained regardless of finish
/// order. A root failure skips only that root's children. A cancelled outer
/// future drops every in-flight future and conservatively charges reservations.
pub async fn fanout(
    branches: Vec<Branch>,
    concurrency: NonZeroUsize,
    budget: ModelBudget,
) -> Vec<Result<BranchOutcome, CoreError>> {
    let futures = branches
        .into_iter()
        .map(|branch_input| {
            Box::pin(branch(branch_input, budget.clone()))
                as BranchFuture<'_, BranchOutcome, CoreError>
        })
        .collect();
    fanout_futures(futures, concurrency).await
}

/// A borrowed branch future. It need not be `'static` or spawn a task.
pub type BranchFuture<'a, T, E> =
    std::pin::Pin<Box<dyn std::future::Future<Output = Result<T, E>> + Send + 'a>>;

/// Poll borrowed host futures concurrently and retain input-order results.
///
/// A future owns its own call/budget policy; this scheduler makes no assumption
/// that an opaque future is a model call. Configure `Turn::budget` or
/// `Completer::budget` inside each model future. Dropping this scheduler drops
/// every active future and never detaches work.
pub async fn fanout_futures<'a, T, E>(
    futures: Vec<BranchFuture<'a, T, E>>,
    concurrency: NonZeroUsize,
) -> Vec<Result<T, E>> {
    let mut pending = futures.into_iter().enumerate();
    let mut active: Vec<(usize, BranchFuture<'a, T, E>)> = Vec::new();
    let mut results = Vec::new();
    for (index, future) in pending.by_ref().take(concurrency.get()) {
        active.push((index, future));
        results.push(None);
    }
    while !active.is_empty() {
        let (slot, outcome) = std::future::poll_fn(|cx| {
            for (slot, (_, future)) in active.iter_mut().enumerate() {
                if let std::task::Poll::Ready(result) = future.as_mut().poll(cx) {
                    return std::task::Poll::Ready((slot, result));
                }
            }
            std::task::Poll::Pending
        })
        .await;
        let (index, _) = active.swap_remove(slot);
        results[index] = Some(outcome);
        if let Some((index, future)) = pending.next() {
            active.push((index, future));
            results.push(None);
        }
    }
    results
        .into_iter()
        .map(|result| result.expect("each admitted branch settled"))
        .collect()
}

impl Runtime {
    /// Run independent model/agent branches with ordered results and a shared ledger.
    pub async fn fanout(
        &self,
        branches: Vec<Branch>,
        concurrency: NonZeroUsize,
        budget: ModelBudget,
    ) -> Vec<Result<BranchOutcome, CoreError>> {
        fanout(branches, concurrency, budget).await
    }
}
