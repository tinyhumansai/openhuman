//! Existing scoped cleanup and callback reservations.
use super::*;

impl McpConfiguredClient {
    /// Close an already-open agent-scoped client without creating one. A
    /// failed close remains in the cache, marked unavailable, so shutdown or
    /// a later cleanup attempt can retry the module's retained transition.
    pub(crate) async fn close_agent_data_dir(data_dir: &std::path::Path) -> bool {
        let key = data_dir.to_path_buf();
        let reservation = Self::reservation_for(&key);
        let Some(client) = Self::retire_cached_client(&key) else {
            return Self::finish_cleanup(&key, None, reservation);
        };
        if let Some(cleaned) = Self::finish_closed_client_synchronously(&key, &client, &reservation)
        {
            return cleaned;
        }
        Self::close_retired_client(key, client, reservation).await
    }

    /// Marks the exact cached client unavailable before scheduling cleanup.
    /// If no Tokio runtime is active, the retired client remains cached for the
    /// registered shutdown hook to close later.
    pub(crate) fn schedule_agent_close_data_dir(data_dir: &std::path::Path) {
        let key = data_dir.to_path_buf();
        let reservation = Self::reservation_for(&key);
        let Some(client) = Self::retire_cached_client(&key) else {
            Self::finish_cleanup(&key, None, reservation);
            return;
        };
        if Self::finish_closed_client_synchronously(&key, &client, &reservation).is_some() {
            return;
        }
        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            return;
        };
        runtime.spawn(async move {
            if !Self::close_retired_client(key, client, reservation).await {
                tracing::warn!("[mcp] agent client close remains pending for retry");
            }
        });
    }

    /// Retires the exact scoped client and runs `after_close` only after the
    /// module acknowledges its close. The path remains unavailable until the
    /// callback succeeds, so a replacement cannot reuse files being purged.
    pub(crate) fn schedule_agent_close_and_cleanup(
        data_dir: &std::path::Path,
        after_close: CleanupAction,
    ) {
        let key = data_dir.to_path_buf();
        let reservation = Self::reservation_with_action(&key, after_close);
        let Some(client) = Self::retire_cached_client(&key) else {
            if !Self::finish_cleanup(&key, None, reservation) {
                tracing::warn!("[mcp] agent cleanup remains pending for retry");
            }
            return;
        };
        if Self::finish_closed_client_synchronously(&key, &client, &reservation).is_some() {
            return;
        }
        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            return;
        };
        runtime.spawn(async move {
            if !Self::close_retired_client(key, client, reservation).await {
                tracing::warn!("[mcp] agent close and cleanup remain pending for retry");
            }
        });
    }

    pub(super) fn retry_orphaned_cleanups() {
        let reservations = CLEANUP_RESERVATIONS
            .get()
            .map(|reservations| {
                reservations
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .iter()
                    .map(|(path, reservation)| (path.clone(), Arc::clone(reservation)))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        for (data_dir, reservation) in reservations {
            let has_client = CLIENTS.get().is_some_and(|clients| {
                clients
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .contains_key(&data_dir)
            });
            if !has_client {
                Self::finish_cleanup(&data_dir, None, reservation);
            }
        }
    }

    pub(crate) fn agent_close_pending(data_dir: &std::path::Path) -> bool {
        Self::cleanup_pending(data_dir)
            || CLIENTS
                .get()
                .and_then(|clients| {
                    clients
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .get(data_dir)
                        .map(|client| client.retiring.load(Ordering::Acquire))
                })
                .unwrap_or(false)
    }

    pub(super) fn cleanup_pending(data_dir: &std::path::Path) -> bool {
        CLEANUP_RESERVATIONS.get().is_some_and(|reservations| {
            reservations
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .contains_key(data_dir)
        })
    }

    pub(super) fn reservation_for(data_dir: &std::path::Path) -> Arc<CleanupReservation> {
        let reservations = CLEANUP_RESERVATIONS.get_or_init(|| Mutex::new(HashMap::new()));
        let mut reservations = reservations
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        Arc::clone(
            reservations
                .entry(data_dir.to_path_buf())
                .or_insert_with(|| Arc::new(CleanupReservation::default())),
        )
    }

    pub(super) fn reservation_with_action(
        data_dir: &std::path::Path,
        action: CleanupAction,
    ) -> Arc<CleanupReservation> {
        loop {
            let reservation = Self::reservation_for(data_dir);
            let mut after_close = reservation
                .after_close
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let current = CLEANUP_RESERVATIONS
                .get()
                .and_then(|reservations| {
                    reservations
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .get(data_dir)
                        .is_some_and(|stored| Arc::ptr_eq(stored, &reservation))
                        .then_some(())
                })
                .is_some();
            if !current {
                continue;
            }
            after_close.get_or_insert(action);
            return Arc::clone(&reservation);
        }
    }

    pub(super) fn retire_cached_client(data_dir: &std::path::Path) -> Option<Arc<Self>> {
        let clients = CLIENTS.get().map(|clients| {
            clients
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
        })?;
        let client = clients.get(data_dir)?;
        client.retiring.store(true, Ordering::Release);
        Some(Arc::clone(client))
    }

    pub(super) fn finish_closed_client_synchronously(
        data_dir: &std::path::Path,
        client: &Arc<Self>,
        reservation: &Arc<CleanupReservation>,
    ) -> Option<bool> {
        let Ok(state) = client.state.try_lock() else {
            return None;
        };
        let is_closed = matches!(&*state, State::Closed);
        drop(state);
        is_closed.then(|| {
            Self::finish_cleanup(data_dir, Some(Arc::clone(client)), Arc::clone(reservation))
        })
    }

    pub(super) async fn close_retired_client(
        data_dir: PathBuf,
        client: Arc<Self>,
        reservation: Arc<CleanupReservation>,
    ) -> bool {
        match client.close().await {
            Ok(()) => Self::finish_cleanup(&data_dir, Some(client), reservation),
            Err(error) => {
                tracing::warn!(?error, "[mcp] agent client close remains pending");
                false
            }
        }
    }

    pub(super) fn finish_cleanup(
        data_dir: &std::path::Path,
        client: Option<Arc<Self>>,
        reservation: Arc<CleanupReservation>,
    ) -> bool {
        let after_close = reservation
            .after_close
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let reservations = CLEANUP_RESERVATIONS.get_or_init(|| Mutex::new(HashMap::new()));
        let is_current = reservations
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(data_dir)
            .is_some_and(|stored| Arc::ptr_eq(stored, &reservation));
        if !is_current || reservation.host_completion_pending.load(Ordering::Acquire) {
            return false;
        }
        let mut clients = if let Some(client) = client.as_ref() {
            let Some(clients) = CLIENTS.get() else {
                return false;
            };
            let clients = clients
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if !clients
                .get(data_dir)
                .is_some_and(|current| Arc::ptr_eq(current, client))
            {
                return false;
            }
            Some(clients)
        } else {
            None
        };
        if after_close.as_ref().is_some_and(|action| !action()) {
            return false;
        }
        if let Some(clients) = clients.as_mut() {
            clients.remove(data_dir);
        }
        let mut reservations = reservations
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if reservation.host_completion_pending.load(Ordering::Acquire) {
            return false;
        }
        if reservations
            .get(data_dir)
            .is_some_and(|stored| Arc::ptr_eq(stored, &reservation))
        {
            reservations.remove(data_dir);
            true
        } else {
            false
        }
    }
}
