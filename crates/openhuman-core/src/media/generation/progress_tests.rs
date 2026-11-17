use super::*;
use tinyagents_harness::tinyinference_video::JobState;

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

fn status(state: JobState) -> VideoJobStatus {
    VideoJobStatus {
        id: "job-1".into(),
        state,
        outputs: 0,
        output_indices: Vec::new(),
        cost_usd: None,
        error: None,
    }
}

#[test]
fn heartbeat_names_the_elapsed_time_and_job_state() {
    assert_eq!(
        heartbeat_message(
            "Generating video",
            Duration::from_secs(95),
            Some("in_progress")
        ),
        "Generating video — 1m 35s elapsed (job in_progress)"
    );
    assert_eq!(
        heartbeat_message("Generating image", Duration::from_secs(15), None),
        "Generating image — 15s elapsed"
    );
}

#[tokio::test(start_paused = true)]
async fn a_long_call_reports_progress_with_the_polled_job_state() {
    let recorder = Recorder::default();
    let slot = JobStateSlot::default();
    let observer = video_status_observer(slot.clone());
    let work = async {
        observer(&status(JobState::Pending));
        tokio::time::sleep(Duration::from_secs(20)).await;
        observer(&status(JobState::InProgress));
        tokio::time::sleep(Duration::from_secs(20)).await;
        "clip.mp4"
    };
    let output = with_progress_heartbeat(
        Some(&recorder),
        "Generating video",
        HEARTBEAT_INTERVAL,
        slot,
        work,
    )
    .await;
    assert_eq!(output, "clip.mp4");
    let updates = recorder.updates.lock().unwrap().clone();
    assert_eq!(
        updates,
        vec![
            "Generating video — 15s elapsed (job pending)".to_string(),
            "Generating video — 30s elapsed (job in_progress)".to_string(),
        ]
    );
}

#[tokio::test(start_paused = true)]
async fn a_quick_call_reports_nothing() {
    let recorder = Recorder::default();
    let output = with_progress_heartbeat(
        Some(&recorder),
        "Generating image",
        HEARTBEAT_INTERVAL,
        JobStateSlot::default(),
        async {
            tokio::time::sleep(Duration::from_secs(3)).await;
            1
        },
    )
    .await;
    assert_eq!(output, 1);
    assert!(recorder.updates.lock().unwrap().is_empty());
}

#[test]
fn concurrent_calls_record_into_their_own_slots() {
    let (a, b) = (JobStateSlot::default(), JobStateSlot::default());
    video_status_observer(a.clone())(&status(JobState::Pending));
    video_status_observer(b.clone())(&status(JobState::InProgress));
    assert_eq!(a.lock().unwrap().as_deref(), Some("pending"));
    assert_eq!(b.lock().unwrap().as_deref(), Some("in_progress"));
}
