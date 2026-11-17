//! Reload a live config context after a document write conflict.

use super::super::Config;

pub(super) fn is_document_conflict(error: &anyhow::Error) -> bool {
    error.chain().any(|cause| {
        cause
            .downcast_ref::<tinystoragedrivers::StorageError>()
            .is_some_and(|error| error.kind() == tinystoragedrivers::ErrorKind::Conflict)
    })
}

impl Config {
    pub(crate) fn bind_context_snapshot(snapshot: &std::sync::Arc<std::sync::Mutex<Config>>) {
        snapshot.lock().unwrap().runtime_state.context_snapshot =
            Some(std::sync::Arc::downgrade(snapshot));
    }

    pub(super) fn refresh_context_after_conflict<'a>(
        &'a self,
        expected: &'a super::super::ConfigDocumentSnapshot,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + 'a>> {
        Box::pin(async move {
            let Some(snapshot) = self
                .runtime_state
                .context_snapshot
                .as_ref()
                .and_then(std::sync::Weak::upgrade)
            else {
                return;
            };
            let (config_path, workspace_dir, runtime) = {
                let current = snapshot.lock().unwrap();
                if *current.runtime_state.document_snapshot.lock().unwrap() != *expected {
                    return;
                }
                (
                    current.config_path.clone(),
                    current.workspace_dir.clone(),
                    current.clone(),
                )
            };
            let Ok(mut refreshed) = Self::load_from_config_path(&config_path, &workspace_dir).await
            else {
                return;
            };
            if refreshed
                .runtime_state
                .document_snapshot
                .lock()
                .unwrap()
                .anchor
                != expected.anchor
            {
                return;
            }
            let overlay = runtime
                .runtime_state
                .context_overlay
                .lock()
                .unwrap()
                .clone();
            if let Some(overlay) = overlay {
                let Ok(merged) = overlay.reapply(&refreshed) else {
                    return;
                };
                refreshed = merged;
            } else {
                super::context_overlay::copy_runtime_bindings(&runtime, &mut refreshed);
            }
            refreshed.runtime_state.context_snapshot = Some(std::sync::Arc::downgrade(&snapshot));

            let mut current = snapshot.lock().unwrap();
            if *current.runtime_state.document_snapshot.lock().unwrap() == *expected {
                *current = refreshed;
            }
        })
    }
}
