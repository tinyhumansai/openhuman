mod args;
mod render;
mod runner;

pub(crate) use runner::run;

/// Whether the top-level `doctor` invocation belongs to the v2 host command.
///
/// `doctor report` and `doctor models` remain owned by the legacy core
/// namespace dispatcher so their raw JSON behavior does not change.
pub(super) fn is_doctor_command(args: &[String]) -> bool {
    args.first().map(String::as_str) == Some("doctor")
        && !matches!(args.get(1).map(String::as_str), Some("report" | "models"))
}

#[cfg(test)]
#[path = "args_tests.rs"]
mod args_tests;

#[cfg(test)]
#[path = "invoke_tests.rs"]
mod invoke_tests;

#[cfg(test)]
#[path = "render_tests.rs"]
mod render_tests;
