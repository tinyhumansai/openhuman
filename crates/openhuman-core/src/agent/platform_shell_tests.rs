use super::*;

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
fn std_preserving_builder_does_not_add_pipefail() {
    let command = "false | true";
    let std = build_std_command_preserving_pipe_status(command);
    let tokio = build_tokio_command_preserving_pipe_status(command);

    assert_eq!(std.get_program(), tokio.as_std().get_program());
    assert_eq!(
        std.get_args().collect::<Vec<_>>(),
        tokio.as_std().get_args().collect::<Vec<_>>()
    );
}
