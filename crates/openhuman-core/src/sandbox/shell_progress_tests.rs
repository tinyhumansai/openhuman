use super::*;

/// A context that records what the tool reported.
#[derive(Default)]
struct Recorder {
    updates: Mutex<Vec<String>>,
}

impl ToolRunContext for Recorder {
    fn report_progress(&self, update: ToolProgress) {
        self.updates
            .lock()
            .unwrap()
            .push(update.message.unwrap_or_default());
    }
}

#[test]
fn message_names_elapsed_time_output_size_and_last_line() {
    assert_eq!(
        progress_message(Duration::from_secs(45), 0, None),
        "running 45s · no output yet"
    );
    assert_eq!(
        progress_message(
            Duration::from_secs(252),
            39_117,
            Some("Compiling serde v1.0")
        ),
        "running 4m 12s · 38.2 KB output · last line: Compiling serde v1.0"
    );
    assert_eq!(
        progress_message(Duration::from_secs(61), 3 * 1024 * 1024, None),
        "running 1m 01s · 3.0 MB output"
    );
}

#[test]
fn meter_counts_bytes_and_keeps_the_last_non_empty_line() {
    let meter = ShellOutputMeter::default();
    meter.record(b"one\ntwo\n\n");
    meter.record(b"  three  \n");
    meter.record(b"\n");
    assert_eq!(meter.snapshot(), (20, Some("three".to_string())));
}

#[test]
fn meter_truncates_a_long_last_line() {
    let meter = ShellOutputMeter::default();
    meter.record(format!("{}\n", "x".repeat(5_000)).as_bytes());
    let (_, line) = meter.snapshot();
    assert_eq!(line.unwrap().chars().count(), LAST_LINE_CHARS);
}

/// A line split across chunks is reported whole, never from its second half.
#[test]
fn meter_joins_a_line_split_across_chunks() {
    let meter = ShellOutputMeter::default();
    meter.record(b"Compil");
    assert_eq!(
        meter.snapshot().1,
        None,
        "an unfinished line is not reported"
    );
    meter.record(b"ing serde v1.0\nDown");
    assert_eq!(meter.snapshot().1.as_deref(), Some("Compiling serde v1.0"));
    meter.record(b"loading\n");
    assert_eq!(meter.snapshot().1.as_deref(), Some("Downloading"));
}

/// A progress bar redrawn with carriage returns reports its latest state.
#[test]
fn meter_treats_a_carriage_return_as_a_line_end() {
    let meter = ShellOutputMeter::default();
    meter.record(b"\r 10%\r 55%\r");
    assert_eq!(meter.snapshot().1.as_deref(), Some("55%"));
}

/// A 40s command that writes output reports twice, with the output it has
/// written so far, through the meter handed to the call.
#[tokio::test(start_paused = true)]
async fn a_long_call_reports_its_output_as_it_runs() {
    let recorder = Recorder::default();
    let work = |meter: Option<ShellOutputMeter>| async move {
        let meter = meter.expect("meter handed to the call");
        meter.record(b"Compiling a\n");
        tokio::time::sleep(Duration::from_secs(20)).await;
        meter.record(b"Compiling b\n");
        tokio::time::sleep(Duration::from_secs(20)).await;
        "done"
    };
    let output = with_shell_progress(Some(&recorder), HEARTBEAT_INTERVAL, work).await;
    assert_eq!(output, "done");
    let updates = recorder.updates.lock().unwrap().clone();
    assert_eq!(
        updates,
        vec![
            "running 15s · 12 B output · last line: Compiling a".to_string(),
            "running 30s · 24 B output · last line: Compiling b".to_string(),
        ]
    );
}

#[tokio::test(start_paused = true)]
async fn a_short_call_reports_nothing() {
    let recorder = Recorder::default();
    let output = with_shell_progress(Some(&recorder), HEARTBEAT_INTERVAL, |_| async { 7 }).await;
    assert_eq!(output, 7);
    assert!(recorder.updates.lock().unwrap().is_empty());
}

#[tokio::test]
async fn without_a_context_no_meter_is_handed_out() {
    let seen = with_shell_progress(None, HEARTBEAT_INTERVAL, |meter| async move { meter }).await;
    assert!(seen.is_none());
}
