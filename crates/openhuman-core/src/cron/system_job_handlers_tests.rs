use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

fn context(name: &str) -> SystemJobContext {
    SystemJobContext {
        job_id: format!("job-{name}"),
        name: name.to_string(),
    }
}

fn answering(result: Result<(), String>, calls: Arc<AtomicUsize>) -> SystemJobHandler {
    Arc::new(move |_ctx| {
        calls.fetch_add(1, Ordering::SeqCst);
        let result = result.clone();
        Box::pin(async move { result })
    })
}

#[tokio::test]
async fn a_job_without_a_handler_is_not_dispatched() {
    assert!(dispatch(context("handlers-unclaimed")).await.is_none());
}

#[tokio::test]
async fn a_registered_handler_receives_the_job_and_its_result_is_returned() {
    let seen = Arc::new(std::sync::Mutex::new(None));
    let sink = Arc::clone(&seen);
    let _registration = register(
        "handlers-echo",
        Arc::new(move |ctx: SystemJobContext| {
            *sink.lock().unwrap() = Some(ctx.clone());
            Box::pin(async { Err("handler said no".to_string()) })
        }),
    );
    let result = dispatch(context("handlers-echo")).await;
    assert_eq!(result, Some(Err("handler said no".to_string())));
    let ctx = seen.lock().unwrap().clone().expect("handler ran");
    assert_eq!(ctx.name, "handlers-echo");
    assert_eq!(ctx.job_id, "job-handlers-echo");
}

#[tokio::test]
async fn dropping_the_registration_unregisters_the_handler() {
    let calls = Arc::new(AtomicUsize::new(0));
    let registration = register("handlers-dropped", answering(Ok(()), Arc::clone(&calls)));
    assert_eq!(dispatch(context("handlers-dropped")).await, Some(Ok(())));
    drop(registration);
    assert!(dispatch(context("handlers-dropped")).await.is_none());
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn a_replaced_registration_does_not_remove_its_replacement() {
    let first_calls = Arc::new(AtomicUsize::new(0));
    let second_calls = Arc::new(AtomicUsize::new(0));
    let first = register(
        "handlers-replaced",
        answering(Ok(()), Arc::clone(&first_calls)),
    );
    let second = register(
        "handlers-replaced",
        answering(Err("second".into()), Arc::clone(&second_calls)),
    );
    drop(first);
    assert_eq!(
        dispatch(context("handlers-replaced")).await,
        Some(Err("second".to_string()))
    );
    assert_eq!(first_calls.load(Ordering::SeqCst), 0);
    assert_eq!(second_calls.load(Ordering::SeqCst), 1);
    drop(second);
}

#[test]
fn has_handler_reports_registration() {
    assert!(!has_handler("handlers-presence"));
    let registration = register(
        "handlers-presence",
        answering(Ok(()), Arc::new(AtomicUsize::new(0))),
    );
    assert!(has_handler("handlers-presence"));
    drop(registration);
    assert!(!has_handler("handlers-presence"));
}
