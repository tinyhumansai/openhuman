use super::*;

/// A port that models TinyMCP's module-wide lifecycle gate: a second
/// `open_configured` while another workspace's transition is still pending is
/// refused instead of queued.
#[derive(Clone, Default)]
struct SingleTransitionPort {
    pending: Arc<std::sync::atomic::AtomicBool>,
}

#[async_trait]
impl McpModulePort for SingleTransitionPort {
    async fn open_configured(
        &self,
        data_dir: &str,
        _config: &McpClientConfig,
    ) -> Result<OpenConfiguredResponse, ModuleCallError> {
        if self.pending.swap(true, std::sync::atomic::Ordering::SeqCst) {
            return Err(ModuleCallError::ModuleFault);
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
        Ok(OpenConfiguredResponse {
            object_path: format!("/workspace{data_dir}"),
            transition_id: "open".to_owned(),
        })
    }

    async fn close(&self, _data_dir: &str) -> Result<Option<String>, ModuleCallError> {
        Ok(None)
    }

    async fn transition_status(
        &self,
        _transition_id: &str,
    ) -> Result<TransitionStatus, ModuleCallError> {
        tokio::time::sleep(Duration::from_millis(5)).await;
        self.pending
            .store(false, std::sync::atomic::Ordering::SeqCst);
        Ok(TransitionStatus::Complete)
    }

    async fn call_object(
        &self,
        _object_path: &str,
        _member: &str,
        _args: Value,
        _confidential: bool,
    ) -> Result<Value, ModuleCallError> {
        Ok(Value::Bool(true))
    }

    async fn prepare_agent_tools(
        &self,
        _request: PrepareAgentToolsRequest,
    ) -> Result<Vec<McpToolPresentation>, ModuleCallError> {
        Ok(Vec::new())
    }

    async fn validate_server_arguments(
        &self,
        _request: ValidateServerArgumentsRequest,
    ) -> Result<ServerArgumentsValidation, ModuleCallError> {
        Err(ModuleCallError::IncompatibleContract)
    }
}

#[tokio::test]
async fn concurrent_workspaces_open_one_at_a_time_through_the_module_lifecycle_gate() {
    let port = SingleTransitionPort::default();
    let first = McpConfiguredClient::with_port("/private/a".to_owned(), Box::new(port.clone()));
    let second = McpConfiguredClient::with_port("/private/b".to_owned(), Box::new(port));
    let config = configured("server");

    let (a, b) = tokio::join!(
        first.call::<bool>(&config, "ConnectedOverview", ()),
        second.call::<bool>(&config, "ConnectedOverview", ()),
    );

    assert!(a.expect("first workspace opens"));
    assert!(b.expect("second workspace opens"));
}
