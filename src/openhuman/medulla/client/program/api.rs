//! Operator-owned program task ledger and its GitHub task sources
//! (`/medulla/v1/tasks`).
//!
//! Split from the client transport core; the shared request helpers live in
//! [`super::super`].

use super::super::*;

impl MedullaClient {
    /// List the operator-owned program task ledger (`GET /medulla/v1/tasks`).
    pub async fn list_program_tasks(&self) -> Result<Vec<ProgramTask>> {
        let req = self.authed(self.http.get(self.url("/medulla/v1/tasks")));
        let payload: TasksPayload = self.send(req).await?;
        Ok(payload.tasks)
    }

    /// Create an operator-owned program task (`POST /medulla/v1/tasks`).
    pub async fn create_program_task(&self, input: CreateProgramTask) -> Result<ProgramTask> {
        let req = self
            .authed(self.http.post(self.url("/medulla/v1/tasks")))
            .json(&input);
        let payload: TaskPayload = self.send(req).await?;
        Ok(payload.task)
    }

    /// Update an operator-owned program task (`PATCH /medulla/v1/tasks/:id`).
    pub async fn update_program_task(
        &self,
        task_id: &str,
        patch: UpdateProgramTask,
    ) -> Result<ProgramTask> {
        let task_id = urlencode(task_id);
        let req = self
            .authed(
                self.http
                    .patch(self.url(&format!("/medulla/v1/tasks/{task_id}"))),
            )
            .json(&patch);
        let payload: TaskPayload = self.send(req).await?;
        Ok(payload.task)
    }

    /// Delete an operator-owned program task (`DELETE /medulla/v1/tasks/:id`).
    pub async fn delete_program_task(&self, task_id: &str) -> Result<bool> {
        let task_id = urlencode(task_id);
        let req = self.authed(
            self.http
                .delete(self.url(&format!("/medulla/v1/tasks/{task_id}"))),
        );
        let payload: DeleteProgramItem = self.send(req).await?;
        Ok(payload.deleted)
    }

    /// List configured GitHub task sources (`GET /medulla/v1/tasks/sources`).
    pub async fn list_program_task_sources(&self) -> Result<Vec<ProgramTaskSource>> {
        let req = self.authed(self.http.get(self.url("/medulla/v1/tasks/sources")));
        let payload: TaskSourcesPayload = self.send(req).await?;
        Ok(payload.sources)
    }

    /// Configure a GitHub task source (`POST /medulla/v1/tasks/sources`).
    ///
    /// `input.token` is write-only: the backend encrypts it and responses expose
    /// only [`ProgramTaskSource::has_token`].
    pub async fn create_program_task_source(
        &self,
        input: CreateProgramTaskSource,
    ) -> Result<ProgramTaskSource> {
        let req = self
            .authed(self.http.post(self.url("/medulla/v1/tasks/sources")))
            .json(&input);
        let payload: TaskSourcePayload = self.send(req).await?;
        Ok(payload.source)
    }

    /// Synchronize one GitHub source into the task ledger.
    pub async fn sync_program_task_source(&self, source_id: &str) -> Result<TaskSourceSyncResult> {
        let source_id = urlencode(source_id);
        let req = self.authed(
            self.http
                .post(self.url(&format!("/medulla/v1/tasks/sources/{source_id}/sync"))),
        );
        let payload: TaskSourceSyncPayload = self.send(req).await?;
        Ok(payload.result)
    }

    /// Remove a configured GitHub task source.
    pub async fn delete_program_task_source(&self, source_id: &str) -> Result<bool> {
        let source_id = urlencode(source_id);
        let req = self.authed(
            self.http
                .delete(self.url(&format!("/medulla/v1/tasks/sources/{source_id}"))),
        );
        let payload: DeleteProgramItem = self.send(req).await?;
        Ok(payload.deleted)
    }
}
