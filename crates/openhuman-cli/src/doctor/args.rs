use serde_json::{json, Value};

/// Parsed options for the host-owned doctor command.
pub(super) struct DoctorArgs {
    pub(super) help: bool,
    pub(super) json: bool,
    pub(super) lint: bool,
    pub(super) severity_min: String,
    pub(super) tier: String,
    pub(super) only: Vec<String>,
    pub(super) skip: Vec<String>,
    pub(super) profile: Option<String>,
}

impl DoctorArgs {
    /// Parameters accepted by `openhuman.doctor_report`.
    pub(super) fn rpc_params(&self) -> Value {
        json!({
            "tier": self.tier,
            "only": self.only,
            "skip": self.skip,
            "profile": self.profile
        })
    }
}

pub(super) fn parse_args(args: &[String]) -> Result<DoctorArgs, String> {
    let mut options = DoctorArgs {
        help: false,
        json: false,
        lint: false,
        severity_min: "error".into(),
        tier: "connect".into(),
        only: Vec::new(),
        skip: Vec::new(),
        profile: None,
    };

    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--help" | "-h" | "help" => options.help = true,
            "--json" => options.json = true,
            "--lint" => options.lint = true,
            "--live" => options.tier = "live".into(),
            "--severity-min" => {
                let value = argument_value(args, index, "--severity-min")?;
                if !matches!(value, "info" | "warn" | "error") {
                    return Err(
                        "invalid value for --severity-min: expected info, warn, or error"
                            .to_owned(),
                    );
                }
                options.severity_min = value.into();
                index += 1;
            }
            "--only" => {
                options
                    .only
                    .push(argument_value(args, index, "--only")?.into());
                index += 1;
            }
            "--skip" => {
                options
                    .skip
                    .push(argument_value(args, index, "--skip")?.into());
                index += 1;
            }
            "--profile" => {
                options.profile = Some(argument_value(args, index, "--profile")?.into());
                index += 1;
            }
            flag => return Err(format!("unknown doctor argument: {flag}")),
        }
        index += 1;
    }

    Ok(options)
}

fn argument_value<'a>(args: &'a [String], index: usize, flag: &str) -> Result<&'a str, String> {
    args.get(index + 1)
        .filter(|value| !value.starts_with('-') && !value.trim().is_empty())
        .map(String::as_str)
        .ok_or_else(|| format!("missing value for {flag}"))
}

/// Usage text for `doctor --help`.
pub(super) const USAGE: &str = "\
Usage: openhuman-core doctor [options]

Run the OpenHuman health checks and print a report.

Options:
  --json                    Print the report as JSON
  --live                    Also run live checks (may use your session or API key)
  --lint                    Exit 1 when a finding reaches --severity-min
  --severity-min <level>    info, warn or error (default: error)
  --only <check-id>         Run only this check (repeatable)
  --skip <check-id>         Skip this check (repeatable)
  --profile <name>          Use this desktop profile
  -h, --help                Show this help
";
