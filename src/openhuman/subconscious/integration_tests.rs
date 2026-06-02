#[cfg(test)]
mod tests {
    use crate::openhuman::subconscious::reflection::{
        hydrate_draft, ReflectionDraft, ReflectionKind,
    };
    use crate::openhuman::subconscious::reflection_store;
    use crate::openhuman::subconscious::store;

    #[test]
    fn reflection_with_thread_id_persists() {
        let dir = tempfile::tempdir().unwrap();
        store::with_connection(dir.path(), |conn| {
            let draft = ReflectionDraft {
                kind: ReflectionKind::Opportunity,
                body: "Test thought".into(),
                proposed_action: Some("Do something".into()),
                source_refs: vec!["entity:test".into()],
            };
            let reflection = hydrate_draft(
                draft,
                "r-1".into(),
                1_700_000_000.0,
                Vec::new(),
                Some("thread-abc".into()),
            );
            reflection_store::add_reflection(conn, &reflection)?;

            let got = reflection_store::get_reflection(conn, "r-1")?.unwrap();
            assert_eq!(got.thread_id, Some("thread-abc".into()));
            assert_eq!(got.body, "Test thought");
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn reflection_without_thread_id_persists() {
        let dir = tempfile::tempdir().unwrap();
        store::with_connection(dir.path(), |conn| {
            let draft = ReflectionDraft {
                kind: ReflectionKind::DailyDigest,
                body: "No thread".into(),
                proposed_action: None,
                source_refs: vec![],
            };
            let reflection = hydrate_draft(draft, "r-2".into(), 1_700_000_000.0, Vec::new(), None);
            reflection_store::add_reflection(conn, &reflection)?;

            let got = reflection_store::get_reflection(conn, "r-2")?.unwrap();
            assert!(got.thread_id.is_none());
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn list_recent_includes_thread_id() {
        let dir = tempfile::tempdir().unwrap();
        store::with_connection(dir.path(), |conn| {
            for i in 0..3 {
                let draft = ReflectionDraft {
                    kind: ReflectionKind::HotnessSpike,
                    body: format!("thought {i}"),
                    proposed_action: None,
                    source_refs: vec![],
                };
                let tid = if i == 1 {
                    Some("thread-xyz".into())
                } else {
                    None
                };
                let reflection = hydrate_draft(
                    draft,
                    format!("r-{i}"),
                    1_700_000_000.0 + f64::from(i),
                    Vec::new(),
                    tid,
                );
                reflection_store::add_reflection(conn, &reflection)?;
            }

            let list = reflection_store::list_recent(conn, 10, None)?;
            assert_eq!(list.len(), 3);
            assert_eq!(list[1].thread_id, Some("thread-xyz".into()));
            assert!(list[0].thread_id.is_none());
            Ok(())
        })
        .unwrap();
    }
}
