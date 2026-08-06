//! Offline CPU/RAM profiler for a locally running OpenHuman Tauri process.
//!
//! This binary is gated by the default-OFF dev-resource-profiler feature and
//! is never linked into the shipped app. It samples the Tauri host, which also
//! embeds openhuman_core, and its CEF descendants. On macOS it also captures an
//! Apple sample report for Rust module-level CPU attribution.
//!
//! Run from the repository root:
//! pnpm profile:tauri --pid PID --duration 15

use std::collections::{BTreeMap, HashMap, HashSet};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::Serialize;
use sysinfo::{Pid, ProcessesToUpdate, System, MINIMUM_CPU_UPDATE_INTERVAL};

const DEFAULT_DURATION_SECS: u64 = 15;
const DEFAULT_INTERVAL_MS: u64 = 250;

#[derive(Debug)]
struct Args {
    pid: u32,
    duration: Duration,
    interval: Duration,
    out_dir: PathBuf,
    capture_stacks: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
enum Component {
    TauriHostAndEmbeddedCore,
    CefRenderer,
    CefGpu,
    CefUtility,
    CefOther,
    OtherChild,
}

impl Component {
    fn label(self) -> &'static str {
        match self {
            Self::TauriHostAndEmbeddedCore => "Tauri host + embedded Rust core",
            Self::CefRenderer => "CEF renderer",
            Self::CefGpu => "CEF GPU",
            Self::CefUtility => "CEF utility",
            Self::CefOther => "CEF other",
            Self::OtherChild => "Other child process",
        }
    }
}

#[derive(Debug, Clone)]
struct ProcessSample {
    pid: u32,
    parent_pid: Option<u32>,
    name: String,
    command: String,
    memory_bytes: u64,
    cpu_percent: f32,
}

#[derive(Debug, Clone, Serialize)]
struct ComponentSample {
    component: Component,
    process_count: usize,
    memory_bytes: u64,
    /// Percentage of one logical CPU. Above 100 means multiple logical CPUs.
    cpu_percent: f32,
}

#[derive(Debug, Clone, Serialize)]
struct TimeSample {
    elapsed_ms: u64,
    components: Vec<ComponentSample>,
}

#[derive(Debug, Clone, Default, Serialize)]
struct ComponentSummary {
    component: Option<Component>,
    sample_count: usize,
    peak_process_count: usize,
    mean_memory_bytes: u64,
    peak_memory_bytes: u64,
    mean_cpu_percent: f32,
    peak_cpu_percent: f32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct RustModuleCpu {
    module: String,
    recursive_samples: u64,
}

#[derive(Debug, Serialize)]
struct ProfileReport {
    schema_version: u32,
    host_pid: u32,
    duration_ms: u64,
    interval_ms: u64,
    logical_cpu_count: usize,
    sampled_at_unix_ms: u128,
    attribution_note: &'static str,
    rust_binary: ComponentSummary,
    desktop_total: ComponentSummary,
    components: Vec<ComponentSummary>,
    samples: Vec<TimeSample>,
    rust_cpu_modules: Vec<RustModuleCpu>,
    cpu_stack_report: Option<String>,
    cpu_stack_error: Option<String>,
}

struct ProfileCapture {
    host_pid: u32,
    duration: Duration,
    interval: Duration,
    logical_cpu_count: usize,
    samples: Vec<TimeSample>,
    rust_cpu_modules: Vec<RustModuleCpu>,
    cpu_stack_report: Option<String>,
    cpu_stack_error: Option<String>,
}

fn main() {
    let arguments = env::args().skip(1).collect::<Vec<_>>();
    if arguments
        .iter()
        .any(|argument| argument == "-h" || argument == "--help")
    {
        println!("{}", usage());
        return;
    }
    if let Err(err) = run(arguments) {
        eprintln!("tauri-resource-profiler: {err}");
        std::process::exit(1);
    }
}

fn run(arguments: Vec<String>) -> Result<(), String> {
    let args = parse_args(arguments)?;
    fs::create_dir_all(&args.out_dir)
        .map_err(|err| format!("create output directory {}: {err}", args.out_dir.display()))?;

    let stack_path = args.out_dir.join("cpu-stacks.txt");
    let (mut stack_child, initial_stack_error) = if args.capture_stacks {
        start_stack_capture(args.pid, args.duration, &stack_path)
    } else {
        (None, None)
    };

    let (logical_cpu_count, samples) = capture_samples(&args)?;
    let final_stack_error = finish_stack_capture(stack_child.as_mut(), initial_stack_error);
    let stack_report = stack_path.exists().then(|| {
        stack_path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned()
    });
    let rust_cpu_modules = fs::read_to_string(&stack_path)
        .map(|contents| parse_rust_module_cpu(&contents))
        .unwrap_or_default();

    let report = build_report(ProfileCapture {
        host_pid: args.pid,
        duration: args.duration,
        interval: args.interval,
        logical_cpu_count,
        samples,
        rust_cpu_modules,
        cpu_stack_report: stack_report,
        cpu_stack_error: final_stack_error,
    })?;
    let json_path = args.out_dir.join("resources.json");
    let markdown_path = args.out_dir.join("resources.md");
    fs::write(
        &json_path,
        serde_json::to_string_pretty(&report).map_err(|err| format!("serialize report: {err}"))?,
    )
    .map_err(|err| format!("write {}: {err}", json_path.display()))?;
    let markdown = render_markdown(&report);
    fs::write(&markdown_path, &markdown)
        .map_err(|err| format!("write {}: {err}", markdown_path.display()))?;

    println!("{markdown}");
    println!("Raw report: {}", json_path.display());
    if stack_path.exists() {
        println!("CPU stacks: {}", stack_path.display());
    }
    Ok(())
}

fn parse_args(arguments: impl IntoIterator<Item = String>) -> Result<Args, String> {
    let mut pid = None;
    let mut duration_secs = DEFAULT_DURATION_SECS;
    let mut interval_ms = DEFAULT_INTERVAL_MS;
    let mut out_dir = None;
    let mut capture_stacks = cfg!(target_os = "macos");
    let mut arguments = arguments.into_iter();

    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "--pid" => pid = Some(parse_value::<u32>("--pid", arguments.next())?),
            "--duration" => {
                duration_secs = parse_value::<u64>("--duration", arguments.next())?;
            }
            "--interval-ms" => {
                interval_ms = parse_value::<u64>("--interval-ms", arguments.next())?;
            }
            "--out" => {
                out_dir = Some(PathBuf::from(
                    arguments
                        .next()
                        .ok_or_else(|| "--out requires a path".to_string())?,
                ));
            }
            "--no-stacks" => capture_stacks = false,
            "--stacks" => capture_stacks = true,
            "--" => {}
            "-h" | "--help" => unreachable!("main handles help before parsing"),
            unknown => return Err(format!("unknown argument {unknown}\n\n{}", usage())),
        }
    }

    let pid = pid.ok_or_else(|| format!("--pid is required\n\n{}", usage()))?;
    if duration_secs == 0 {
        return Err("--duration must be greater than zero".into());
    }
    if interval_ms == 0 {
        return Err("--interval-ms must be greater than zero".into());
    }
    let interval = Duration::from_millis(interval_ms).max(MINIMUM_CPU_UPDATE_INTERVAL);

    Ok(Args {
        pid,
        duration: Duration::from_secs(duration_secs),
        interval,
        out_dir: out_dir.unwrap_or_else(default_out_dir),
        capture_stacks,
    })
}

fn parse_value<T>(flag: &str, value: Option<String>) -> Result<T, String>
where
    T: std::str::FromStr,
{
    let raw = value.ok_or_else(|| format!("{flag} requires a value"))?;
    raw.parse()
        .map_err(|_| format!("invalid {flag} value {raw}"))
}

fn usage() -> &'static str {
    "Usage: pnpm profile:tauri --pid PID [--duration SECONDS] [--interval-ms MS] [--out PATH] [--stacks|--no-stacks]\n\nAttach to the main OpenHuman Tauri PID, not a CEF helper PID. On macOS, CPU stack sampling is enabled by default."
}

fn default_out_dir() -> PathBuf {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    PathBuf::from("target")
        .join("profile")
        .join(format!("tauri-resources-{timestamp}"))
}

fn capture_samples(args: &Args) -> Result<(usize, Vec<TimeSample>), String> {
    let root_pid = Pid::from_u32(args.pid);
    let mut system = System::new_all();
    if system.process(root_pid).is_none() {
        return Err(format!("pid {} is not running", args.pid));
    }

    let started = Instant::now();
    let mut samples = Vec::new();
    while started.elapsed() < args.duration {
        std::thread::sleep(args.interval);
        system.refresh_processes(ProcessesToUpdate::All, true);
        if system.process(root_pid).is_none() {
            return Err(format!("pid {} exited during profiling", args.pid));
        }
        let processes = collect_processes(&system);
        samples.push(TimeSample {
            elapsed_ms: started.elapsed().as_millis() as u64,
            components: group_process_tree(args.pid, &processes)?,
        });
    }
    Ok((system.cpus().len(), samples))
}

fn collect_processes(system: &System) -> Vec<ProcessSample> {
    system
        .processes()
        .iter()
        .map(|(pid, process)| ProcessSample {
            pid: pid.as_u32(),
            parent_pid: process.parent().map(Pid::as_u32),
            name: process.name().to_string_lossy().into_owned(),
            command: process
                .cmd()
                .iter()
                .map(|part| part.to_string_lossy())
                .collect::<Vec<_>>()
                .join(" "),
            memory_bytes: process.memory(),
            cpu_percent: process.cpu_usage(),
        })
        .collect()
}

fn group_process_tree(
    root_pid: u32,
    samples: &[ProcessSample],
) -> Result<Vec<ComponentSample>, String> {
    let by_pid = samples
        .iter()
        .map(|sample| (sample.pid, sample))
        .collect::<HashMap<_, _>>();
    if !by_pid.contains_key(&root_pid) {
        return Err(format!(
            "host pid {root_pid} disappeared from the process table"
        ));
    }

    let mut grouped = BTreeMap::<Component, ComponentSample>::new();
    for sample in samples
        .iter()
        .filter(|sample| belongs_to_tree(sample.pid, root_pid, &by_pid))
    {
        let component = classify_process(sample, root_pid);
        let entry = grouped.entry(component).or_insert(ComponentSample {
            component,
            process_count: 0,
            memory_bytes: 0,
            cpu_percent: 0.0,
        });
        entry.process_count += 1;
        entry.memory_bytes = entry.memory_bytes.saturating_add(sample.memory_bytes);
        entry.cpu_percent += sample.cpu_percent;
    }
    Ok(grouped.into_values().collect())
}

fn belongs_to_tree(pid: u32, root_pid: u32, by_pid: &HashMap<u32, &ProcessSample>) -> bool {
    let mut current = Some(pid);
    let mut seen = HashSet::new();
    while let Some(candidate) = current {
        if candidate == root_pid {
            return true;
        }
        if !seen.insert(candidate) {
            return false;
        }
        current = by_pid.get(&candidate).and_then(|sample| sample.parent_pid);
    }
    false
}

fn classify_process(sample: &ProcessSample, root_pid: u32) -> Component {
    if sample.pid == root_pid {
        return Component::TauriHostAndEmbeddedCore;
    }
    let identity = format!("{} {}", sample.name, sample.command).to_ascii_lowercase();
    if identity.contains("--type=renderer") || identity.contains("renderer") {
        Component::CefRenderer
    } else if identity.contains("--type=gpu-process")
        || identity.contains("gpu process")
        || identity.contains("gpu-process")
    {
        Component::CefGpu
    } else if identity.contains("--type=utility") || identity.contains("utility") {
        Component::CefUtility
    } else if identity.contains("--type=zygote")
        || identity.contains("--type=broker")
        || identity.contains("crashpad")
        || identity.contains("cef")
    {
        Component::CefOther
    } else {
        Component::OtherChild
    }
}

fn build_report(capture: ProfileCapture) -> Result<ProfileReport, String> {
    if capture.samples.is_empty() {
        return Err("profiling produced no samples".into());
    }

    let mut by_component = BTreeMap::<Component, Vec<ComponentSample>>::new();
    let mut totals = Vec::new();
    for sample in &capture.samples {
        for component in &sample.components {
            by_component
                .entry(component.component)
                .or_default()
                .push(component.clone());
        }
        totals.push(ComponentSample {
            component: Component::TauriHostAndEmbeddedCore,
            process_count: sample
                .components
                .iter()
                .map(|value| value.process_count)
                .sum(),
            memory_bytes: sample
                .components
                .iter()
                .map(|value| value.memory_bytes)
                .sum(),
            cpu_percent: sample
                .components
                .iter()
                .map(|value| value.cpu_percent)
                .sum(),
        });
    }
    let components = by_component
        .iter()
        .map(|(component, values)| summarize(Some(*component), values))
        .collect::<Vec<_>>();
    let rust_binary = components
        .iter()
        .find(|summary| summary.component == Some(Component::TauriHostAndEmbeddedCore))
        .cloned()
        .ok_or_else(|| "host process was not present in samples".to_string())?;

    Ok(ProfileReport {
        schema_version: 1,
        host_pid: capture.host_pid,
        duration_ms: capture.duration.as_millis() as u64,
        interval_ms: capture.interval.as_millis() as u64,
        logical_cpu_count: capture.logical_cpu_count,
        sampled_at_unix_ms: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis(),
        attribution_note: "The Rust core is embedded in the Tauri host, so OS process metrics report them together. CEF roles are separate child processes. Use cpu-stacks.txt to attribute host CPU samples to Rust modules.",
        rust_binary,
        desktop_total: summarize(None, &totals),
        components,
        samples: capture.samples,
        rust_cpu_modules: capture.rust_cpu_modules,
        cpu_stack_report: capture.cpu_stack_report,
        cpu_stack_error: capture.cpu_stack_error,
    })
}

fn summarize(component: Option<Component>, values: &[ComponentSample]) -> ComponentSummary {
    let count = values.len().max(1);
    ComponentSummary {
        component,
        sample_count: values.len(),
        peak_process_count: values
            .iter()
            .map(|value| value.process_count)
            .max()
            .unwrap_or(0),
        mean_memory_bytes: values.iter().map(|value| value.memory_bytes).sum::<u64>()
            / count as u64,
        peak_memory_bytes: values
            .iter()
            .map(|value| value.memory_bytes)
            .max()
            .unwrap_or(0),
        mean_cpu_percent: values.iter().map(|value| value.cpu_percent).sum::<f32>() / count as f32,
        peak_cpu_percent: values
            .iter()
            .map(|value| value.cpu_percent)
            .fold(0.0, f32::max),
    }
}

fn render_markdown(report: &ProfileReport) -> String {
    use std::fmt::Write as _;
    let mut output = String::new();
    let _ = writeln!(output, "### Tauri resource profile");
    let _ = writeln!(output);
    let _ = writeln!(
        output,
        "PID {} - {:.1}s - {}ms interval - {} logical CPUs",
        report.host_pid,
        report.duration_ms as f64 / 1000.0,
        report.interval_ms,
        report.logical_cpu_count
    );
    let _ = writeln!(output);
    let _ = writeln!(
        output,
        "| component | processes (peak) | RAM mean | RAM peak | CPU mean | CPU peak |"
    );
    let _ = writeln!(
        output,
        "| --------- | ---------------- | -------- | -------- | -------- | -------- |"
    );
    for summary in &report.components {
        let label = summary
            .component
            .map(Component::label)
            .unwrap_or("Desktop total");
        let _ = writeln!(
            output,
            "| {label} | {} | {:.1} MiB | {:.1} MiB | {:.1}% | {:.1}% |",
            summary.peak_process_count,
            to_mib(summary.mean_memory_bytes),
            to_mib(summary.peak_memory_bytes),
            summary.mean_cpu_percent,
            summary.peak_cpu_percent,
        );
    }
    let total = &report.desktop_total;
    let _ = writeln!(
        output,
        "| **Desktop total** | **{}** | **{:.1} MiB** | **{:.1} MiB** | **{:.1}%** | **{:.1}%** |",
        total.peak_process_count,
        to_mib(total.mean_memory_bytes),
        to_mib(total.peak_memory_bytes),
        total.mean_cpu_percent,
        total.peak_cpu_percent,
    );
    let _ = writeln!(output);
    let _ = writeln!(output, "> {}", report.attribution_note);
    if !report.rust_cpu_modules.is_empty() {
        let _ = writeln!(output);
        let _ = writeln!(output, "| Rust module | recursive CPU samples |");
        let _ = writeln!(output, "| ----------- | --------------------- |");
        for module in report.rust_cpu_modules.iter().take(20) {
            let _ = writeln!(
                output,
                "| {} | {} |",
                module.module, module.recursive_samples
            );
        }
    }
    if let Some(error) = &report.cpu_stack_error {
        let _ = writeln!(output);
        let _ = writeln!(output, "CPU stack capture unavailable: {error}");
    }
    output
}

fn to_mib(bytes: u64) -> f64 {
    bytes as f64 / (1024.0 * 1024.0)
}

fn parse_rust_module_cpu(contents: &str) -> Vec<RustModuleCpu> {
    let recursive_section = contents
        .split("Total number in stack (recursive counted multiple")
        .nth(1)
        .and_then(|tail| tail.split("Sort by top of stack").next())
        .unwrap_or("");
    let mut modules = BTreeMap::<String, u64>::new();
    for line in recursive_section.lines() {
        let trimmed = line.trim();
        let Some((count, symbol_line)) = trimmed.split_once(char::is_whitespace) else {
            continue;
        };
        let Ok(count) = count.parse::<u64>() else {
            continue;
        };
        let symbol = symbol_line
            .split("  (in ")
            .next()
            .unwrap_or(symbol_line)
            .trim();
        let Some(module) = own_rust_module(symbol) else {
            continue;
        };
        *modules.entry(module).or_default() += count;
    }
    let mut modules = modules
        .into_iter()
        .map(|(module, recursive_samples)| RustModuleCpu {
            module,
            recursive_samples,
        })
        .collect::<Vec<_>>();
    modules.sort_by(|left, right| {
        right
            .recursive_samples
            .cmp(&left.recursive_samples)
            .then_with(|| left.module.cmp(&right.module))
    });
    modules
}

fn own_rust_module(symbol: &str) -> Option<String> {
    const CORE_PREFIX: &str = "openhuman_core::openhuman::";
    const TAURI_PREFIX: &str = "openhuman::";
    if let Some(start) = symbol.find(CORE_PREFIX) {
        let tail = &symbol[start + CORE_PREFIX.len()..];
        let domain = tail
            .split("::")
            .next()?
            .trim_matches(|value| value == '<' || value == '>');
        if !domain.is_empty() {
            return Some(format!("{CORE_PREFIX}{domain}"));
        }
    }
    if let Some(start) = symbol.find(TAURI_PREFIX) {
        let tail = &symbol[start + TAURI_PREFIX.len()..];
        let module = tail
            .split("::")
            .next()?
            .trim_matches(|value| value == '<' || value == '>');
        if !module.is_empty() {
            return Some(format!("{TAURI_PREFIX}{module}"));
        }
    }
    None
}

#[cfg(target_os = "macos")]
fn start_stack_capture(
    pid: u32,
    duration: Duration,
    output: &Path,
) -> (Option<Child>, Option<String>) {
    match Command::new("/usr/bin/sample")
        .arg(pid.to_string())
        .arg(duration.as_secs().max(1).to_string())
        .arg("10")
        .arg("-mayDie")
        .arg("-file")
        .arg(output)
        .spawn()
    {
        Ok(child) => (Some(child), None),
        Err(err) => (None, Some(format!("start /usr/bin/sample: {err}"))),
    }
}

#[cfg(not(target_os = "macos"))]
fn start_stack_capture(
    _pid: u32,
    _duration: Duration,
    _output: &Path,
) -> (Option<Child>, Option<String>) {
    (
        None,
        Some("automatic stack capture is currently available on macOS only".into()),
    )
}

fn finish_stack_capture(child: Option<&mut Child>, prior_error: Option<String>) -> Option<String> {
    if prior_error.is_some() {
        return prior_error;
    }
    let child = child?;
    match child.wait() {
        Ok(status) if status.success() => None,
        Ok(status) => Some(format!("stack sampler exited with {status}")),
        Err(err) => Some(format!("wait for stack sampler: {err}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn process(
        pid: u32,
        parent_pid: Option<u32>,
        name: &str,
        command: &str,
        memory_bytes: u64,
        cpu_percent: f32,
    ) -> ProcessSample {
        ProcessSample {
            pid,
            parent_pid,
            name: name.into(),
            command: command.into(),
            memory_bytes,
            cpu_percent,
        }
    }

    #[test]
    fn classifies_cef_roles() {
        assert_eq!(
            classify_process(
                &process(2, Some(1), "OpenHuman Helper", "--type=renderer", 1, 1.0),
                1
            ),
            Component::CefRenderer
        );
        assert_eq!(
            classify_process(
                &process(3, Some(1), "OpenHuman Helper", "--type=gpu-process", 1, 1.0),
                1
            ),
            Component::CefGpu
        );
        assert_eq!(
            classify_process(
                &process(4, Some(1), "OpenHuman Helper", "--type=utility", 1, 1.0),
                1
            ),
            Component::CefUtility
        );
    }

    #[test]
    fn groups_only_host_process_tree() {
        let processes = vec![
            process(10, Some(1), "OpenHuman", "OpenHuman", 100, 20.0),
            process(11, Some(10), "Helper", "--type=renderer", 40, 30.0),
            process(12, Some(11), "Helper", "--type=utility", 10, 5.0),
            process(99, Some(1), "unrelated", "unrelated", 1_000, 100.0),
        ];

        let grouped = group_process_tree(10, &processes).unwrap();
        assert_eq!(grouped.len(), 3);
        assert_eq!(
            grouped.iter().map(|value| value.memory_bytes).sum::<u64>(),
            150
        );
        assert_eq!(
            grouped.iter().map(|value| value.cpu_percent).sum::<f32>(),
            55.0
        );
    }

    #[test]
    fn report_separates_rust_binary_from_desktop_total() {
        let samples = vec![TimeSample {
            elapsed_ms: 250,
            components: vec![
                ComponentSample {
                    component: Component::TauriHostAndEmbeddedCore,
                    process_count: 1,
                    memory_bytes: 100,
                    cpu_percent: 20.0,
                },
                ComponentSample {
                    component: Component::CefRenderer,
                    process_count: 2,
                    memory_bytes: 50,
                    cpu_percent: 30.0,
                },
            ],
        }];
        let report = build_report(ProfileCapture {
            host_pid: 10,
            duration: Duration::from_secs(1),
            interval: Duration::from_millis(250),
            logical_cpu_count: 8,
            samples,
            rust_cpu_modules: Vec::new(),
            cpu_stack_report: None,
            cpu_stack_error: None,
        })
        .unwrap();

        assert_eq!(report.rust_binary.mean_memory_bytes, 100);
        assert_eq!(report.desktop_total.mean_memory_bytes, 150);
        assert_eq!(report.desktop_total.peak_process_count, 3);
        assert!(render_markdown(&report).contains("Tauri host + embedded Rust core"));
    }

    #[test]
    fn parser_requires_pid_and_clamps_cpu_interval() {
        assert!(parse_args(Vec::<String>::new()).is_err());
        let args = parse_args([
            "--pid".into(),
            "123".into(),
            "--duration".into(),
            "2".into(),
            "--interval-ms".into(),
            "1".into(),
            "--no-stacks".into(),
        ])
        .unwrap();
        assert_eq!(args.pid, 123);
        assert_eq!(args.duration, Duration::from_secs(2));
        assert_eq!(args.interval, MINIMUM_CPU_UPDATE_INTERVAL);
        assert!(!args.capture_stacks);
    }

    #[test]
    fn parses_recursive_stack_counts_into_openhuman_modules() {
        let sample = r#"
Total number in stack (recursive counted multiple, when >=5):
        81 openhuman_core::openhuman::agent::run  (in OpenHuman) + 10
        34 <openhuman_core::openhuman::agent::Tool as core::future::Future>::poll  (in OpenHuman) + 2
        17 openhuman_core::openhuman::memory::search  (in OpenHuman) + 4
         9 openhuman::core_process::ensure_running  (in OpenHuman) + 1
       200 tokio::runtime::park  (in OpenHuman) + 3

Sort by top of stack, same collapsed (when >= 5):
"#;
        assert_eq!(
            parse_rust_module_cpu(sample),
            vec![
                RustModuleCpu {
                    module: "openhuman_core::openhuman::agent".into(),
                    recursive_samples: 115,
                },
                RustModuleCpu {
                    module: "openhuman_core::openhuman::memory".into(),
                    recursive_samples: 17,
                },
                RustModuleCpu {
                    module: "openhuman::core_process".into(),
                    recursive_samples: 9,
                },
            ]
        );
    }
}
