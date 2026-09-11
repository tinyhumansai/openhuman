use super::*;
use crate::openhuman::config::Config;

#[test]
fn linux_service_file_uses_config_dir() {
    let config = Config::default();
    let path = linux_service_file(&config).unwrap();
    assert!(path.ends_with(".config/systemd/user/openhuman.service"));
}
