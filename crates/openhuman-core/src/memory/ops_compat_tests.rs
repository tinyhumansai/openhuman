use super::*;

#[tokio::test]
async fn forget_with_no_id_is_invalid_even_when_memory_is_off() {
    let tmp = tempfile::tempdir().unwrap();
    let config = config_in(&tmp);
    let error = forget(
        &config,
        ForgetParams {
            ids: vec!["  ".into()],
            reach: None,
        },
    )
    .await
    .unwrap_err();
    assert_eq!(error.code(), "INVALID_REQUEST");
}

#[tokio::test]
async fn single_user_reads_writes_and_forgets_ancestor_and_service_memory() {
    let tmp = tempfile::tempdir().unwrap();
    let config = config_in(&tmp);
    let engine = bind_reference(&config);
    let ancestor_id = store_item(
        &config,
        StoreItem::learning(
            "shared ancestor memory",
            tinymemory_api::LearningKind::Fact,
            0.9,
            MemoryMeta::default(),
        ),
    )
    .await
    .unwrap()
    .id
    .0;
    let service_namespace: tinymemory_api::Namespace =
        "ws:main/service:newsletter".parse().unwrap();
    let service_id = store_item(
        &config,
        StoreItem::learning(
            "service private memory",
            tinymemory_api::LearningKind::Fact,
            0.9,
            MemoryMeta {
                namespace: service_namespace.clone(),
                ..MemoryMeta::default()
            },
        ),
    )
    .await
    .unwrap()
    .id
    .0;

    let inherited = items_list(
        &config,
        ItemsListParams {
            filter: Some(MetaFilter {
                reach: Some(tinymemory_api::Reach::of(
                    "ws:main/agent:writer".parse().unwrap(),
                )),
                ..MetaFilter::default()
            }),
            ..ItemsListParams::default()
        },
    )
    .await
    .unwrap();
    assert!(inherited.items.iter().any(|item| item.id.0 == ancestor_id));

    let service_reader = items_list(
        &config,
        ItemsListParams {
            filter: Some(MetaFilter {
                reach: Some(tinymemory_api::Reach::of(
                    "ws:main/service:newsletter/agent:writer".parse().unwrap(),
                )),
                ..MetaFilter::default()
            }),
            ..ItemsListParams::default()
        },
    )
    .await
    .unwrap();
    assert!(service_reader
        .items
        .iter()
        .any(|item| item.id.0 == service_id));
    let service_item = stored(
        &engine,
        MetaFilter {
            reach: Some(tinymemory_api::Reach::exact(service_namespace.clone())),
            ..MetaFilter::default()
        },
    )
    .await
    .into_iter()
    .find(|item| item.id.0 == service_id)
    .expect("the explicit service reach reads the stored item");
    assert_eq!(service_item.meta.namespace, service_namespace);

    let forgotten = forget(
        &config,
        ForgetParams {
            ids: vec![service_id.clone()],
            reach: None,
        },
    )
    .await
    .unwrap();
    assert_eq!(forgotten.forgotten, 1);
    assert!(stored(
        &engine,
        MetaFilter {
            reach: Some(tinymemory_api::Reach::exact(service_item.meta.namespace)),
            ..MetaFilter::default()
        },
    )
    .await
    .iter()
    .all(|item| item.id.0 != service_id));
}
