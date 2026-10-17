use super::*;

use crate::memory::scope::MemoryIdentity;
use tinymemory_tools::layout::placement::file_into;

use tinymemory_api::{ItemKind, MemoryMeta, MetaFilter};

use crate::memory::test_fixtures::{bind_reference, config_in, stored};

fn document(mime: Option<&str>, path: Option<&str>) -> StoreItem {
    let mut meta = MemoryMeta::default();
    meta.file_path = path.map(str::to_string);
    meta.agent_id = Some("someone".into());
    StoreItem::Document {
        title: None,
        body: tinymemory_api::DocumentBody::Text("body".into()),
        mime: mime.map(str::to_string),
        meta,
    }
}

#[test]
fn synced_items_are_filed_by_their_connector() {
    for (kind, target, want) in [
        (MemorySourceKind::Github, "o/r", BrainSource::Github),
        (MemorySourceKind::Link, "https://x", BrainSource::Web),
        (MemorySourceKind::Rss, "https://x/feed", BrainSource::Web),
        // Local files, whatever their format, share one source.
        (MemorySourceKind::Folder, "/n", files_source()),
        (MemorySourceKind::File, "/n/deck/Q3.PDF", files_source()),
    ] {
        assert_eq!(brain_source(kind), want, "{kind:?} {target}");
    }
    assert_eq!(files_source().to_string(), "files");
}

#[tokio::test]
async fn a_sources_collections_count_and_forget_with_it() {
    use tinymemory_api::MemoryEngine as _;
    let tmp = tempfile::tempdir().unwrap();
    let config = config_in(&tmp);
    let engine = bind_reference(&config);
    let layout = MemoryLayout::default();
    for (node, text) in [
        (layout.brain(&BrainSource::Github).unwrap(), "org notes"),
        (
            layout
                .brain_collection(&BrainSource::Github, "acme-api")
                .unwrap(),
            "an issue",
        ),
        (layout.brain(&BrainSource::Notion).unwrap(), "a page"),
    ] {
        let meta = MemoryMeta {
            namespace: node,
            ..MemoryMeta::default()
        };
        engine.store(StoreItem::document(text, meta)).await.unwrap();
    }
    let view = sources(&config, &MemoryIdentity::root()).await.unwrap();
    let counts: Vec<(&str, u64)> = view
        .sources
        .iter()
        .map(|s| (s.source.as_str(), s.documents))
        .collect();
    assert_eq!(counts, [("github", 2), ("notion", 1)]);
    assert_eq!(view.unfiled, 0);

    let gone = forget(
        &config,
        &MemoryIdentity::root(),
        BrainForgetParams {
            source: "github".into(),
        },
    )
    .await
    .unwrap();
    assert_eq!(gone.forgotten, 2, "the source and its collection");
}

#[test]
fn filing_moves_an_item_to_its_source_node_without_an_agent() {
    let team = MemoryLayout::new("team:acme".parse().unwrap()).unwrap();
    let filed = file_into(team.brain(&BrainSource::Pdf).unwrap(), document(None, None));
    assert_eq!(filed.meta().namespace.to_string(), "team:acme/source:pdf");
    assert_eq!(filed.meta().agent_id, None);
}

#[tokio::test]
async fn text_is_ingested_searched_counted_and_forgotten_per_source() {
    let tmp = tempfile::tempdir().unwrap();
    let config = config_in(&tmp);
    let engine = bind_reference(&config);

    let ingested = ingest(
        &config,
        &MemoryIdentity::root(),
        BrainIngestParams {
            path: None,
            text: Some("Refunds are issued within 14 days.".into()),
            source: Some("notion".into()),
            title: Some("Refund policy".into()),
        },
    )
    .await
    .unwrap();
    assert_eq!(ingested.source, "notion");
    ingest(
        &config,
        &MemoryIdentity::root(),
        BrainIngestParams {
            path: None,
            text: Some("Onboarding takes a week.".into()),
            source: None,
            title: None,
        },
    )
    .await
    .unwrap();

    let docs = stored(&engine, MetaFilter::kinds([ItemKind::Document])).await;
    assert_eq!(docs.len(), 2);
    assert_eq!(
        jobs::snapshot(&config).await.pending.len(),
        2,
        "one belief build per source"
    );

    let counts = sources(&config, &MemoryIdentity::root()).await.unwrap();
    assert_eq!(counts.root, "root");
    assert_eq!(counts.sources.len(), 2);
    assert_eq!(counts.unfiled, 0);

    let found = search(
        &config,
        &MemoryIdentity::root(),
        BrainSearchParams {
            query: "refunds".into(),
            source: Some("notion".into()),
            limit: None,
        },
    )
    .await
    .unwrap();
    assert_eq!(found.hits.len(), 1);

    let gone = forget(
        &config,
        &MemoryIdentity::root(),
        BrainForgetParams {
            source: "notion".into(),
        },
    )
    .await
    .unwrap();
    assert_eq!(gone.forgotten, 1);
    assert_eq!(
        stored(&engine, MetaFilter::kinds([ItemKind::Document]))
            .await
            .len(),
        1
    );
}

#[tokio::test]
async fn ingest_refuses_bad_input() {
    let tmp = tempfile::tempdir().unwrap();
    let config = config_in(&tmp);
    bind_reference(&config);
    for params in [
        BrainIngestParams {
            path: None,
            text: None,
            source: None,
            title: None,
        },
        BrainIngestParams {
            path: Some("/nope/missing.md".into()),
            text: None,
            source: None,
            title: None,
        },
        BrainIngestParams {
            path: None,
            text: Some("x".into()),
            source: Some(" ".into()),
            title: None,
        },
    ] {
        assert!(matches!(
            ingest(&config, &MemoryIdentity::root(), params).await,
            Err(MemoryError::InvalidRequest(_))
        ));
    }
    assert!(search(
        &config,
        &MemoryIdentity::root(),
        BrainSearchParams {
            query: " ".into(),
            source: None,
            limit: None
        }
    )
    .await
    .is_err());
}

#[tokio::test]
async fn a_file_is_converted_and_filed_under_files() {
    let tmp = tempfile::tempdir().unwrap();
    let config = config_in(&tmp);
    let engine = bind_reference(&config);
    let path = tmp.path().join("guide.md");
    std::fs::write(&path, "# Guide\n\nAlways tag releases.").unwrap();
    let ingested = ingest(
        &config,
        &MemoryIdentity::root(),
        BrainIngestParams {
            path: Some(path.display().to_string()),
            text: None,
            source: None,
            title: None,
        },
    )
    .await
    .unwrap();
    assert_eq!(ingested.source, "files");
    let docs = stored(&engine, MetaFilter::kinds([ItemKind::Document])).await;
    assert_eq!(docs[0].meta.namespace.to_string(), "source:files");
}

#[tokio::test]
async fn an_ingest_queues_a_belief_build_only_for_an_engine_that_waits_for_one() {
    use std::sync::Arc;
    use tinymemory_api::conformance::ReferenceEngine;
    use tinymemory_api::Consolidation;

    for (consolidation, queued) in [(Consolidation::OnDemand, 1), (Consolidation::Automatic, 0)] {
        let tmp = tempfile::tempdir().unwrap();
        let config = config_in(&tmp);
        crate::memory::engine::install_test_engine(
            &config.workspace_dir,
            Arc::new(ReferenceEngine::new().with_consolidation(consolidation)),
        );
        ingest(
            &config,
            &MemoryIdentity::root(),
            BrainIngestParams {
                path: None,
                text: Some("Refunds are issued within 14 days.".into()),
                source: Some("notion".into()),
                title: None,
            },
        )
        .await
        .unwrap();
        let pending = crate::memory::lifecycle::jobs::snapshot(&config)
            .await
            .pending
            .len();
        assert_eq!(pending, queued, "{consolidation:?}");
    }
}

fn path_params(path: String) -> BrainIngestParams {
    BrainIngestParams {
        path: Some(path),
        text: None,
        source: None,
        title: None,
    }
}

#[tokio::test]
async fn ingest_refuses_a_credential_store_a_system_root_traversal_and_null_bytes() {
    let tmp = tempfile::tempdir().unwrap();
    let config = config_in(&tmp);
    let engine = bind_reference(&config);
    let ssh = tmp.path().join(".ssh");
    std::fs::create_dir_all(&ssh).unwrap();
    std::fs::write(ssh.join("id_ed25519"), "fixture: not a real key").unwrap();
    let notes = tmp.path().join("notes");
    std::fs::create_dir_all(&notes).unwrap();
    std::fs::write(tmp.path().join("guide.md"), "# Guide").unwrap();

    for path in [
        ssh.join("id_ed25519").display().to_string(),
        "/etc/hosts".to_string(),
        format!("{}/../guide.md", notes.display()),
        format!("{}\0.md", tmp.path().join("guide").display()),
    ] {
        let result = ingest(&config, &MemoryIdentity::root(), path_params(path.clone())).await;
        assert!(
            matches!(result, Err(MemoryError::InvalidRequest(_))),
            "{path:?}: {result:?}"
        );
    }
    assert!(engine.is_empty(), "nothing refused may be stored");
}

#[cfg(unix)]
#[tokio::test]
async fn ingest_refuses_a_symlink_into_a_credential_store() {
    let tmp = tempfile::tempdir().unwrap();
    let config = config_in(&tmp);
    let engine = bind_reference(&config);
    let aws = tmp.path().join(".aws");
    std::fs::create_dir_all(&aws).unwrap();
    std::fs::write(aws.join("credentials"), "aws_secret_access_key = x").unwrap();
    let link = tmp.path().join("innocent.md");
    std::os::unix::fs::symlink(aws.join("credentials"), &link).unwrap();

    let result = ingest(
        &config,
        &MemoryIdentity::root(),
        path_params(link.display().to_string()),
    )
    .await;
    assert!(
        matches!(result, Err(MemoryError::InvalidRequest(_))),
        "{result:?}"
    );
    assert!(engine.is_empty());
}
