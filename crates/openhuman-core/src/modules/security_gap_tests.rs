use super::stub::{Asked, StubPort};
use super::*;
use tinysecurity_bus::PathCheckKind;

fn scope() -> PathPolicy {
    PathPolicy {
        enabled: false,
        workspace_only: false,
        workspace_dir: "/w".into(),
        action_dir: "/a".into(),
        home_dir: None,
        forbidden_paths: vec![],
        trusted_roots: vec![],
        reserved_paths: vec![],
        reserved_prefixes: vec![],
        readonly_paths: vec![],
        reserved_names: vec![],
    }
}

#[tokio::test]
async fn a_walk_larger_than_one_batch_is_split_and_keeps_order() {
    let stub = StubPort::allowing()
        .paths(|_, _, path| !path.ends_with("7"))
        .shared();
    let _guard = install_port(stub.clone());
    let paths: Vec<PathBuf> = (0..MAX_PATH_BATCH + 3)
        .map(|n| PathBuf::from(format!("/a/{n}")))
        .collect();
    let answers = check_paths(scope(), PathCheckKind::StringAllowed, &paths)
        .await
        .unwrap();
    assert_eq!(answers.len(), paths.len());
    assert!(!answers[7]);
    assert!(answers[8]);
    let asked = stub.asked();
    assert_eq!(asked.len(), 2, "one call per batch, not per path");
}

#[tokio::test]
async fn an_unavailable_module_protects_every_path() {
    let _guard = install_port(StubPort::unavailable().shared());
    assert!(is_protected(Path::new("/home/u/project")).await);
    assert!(any_protected(&[Path::new("/a"), Path::new("/b")]).await);
    assert!(is_protected_blocking(Path::new("/home/u/project")));
    assert!(
        check_paths(scope(), PathCheckKind::StringAllowed, &["/a".into()])
            .await
            .is_err()
    );
}

#[tokio::test]
async fn a_short_answer_is_a_refusal_not_a_partial_allow() {
    #[derive(Debug)]
    struct Short;
    #[async_trait]
    impl GapPort for Short {
        async fn check_paths(
            &self,
            _: PathPolicy,
            _: PathCheckKind,
            _: Vec<PathBuf>,
        ) -> Result<Vec<bool>, String> {
            Ok(vec![true])
        }
        async fn protected_paths(&self, _: Vec<PathBuf>) -> Result<Vec<bool>, String> {
            Ok(vec![false])
        }
        async fn protected_floor(&self) -> Result<ProtectedFloor2, String> {
            Err("n/a".into())
        }
        async fn budget(
            &self,
            _: PathBuf,
            _: CommandPolicy,
            _: BudgetOp,
        ) -> Result<BudgetDecision2, String> {
            Err("n/a".into())
        }
        async fn privacy(&self, _: PrivacyCheckRequest2) -> Result<Decision, String> {
            Err("n/a".into())
        }
        async fn host_action_class(
            &self,
            _: HostActionClassRequest2,
        ) -> Result<CommandClass, String> {
            Err("n/a".into())
        }
    }
    let _guard = install_port(Arc::new(Short));
    let two: Vec<PathBuf> = vec!["/a".into(), "/b".into()];
    assert!(check_paths(scope(), PathCheckKind::StringAllowed, &two)
        .await
        .is_err());
    assert!(any_protected(&[Path::new("/a"), Path::new("/b")]).await);
}

#[tokio::test]
async fn the_port_is_restored_when_the_guard_drops() {
    {
        let stub = StubPort::allowing().shared();
        let _guard = install_port(stub.clone());
        assert!(!is_protected(Path::new("/a")).await);
        assert_eq!(stub.asked(), vec![Asked::Protected(vec!["/a".into()])]);
    }
    assert!(overridden().is_none());
}
