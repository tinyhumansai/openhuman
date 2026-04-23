use super::{
    default_core_port, default_core_run_mode, same_executable_path, CoreProcessHandle,
    CoreRunMode,
};

struct EnvGuard {
    key: &'static str,
    old: Option<String>,
}

impl EnvGuard {
    fn set(key: &'static str, value: &str) -> Self {
        let old = std::env::var(key).ok();
        std::env::set_var(key, value);
        Self { key, old }
    }

    fn unset(key: &'static str) -> Self {
        let old = std::env::var(key).ok();
        std::env::remove_var(key);
        Self { key, old }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        if let Some(old) = &self.old {
            std::env::set_var(self.key, old);
        } else {
            std::env::remove_var(self.key);
        }
    }
}

#[test]
fn default_core_run_mode_env_parsing() {
    let _unset = EnvGuard::unset("OPENHUMAN_CORE_RUN_MODE");
    assert_eq!(default_core_run_mode(false), CoreRunMode::ChildProcess);

    let _guard = EnvGuard::set("OPENHUMAN_CORE_RUN_MODE", "in-process");
    assert_eq!(default_core_run_mode(false), CoreRunMode::InProcess);

    let _guard = EnvGuard::set("OPENHUMAN_CORE_RUN_MODE", "sidecar");
    assert_eq!(default_core_run_mode(false), CoreRunMode::ChildProcess);
}

#[test]
fn default_core_port_env_and_fallback() {
    let _unset = EnvGuard::unset("OPENHUMAN_CORE_PORT");
    assert_eq!(default_core_port(), 7788);

    let _set = EnvGuard::set("OPENHUMAN_CORE_PORT", "8899");
    assert_eq!(default_core_port(), 8899);
}

#[test]
fn same_executable_path_handles_equal_and_non_equal_paths() {
    let current = std::env::current_exe().expect("current exe");
    assert!(same_executable_path(&current, &current));

    let different = current.with_file_name("definitely-not-the-current-exe");
    assert!(!same_executable_path(&current, &different));
}

#[test]
fn ensure_running_returns_ok_when_rpc_port_already_open() {
    let rt = tokio::runtime::Runtime::new().expect("runtime");
    let result = rt.block_on(async {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind test listener");
        let port = listener.local_addr().expect("local addr").port();
        let handle = CoreProcessHandle::new(port, None, CoreRunMode::ChildProcess);
        handle.ensure_running().await
    });
    assert!(
        result.is_ok(),
        "ensure_running should fast-path: {result:?}"
    );
}
