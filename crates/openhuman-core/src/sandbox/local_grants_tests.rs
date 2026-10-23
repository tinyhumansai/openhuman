use super::*;

#[test]
fn saas_never_consults_the_operator_home() {
    let home = grant_home(true, || {
        panic!("a tenant must not read the operator's home")
    });
    assert!(home.is_none());
}

#[test]
fn single_user_keeps_the_operator_home() {
    let home = grant_home(false, || Some(PathBuf::from("/home/op")));
    assert_eq!(home, Some(PathBuf::from("/home/op")));
}

#[cfg(feature = "security-module")]
mod with_module {
    use super::*;
    use crate::modules::security::gap::install_port;
    use crate::modules::security::gap::stub::StubPort;
    use crate::modules::tinybox::contract::LocalGrants;
    use std::sync::Arc;

    /// What the module's floor answers for these paths. The module owns the real
    /// match; this scripts the answers the host must act on.
    fn floor_port() -> Arc<StubPort> {
        Arc::new(StubPort::allowing().protecting(|path| {
            let text = path.to_string_lossy();
            [".ssh", ".aws", "credentials.toml", "/etc", "/proc/1"]
                .iter()
                .any(|needle| text.contains(needle))
        }))
    }

    #[tokio::test]
    async fn a_forbidden_path_returned_by_the_module_is_refused() {
        let stub = floor_port();
        let _port = install_port(stub.clone());
        let reply = LocalGrants {
            read_only: vec![
                "/home/op/.ssh".into(),
                "/home/op/.cargo/credentials.toml".into(),
                "/etc".into(),
                "/home/op/.rustup".into(),
            ],
            read_write: vec![
                "/home/op/.aws/sso".into(),
                "/home/op/.cargo/registry".into(),
            ],
        };
        let grants = admit_module_grants(reply, false).await;
        assert_eq!(grants.read_only, [PathBuf::from("/home/op/.rustup")]);
        assert_eq!(
            grants.read_write,
            [PathBuf::from("/home/op/.cargo/registry")]
        );
        assert_eq!(
            stub.asked().len(),
            1,
            "the whole reply is one floor question"
        );
    }

    #[tokio::test]
    async fn proc_is_admitted_only_through_allow_proc() {
        let _port = install_port(Arc::new(
            StubPort::allowing().protecting(|path| path.starts_with("/proc")),
        ));
        let reply = || LocalGrants {
            read_only: vec!["/proc".into(), "/proc/1/environ".into()],
            read_write: Vec::new(),
        };
        assert!(admit_module_grants(reply(), false)
            .await
            .read_only
            .is_empty());
        assert_eq!(
            admit_module_grants(reply(), true).await.read_only,
            [PathBuf::from("/proc")]
        );
    }

    #[tokio::test]
    async fn no_grant_is_admitted_when_the_floor_is_unavailable() {
        let _port = install_port(Arc::new(StubPort::unavailable()));
        let reply = LocalGrants {
            read_only: vec!["/home/op/.rustup".into()],
            read_write: vec!["/home/op/.cargo/registry".into()],
        };
        let grants = admit_module_grants(reply, true).await;
        assert!(grants.read_only.is_empty() && grants.read_write.is_empty());
    }

    #[tokio::test]
    async fn request_carries_config_and_the_floor_the_module_supplied() {
        let _port = install_port(Arc::new(StubPort::allowing()));
        let jail = LocalJailConfig {
            toolchain_homes: false,
            extra_read_only: vec!["~/a".into()],
            extra_read_write: vec!["~/b".into()],
            allow_proc: true,
        };
        let request = grant_request(Some(std::path::Path::new("/home/op")), &jail)
            .await
            .unwrap();
        assert_eq!(request.home.as_deref(), Some("/home/op"));
        assert!(!request.config.toolchain_homes && request.config.allow_proc);
        assert_eq!(request.config.extra_read_only, ["~/a"]);
        assert_eq!(request.config.extra_read_write, ["~/b"]);
        // The scripted floor: directory components and single-file stores travel
        // together as TinyBox's `components`.
        assert_eq!(request.protected.components, [".ssh", ".netrc"]);
        assert_eq!(
            request.protected.component_pairs,
            [[".docker".to_owned(), "config.json".to_owned()]]
        );
        assert_eq!(request.protected.prefixes, ["/etc"]);
        assert!(request
            .protected
            .credential_roots
            .contains(&"/home/op/.ssh".to_owned()));
    }

    #[tokio::test]
    async fn no_home_means_no_credential_roots() {
        let _port = install_port(Arc::new(StubPort::allowing()));
        let request = grant_request(None, &LocalJailConfig::default())
            .await
            .unwrap();
        assert!(request.home.is_none() && request.protected.credential_roots.is_empty());
    }

    #[tokio::test]
    async fn no_request_is_built_without_the_floor() {
        let _port = install_port(Arc::new(StubPort::unavailable()));
        assert!(grant_request(None, &LocalJailConfig::default())
            .await
            .is_err());
    }
}
