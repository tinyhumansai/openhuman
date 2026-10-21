use serde_json::Value;

use super::args::DoctorArgs;

pub(super) fn doctor_report_body(invoke_result: &Value) -> Result<&Value, String> {
    invoke_result
        .get("result")
        .filter(|result| result.is_object())
        .ok_or_else(|| "doctor RPC response did not include a report".into())
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

    let tokio = match openhuman_rpc::embed::process::tokio_runtime() {
        Ok(runtime) => runtime,
        Err(_) => {
            print_runtime_failure(options.json);
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
        Err(()) => {
            print_runtime_failure(options.json);
            exit_code(&options, Err(()))
        }
    }
}

async fn invoke_report(options: &DoctorArgs) -> Result<Value, ()> {
    let runtime = openhuman_rpc::host::doctor_runtime(options.profile.as_deref())
        .await
        .map_err(|_| ())?;
    let invoke_result = runtime
        .core_runtime()
        .invoke("openhuman.doctor_report", options.rpc_params())
        .await
        .map_err(|_| ())?;
    doctor_report_body(&invoke_result).cloned().map_err(|_| ())
}

fn print_runtime_failure(json: bool) {
    if json {
        println!("{{\"ok\":false}}");
    } else {
        eprintln!("doctor could not produce a report");
    }
}
