use super::*;
use std::cell::Cell;
use std::future::Future;
use std::sync::Arc;
use std::task::{Context, Poll, Wake, Waker};

struct Noop;
impl Wake for Noop {
    fn wake(self: Arc<Self>) {}
}

#[test]
fn a_closed_instance_registers_nothing_and_publishes_neither_flow_surface() {
    let scope = ApprovalScope::default();
    scope.close("agent_removed");
    let publications = Cell::new(0);
    let mut publication = std::pin::pin!(register_after_workspace(
        Some(&scope),
        std::future::ready(17),
        |_| publications.set(publications.get() + 1),
    ));
    let waker = Waker::from(Arc::new(Noop));
    assert_eq!(
        publication.as_mut().poll(&mut Context::from_waker(&waker)),
        Poll::Ready(Err("agent_removed".to_owned()))
    );
    assert_eq!(publications.get(), 0);
}

#[test]
fn removal_during_workspace_lookup_suppresses_registration_and_all_surfaces() {
    let scope = ApprovalScope::default();
    let ready = Cell::new(false);
    let workspace = std::future::poll_fn(|_| {
        if ready.get() {
            Poll::Ready(17)
        } else {
            Poll::Pending
        }
    });
    let publications = Cell::new(0);
    let mut publication = std::pin::pin!(register_after_workspace(Some(&scope), workspace, |_| {
        publications.set(publications.get() + 1)
    },));
    let waker = Waker::from(Arc::new(Noop));
    let mut cx = Context::from_waker(&waker);
    assert_eq!(publication.as_mut().poll(&mut cx), Poll::Pending);
    assert_eq!(
        publications.get(),
        0,
        "registration preceded workspace resolution"
    );
    scope.close("agent_removed");
    ready.set(true);
    assert_eq!(
        publication.as_mut().poll(&mut cx),
        Poll::Ready(Err("agent_removed".to_owned()))
    );
    assert_eq!(publications.get(), 0);
}

#[test]
fn a_live_or_unscoped_flow_registers_once_with_its_resolved_workspace() {
    let scope = ApprovalScope::default();
    for scope in [Some(&scope), None] {
        let publications = Cell::new(0);
        let mut publication = std::pin::pin!(register_after_workspace(
            scope,
            std::future::ready(Some(("ws_test", 17))),
            |workspace| {
                assert_eq!(workspace, Some(("ws_test", 17)));
                publications.set(publications.get() + 1);
                42
            },
        ));
        let waker = Waker::from(Arc::new(Noop));
        assert_eq!(
            publication.as_mut().poll(&mut Context::from_waker(&waker)),
            Poll::Ready(Ok(42))
        );
        assert_eq!(publications.get(), 1);
    }
}
