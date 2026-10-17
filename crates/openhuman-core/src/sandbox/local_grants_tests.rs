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

#[cfg(feature = "modules")]
mod with_module {
    use super::*;
    use crate::modules::tinybox::contract::LocalGrants;

    #[test]
    fn a_forbidden_path_returned_by_the_module_is_refused() {
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
        let grants = admit_module_grants(reply, false);
        assert_eq!(grants.read_only, [PathBuf::from("/home/op/.rustup")]);
        assert_eq!(
            grants.read_write,
            [PathBuf::from("/home/op/.cargo/registry")]
        );
    }

    #[test]
    fn proc_is_admitted_only_through_allow_proc() {
        let reply = || LocalGrants {
            read_only: vec!["/proc".into(), "/proc/1/environ".into()],
            read_write: Vec::new(),
        };
        assert!(admit_module_grants(reply(), false).read_only.is_empty());
        assert_eq!(
            admit_module_grants(reply(), true).read_only,
            [PathBuf::from("/proc")]
        );
    }

    #[test]
    fn request_carries_config_and_the_host_floor() {
        let jail = LocalJailConfig {
            toolchain_homes: false,
            extra_read_only: vec!["~/a".into()],
            extra_read_write: vec!["~/b".into()],
            allow_proc: true,
        };
        let request = grant_request(Some(std::path::Path::new("/home/op")), &jail);
        assert_eq!(request.home.as_deref(), Some("/home/op"));
        assert!(!request.config.toolchain_homes && request.config.allow_proc);
        assert_eq!(request.config.extra_read_only, ["~/a"]);
        assert_eq!(request.config.extra_read_write, ["~/b"]);
        assert!(request.protected.components.contains(&".ssh".to_owned()));
        assert!(request.protected.components.contains(&".netrc".to_owned()));
        assert!(request
            .protected
            .component_pairs
            .contains(&[".cargo".to_owned(), "credentials.toml".to_owned()]));
        assert!(request.protected.prefixes.contains(&"/etc".to_owned()));
        assert!(request
            .protected
            .credential_roots
            .contains(&"/home/op/.ssh".to_owned()));
    }

    #[test]
    fn no_home_means_no_credential_roots() {
        let request = grant_request(None, &LocalJailConfig::default());
        assert!(request.home.is_none() && request.protected.credential_roots.is_empty());
    }
}
