use super::*;

#[test]
fn xml_escape_replaces_entities() {
    let raw = "<tag>\"&'";
    let escaped = xml_escape(raw);
    assert!(escaped.contains("&lt;tag&gt;"));
    assert!(escaped.contains("&quot;"));
    assert!(escaped.contains("&amp;"));
    assert!(escaped.contains("&apos;"));
}

#[test]
fn systemd_command_line_keeps_a_plain_service_line_unchanged() {
    let exe = Path::new("/opt/open human/openhuman-core");
    assert_eq!(
        systemd_command_line(exe, &["run".to_string()]).unwrap(),
        "\"/opt/open human/openhuman-core\" run"
    );
    assert_eq!(
        systemd_command_line(exe, &[]).unwrap(),
        "\"/opt/open human/openhuman-core\""
    );
}

#[test]
fn systemd_command_line_keeps_each_argument_one_word() {
    let exe = Path::new("/usr/bin/openhuman-core");
    let args = [
        "--config".to_string(),
        "/home/a b/config.toml".to_string(),
        String::new(),
        "50%".to_string(),
        "$HOME".to_string(),
        "say \"hi\"".to_string(),
        "C:\\dir".to_string(),
    ];
    assert_eq!(
        systemd_command_line(exe, &args).unwrap(),
        "\"/usr/bin/openhuman-core\" --config \"/home/a b/config.toml\" \"\" 50%% $$HOME \"say \\\"hi\\\"\" \"C:\\\\dir\""
    );
}

#[test]
fn systemd_command_line_refuses_a_line_break() {
    let exe = Path::new("/usr/bin/openhuman-core");
    let err = systemd_command_line(exe, &["a\nExecStartPre=/bin/evil".to_string()]).unwrap_err();
    assert!(err.to_string().contains("line break"), "{err}");
}

#[test]
fn cmd_command_line_keeps_a_plain_service_line_unchanged() {
    let exe = Path::new("C:\\Program Files\\OpenHuman\\openhuman-core.exe");
    assert_eq!(
        cmd_command_line(exe, &["run".to_string()]).unwrap(),
        "\"C:\\Program Files\\OpenHuman\\openhuman-core.exe\" run"
    );
}

#[test]
fn cmd_command_line_quotes_spaces_empty_and_metacharacters() {
    let exe = Path::new("C:\\oh\\openhuman-core.exe");
    let args = [
        "--config".to_string(),
        "C:\\User Data\\config.json".to_string(),
        String::new(),
        "a&b".to_string(),
        "100%".to_string(),
        "C:\\User Data\\".to_string(),
    ];
    assert_eq!(
        cmd_command_line(exe, &args).unwrap(),
        "\"C:\\oh\\openhuman-core.exe\" --config \"C:\\User Data\\config.json\" \"\" \"a&b\" 100%% \"C:\\User Data\\\\\""
    );
}

#[test]
fn cmd_command_line_refuses_quotes_and_line_breaks() {
    let exe = Path::new("C:\\oh\\openhuman-core.exe");
    for bad in ["say \"hi\"", "a\r\nnet user"] {
        let err = cmd_command_line(exe, &[bad.to_string()]).unwrap_err();
        assert!(err.to_string().contains("quote or line break"), "{err}");
    }
}
