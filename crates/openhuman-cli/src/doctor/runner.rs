use serde_json::Value;

use super::args::DoctorArgs;

pub(super) fn doctor_report_body(invoke_result: &Value) -> Result<&Value, String> {
    invoke_result
        .get("result")
        .filter(|result| result.is_object())
        .ok_or_else(|| "doctor RPC response did not include a report".into())
}

/// Why the CLI doctor produced no report. Each class is a fixed, safe summary:
/// the underlying errors can carry endpoints or credential context, so they are
/// never printed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum DoctorFailure {
    /// The async runtime could not be created.
    Runtime,
    /// The doctor host could not start (profile selection, config, or boot).
    Host,
    /// The core rejected or failed the `doctor_report` call.
    Rpc,
    /// The core answered without a report.
    NoReport,
}

impl DoctorFailure {
    pub(super) fn reason(self) -> &'static str {
        match self {
            Self::Runtime => "the async runtime could not start",
            Self::Host => {
                "the doctor host could not start (profile selection, config or core boot)"
            }
            Self::Rpc => "the doctor_report call failed",
            Self::NoReport => "the core answered without a report",
        }
    }
}

pub(super) fn exit_code(options: &DoctorArgs, report: Result<&Value, ()>) -> u8 {
    let Ok(report) = report else {
        return 2;
    };
    if !options.lint {
        return 0;
    }

    let minimum = severity_rank(&options.severity_min);
    let has_finding_at_threshold = report
        .get("findings")
        .and_then(Value::as_array)
        .is_some_and(|findings| {
            findings.iter().any(|finding| {
                let severity = finding
                    .get("severity")
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let rank = severity_rank(severity);
                rank > 0 && rank >= minimum
            })
        });
    u8::from(has_finding_at_threshold)
}

fn severity_rank(severity: &str) -> u8 {
    match severity {
        "info" => 1,
        "warn" => 2,
        "error" => 3,
        _ => 0,
    }
}

pub(crate) fn run(args: &[String]) -> u8 {
    let options = match super::args::parse_args(args) {
        Ok(options) => options,
        Err(error) => {
            eprintln!("doctor: {error}");
            return 2;
        }
    };

    if options.help {
        print!("{}", super::args::USAGE);
        return 0;
    }

    let tokio = match openhuman_rpc::embed::process::tokio_runtime() {
        Ok(runtime) => runtime,
        Err(_) => {
            print_runtime_failure(options.json, DoctorFailure::Runtime);
            return 2;
        }
    };
    let report = tokio.block_on(invoke_report(&options));
    drop(tokio);

    match report {
        Ok(report) => {
            if options.json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&report).unwrap_or_default()
                );
            } else {
                print!(
                    "{}",
                    super::render::render_table_for_args(&report, &options)
                );
            }
            exit_code(&options, Ok(&report))
        }
        Err(failure) => {
            print_runtime_failure(options.json, failure);
            exit_code(&options, Err(()))
        }
    }
}

async fn invoke_report(options: &DoctorArgs) -> Result<Value, DoctorFailure> {
    let runtime = openhuman_rpc::host::doctor_runtime(options.profile.as_deref())
        .await
        .map_err(|_| DoctorFailure::Host)?;
    let invoke_result = runtime
        .core_runtime()
        .invoke("openhuman.doctor_report", options.rpc_params())
        .await
        .map_err(|_| DoctorFailure::Rpc)?;
    doctor_report_body(&invoke_result)
        .cloned()
        .map_err(|_| DoctorFailure::NoReport)
}

pub(super) fn failure_output(json: bool, failure: DoctorFailure) -> String {
    if json {
        serde_json::json!({ "ok": false, "reason": failure.reason() }).to_string()
    } else {
        format!("doctor could not produce a report: {}", failure.reason())
    }
}

fn print_runtime_failure(json: bool, failure: DoctorFailure) {
    if json {
        println!("{}", failure_output(json, failure));
    } else {
        eprintln!("{}", failure_output(json, failure));
    }
}
