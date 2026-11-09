use super::*;

use std::sync::Arc;

#[test]
fn admission_rejection_kinds_remain_indexed_under_concurrent_policy_calls() {
    let kinds = Arc::new(RejectionKinds::default());
    let calls = [
        (0, ParallelTaskRejectionKind::MissingAgentOrPrompt),
        (1, ParallelTaskRejectionKind::UnknownAgent),
        (2, ParallelTaskRejectionKind::OutsideAllowlist),
        (3, ParallelTaskRejectionKind::RequiresIsolation),
    ];
    let workers = calls
        .into_iter()
        .map(|(index, kind)| {
            let kinds = Arc::clone(&kinds);
            std::thread::spawn(move || kinds.insert(index, kind))
        })
        .collect::<Vec<_>>();
    for worker in workers {
        worker.join().unwrap();
    }

    let kinds = match Arc::try_unwrap(kinds) {
        Ok(kinds) => kinds.into_inner(),
        Err(_) => panic!("all admission callers released the shared policy state"),
    };
    assert_eq!(kinds.len(), 4);
    assert_eq!(
        kinds.get(&0),
        Some(&ParallelTaskRejectionKind::MissingAgentOrPrompt)
    );
    assert_eq!(
        kinds.get(&1),
        Some(&ParallelTaskRejectionKind::UnknownAgent)
    );
    assert_eq!(
        kinds.get(&2),
        Some(&ParallelTaskRejectionKind::OutsideAllowlist)
    );
    assert_eq!(
        kinds.get(&3),
        Some(&ParallelTaskRejectionKind::RequiresIsolation)
    );
}
