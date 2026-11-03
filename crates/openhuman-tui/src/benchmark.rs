//! Deterministic, offline measurements of the real viewport and TestBackend renderer.
use super::{render, state::TranscriptState, ui_state::UiState, viewport::ViewportCache};
use openhuman_rpc::embed::chat_surface::{WebChannelEvent, WebEventContext};
use ratatui::{backend::TestBackend, Terminal};
use serde_json::{json, Value};
use std::{hint::black_box, time::Instant};

const SAMPLES: usize = 200;

fn measure(mut operation: impl FnMut(usize)) -> Value {
    for i in 0..20 {
        operation(i);
    }
    let mut samples = Vec::with_capacity(SAMPLES);
    for i in 0..SAMPLES {
        let start = Instant::now();
        operation(i);
        samples.push(start.elapsed().as_nanos() as u64);
    }
    samples.sort_unstable();
    json!({"samples": SAMPLES, "p50_us": samples[SAMPLES / 2] as f64 / 1000.0,
        "p95_us": samples[SAMPLES * 95 / 100] as f64 / 1000.0})
}

/// Emit reproducible measurements without starting a core, logging in, or reading a profile.
pub(crate) fn run() -> anyhow::Result<()> {
    let mut results = Vec::new();
    for (width, height) in [(80, 24), (120, 40)] {
        for entries in [250, 500, 5_000, 10_000, 20_000, 40_000] {
            let mut state = TranscriptState::new("benchmark");
            for index in 0..entries {
                state.push_system(format!(
                    "Settled entry {index:05}: deterministic ASCII text."
                ));
            }
            let mut cache = ViewportCache::default();
            cache.rows(&state, width - 4, height - 9, 0);
            let idle = measure(|_| {
                black_box(cache.rows(&state, width - 4, height - 9, 0));
            });
            let scroll = measure(|i| {
                black_box(cache.rows(&state, width - 4, height - 9, i * 3));
            });
            let mut terminal = Terminal::new(TestBackend::new(width, height))?;
            let mut ui = UiState::new("benchmark".into(), "benchmark".into());
            let render_idle = measure(|_| {
                terminal
                    .draw(|frame| render::draw(frame, &state, &mut ui))
                    .unwrap();
            });
            let streaming = measure(|_| {
                state.apply_event(&WebChannelEvent::TextDelta {
                    context: WebEventContext {
                        client_id: "benchmark".into(),
                        ..Default::default()
                    },
                    delta: Some(" streamed token".into()),
                    delta_kind: None,
                    round: None,
                });
                black_box(cache.rows(&state, width - 4, height - 9, 0));
            });
            results.push(json!({"width":width,"height":height,"entries":entries,
                "settled_rows": entries * 2,"viewport_idle":idle,"viewport_scroll":scroll,
                "render_idle":render_idle,"streaming_tail":streaming}));
        }
    }
    let mut tool = TranscriptState::new("benchmark");
    tool.apply_event(&WebChannelEvent::ToolResult {
        context: WebEventContext {
            client_id: "benchmark".into(),
            ..Default::default()
        },
        tool_call_id: Some("large-output".into()),
        tool_name: Some("benchmark".into()),
        output: Some("deterministic tool output\n".repeat(40_000)),
        success: Some(true),
        skill_id: None,
        args: None,
        round: None,
        failure: None,
        elapsed_ms: None,
        tool_display_label: None,
        tool_display_detail: None,
        structured: None,
    });
    tool.toggle_entry(0);
    let mut cache = ViewportCache::default();
    let large_tool = measure(|_| {
        black_box(cache.rows(&tool, 76, 15, 0));
    });
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "schema":1,"profile":if cfg!(debug_assertions) {"debug"} else {"release"},
            "backend":"ratatui TestBackend; no terminal I/O", "warmup":20,
            "results":results,"large_tool_output":large_tool
        }))?
    );
    Ok(())
}
