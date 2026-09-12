use super::*;
use std::path::PathBuf;

#[test]
fn tokio_command_selects_platform_shell() {
    let cmd = build_tokio_command("echo hi");
    let prog = cmd.as_std().get_program().to_string_lossy().into_owned();
    let args: Vec<String> = cmd
        .as_std()
        .get_args()
        .map(|a| a.to_string_lossy().into_owned())
        .collect();

    if cfg!(windows) {
        assert_eq!(prog, "cmd");
        assert_eq!(args, vec!["/C".to_string(), "echo hi".to_string()]);
    } else if let Some(bash) = bash_path() {
        assert_eq!(prog, bash);
        assert_eq!(
            args,
            vec!["-lc".to_string(), "set -o pipefail\necho hi".to_string()]
        );
    } else {
        assert_eq!(prog, "sh");
        assert_eq!(args, vec!["-lc".to_string(), "echo hi".to_string()]);
    }
}

#[test]
fn std_command_selects_platform_shell() {
    let cmd = build_std_command("echo hi");
    let prog = cmd.get_program().to_string_lossy().into_owned();
    let args: Vec<String> = cmd
        .get_args()
        .map(|a| a.to_string_lossy().into_owned())
        .collect();

    if cfg!(windows) {
        assert_eq!(prog, "cmd");
        assert_eq!(args, vec!["/C".to_string(), "echo hi".to_string()]);
    } else if let Some(bash) = bash_path() {
        assert_eq!(prog, bash);
        assert_eq!(
            args,
            vec!["-lc".to_string(), "set -o pipefail\necho hi".to_string()]
        );
    } else {
        assert_eq!(prog, "sh");
        assert_eq!(args, vec!["-lc".to_string(), "echo hi".to_string()]);
    }
}

#[test]
fn output_redirection_wraps_per_platform() {
    let stdout = PathBuf::from("/tmp/openhuman/out.log");
    let stderr = PathBuf::from("/tmp/openhuman/err.log");
    let wrapped = wrap_with_output_redirection("echo hi", &stdout, &stderr);

    if cfg!(windows) {
        assert_eq!(
            wrapped,
            r#"echo hi > "/tmp/openhuman/out.log" 2> "/tmp/openhuman/err.log""#
        );
    } else {
        assert_eq!(
            wrapped,
            "{ echo hi ; } > '/tmp/openhuman/out.log' 2> '/tmp/openhuman/err.log'"
        );
    }
}
