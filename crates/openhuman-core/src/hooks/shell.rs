//! The shell that runs `command` hooks, as selected by TinyBox.
//!
//! The hook engine (`tinyagents_runtime::command_hooks`) spawns the hook process
//! itself and asks its host for a synchronous `fn(&str) -> Command`. The host
//! does not choose the shell: it asks TinyBox's `SelectShell` once, at the async
//! boundary where hooks are (re)loaded, and the factory only fills the hook's
//! command into that selection. A selection that never resolved yields a command
//! that fails to spawn, so a hook never silently runs under a different shell.

use crate::core::runtime::DispatchCtx;
use std::sync::{OnceLock, RwLock};

use tinybox_bus::ShellInvocation;

/// Stands in for the hook command while asking the module for a selection. It is
/// substituted back in verbatim wherever the module embedded it.
const PLACEHOLDER: &str = "__OPENHUMAN_HOOK_COMMAND__";

/// A program that does not exist, so an unresolved selection fails the spawn.
const UNAVAILABLE: &str = "openhuman-hook-shell-unavailable";

fn slot() -> &'static RwLock<Option<ShellInvocation>> {
    static SLOT: OnceLock<RwLock<Option<ShellInvocation>>> = OnceLock::new();
    SLOT.get_or_init(|| RwLock::new(None))
}

/// Ask TinyBox which shell it selects and remember it for [`command`].
///
/// A failure is logged and leaves any earlier selection in place.
pub(super) async fn resolve(ctx: &DispatchCtx) {
    #[cfg(feature = "modules")]
    {
        let config = match ctx.load_config().await {
            Ok(config) => config,
            Err(error) => {
                log::warn!("[hooks] cannot read config to select the hook shell: {error}");
                return;
            }
        };
        let request = tinybox_bus::SelectShellRequest {
            command: PLACEHOLDER.to_owned(),
            preserve_pipeline_status: false,
            runtime_path: None,
            python_utf8: false,
        };
        match crate::modules::tinybox::select_shell(&config, request).await {
            Ok(selection) if embeds_placeholder_once(&selection) => install(selection),
            Ok(_) => log::warn!("[hooks] TinyBox shell selection did not embed the hook command"),
            Err(error) => log::warn!("[hooks] TinyBox shell selection failed: {error}"),
        }
    }
    #[cfg(not(feature = "modules"))]
    log::warn!("[hooks] built without `modules`; hook commands cannot select a shell");
}

fn install(selection: ShellInvocation) {
    log::debug!(
        "[hooks] hook shell selected flavor={:?} program_set={}",
        selection.flavor,
        !selection.program.is_empty()
    );
    *slot()
        .write()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(selection);
}

fn embeds_placeholder_once(selection: &ShellInvocation) -> bool {
    let count = selection
        .arguments
        .iter()
        .map(String::as_str)
        .chain(selection.raw_arguments.as_deref())
        .map(|text| text.matches(PLACEHOLDER).count())
        .sum::<usize>();
    count == 1
}

/// Build the process for one hook `command` from the remembered selection.
pub(super) fn command(command: &str) -> tokio::process::Command {
    let selection = slot()
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone();
    let process = match selection {
        Some(selection) => from_selection(&selection, command),
        None => {
            log::warn!("[hooks] no TinyBox shell selection; the hook command will not start");
            std::process::Command::new(UNAVAILABLE)
        }
    };
    let mut process = tokio::process::Command::from(process);
    // A hook leads its own process group and dies with the handle that owns it,
    // so a deadline cannot leave its pipeline running.
    process.kill_on_drop(true);
    tinytools_std::timeout::own_process_group(process.as_std_mut());
    process
}

fn from_selection(selection: &ShellInvocation, command: &str) -> std::process::Command {
    let mut process = std::process::Command::new(&selection.program);
    process.args(
        selection
            .arguments
            .iter()
            .map(|argument| argument.replace(PLACEHOLDER, command)),
    );
    for name in &selection.remove_environment {
        process.env_remove(name);
    }
    #[cfg(windows)]
    if let Some(raw) = &selection.raw_arguments {
        use std::os::windows::process::CommandExt;
        process.raw_arg(raw.replace(PLACEHOLDER, command));
    }
    process
}

#[cfg(test)]
#[path = "shell_tests.rs"]
mod tests;
