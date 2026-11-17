use std::path::PathBuf;

use super::types::{DurableSubagentSession, SubagentSessionStore};

impl SubagentSessionStore {
    pub fn new(workspace_dir: PathBuf) -> Self {
        Self {
            workspace_dir,
            context: None,
        }
    }

    /// Opens the session store under the dispatch's resolved storage scope.
    /// Keep the context on the handle because these synchronous compatibility
    /// operations can run after an async task-local scope has been lost.
    pub fn new_in_context(
        ctx: &crate::core::runtime::DispatchCtx,
        workspace_dir: PathBuf,
    ) -> Result<Self, String> {
        let context = ctx
            .resolved()
            .ok_or_else(|| "subagent session store has no resolved owner scope".to_owned())?;
        Ok(Self {
            workspace_dir,
            context: Some(context),
        })
    }

    fn in_scope<T>(&self, f: impl FnOnce() -> T) -> T {
        match &self.context {
            Some(context) => crate::core::runtime::CoreContext::sync_scope(Arc::clone(context), f),
            None => f(),
        }
    }

    /// The pre-documents `<workspace>/.openhuman/subagent_sessions.json`.
    fn legacy_path(&self) -> PathBuf {
        self.workspace_dir
            .join(".openhuman")
            .join("subagent_sessions.json")
    }

    /// Imports the pre-documents session file once: its sessions are written
    /// (an already stored newer copy wins) and a marker document records that
    /// the import finished. The file is only read, never renamed or rewritten,
    /// so it stays byte-identical. A missing file is the common case and a
    /// no-op.
    fn import_legacy(&self, docs: &super::store_documents::Docs) -> Result<(), String> {
        if docs
            .legacy_imported(&self.workspace_dir)
            .map_err(|err| format!("{err:#}"))?
        {
            return Ok(());
        }
        let path = self.legacy_path();
        let raw = match std::fs::read_to_string(&path) {
            Ok(raw) => raw,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(err) => return Err(format!("failed to read legacy subagent sessions: {err}")),
        };
        let sessions: Vec<DurableSubagentSession> = serde_json::from_str(&raw)
            .map_err(|err| format!("failed to parse legacy subagent sessions: {err}"))?;
        docs.import_legacy(&self.workspace_dir, &sessions)
            .map_err(|err| format!("{err:#}"))?;
        log::debug!(
            "[subagent_sessions] imported legacy sessions count={}",
            sessions.len()
        );
        Ok(())
    }

    pub fn load(&self) -> Result<Vec<DurableSubagentSession>, String> {
        self.in_scope(|| {
            let docs = super::store_documents::current(&self.workspace_dir)
                .map_err(|err| format!("{err:#}"))?;
            self.import_legacy(&docs)?;
            docs.load().map_err(|err| format!("{err:#}"))
        })
    }

    /// Finds one row for the TinyAgents migration adapter. Unlike `load`, this
    /// performs an indexed read after any one-time legacy-file import and does
    /// not rewrite the host session row.
    pub(super) fn find_task_for_import(
        &self,
        task_id: &str,
    ) -> Result<Option<DurableSubagentSession>, String> {
        self.in_scope(|| {
            let docs = super::store_documents::current(&self.workspace_dir)
                .map_err(|err| format!("{err:#}"))?;
            self.import_legacy(&docs)?;
            docs.find_task(task_id).map_err(|err| format!("{err:#}"))
        })
    }

    pub fn save(&self, sessions: &[DurableSubagentSession]) -> Result<(), String> {
        self.in_scope(|| {
            let docs = super::store_documents::current(&self.workspace_dir)
                .map_err(|err| format!("{err:#}"))?;
            self.import_legacy(&docs)?;
            docs.save(sessions).map_err(|err| format!("{err:#}"))
        })
    }

    pub fn update_session<F>(
        &self,
        subagent_session_id: &str,
        update: F,
    ) -> Result<Option<DurableSubagentSession>, String>
    where
        F: FnMut(&mut DurableSubagentSession, String) + Send + 'static,
    {
        self.in_scope(|| {
            let docs = super::store_documents::current(&self.workspace_dir)
                .map_err(|err| format!("{err:#}"))?;
            self.import_legacy(&docs)?;
            docs.update_session(subagent_session_id, update)
                .map_err(|err| format!("{err:#}"))
        })
    }

    pub fn update_session_if<F>(
        &self,
        subagent_session_id: &str,
        update: F,
    ) -> Result<Option<DurableSubagentSession>, String>
    where
        F: FnMut(&mut DurableSubagentSession, String) -> bool + Send + 'static,
    {
        self.in_scope(|| {
            let docs = super::store_documents::current(&self.workspace_dir)
                .map_err(|err| format!("{err:#}"))?;
            self.import_legacy(&docs)?;
            docs.update_session_if(subagent_session_id, update)
                .map_err(|err| format!("{err:#}"))
        })
    }
}

use std::sync::Arc;
