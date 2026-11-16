use super::*;

#[test]
fn linux_service_file_uses_the_work_dir() {
    let spec = ServiceSpec {
        label: "openhuman.service".to_string(),
        legacy_labels: Vec::new(),
        exe: PathBuf::from("/usr/bin/openhuman-core"),
        args: vec!["run".to_string()],
        logs_dir: PathBuf::from("/home/u/.openhuman/logs"),
        work_dir: PathBuf::from("/home/u/.openhuman"),
    };
    let path = linux_service_file(&spec);
    assert_eq!(
        path,
        PathBuf::from("/home/u/.openhuman/.config/systemd/user/openhuman.service")
    );
}
