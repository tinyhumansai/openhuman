use std::cmp::Ordering;
use std::collections::BTreeSet;
use std::path::Path;

use chrono::Utc;
use log::{debug, trace, warn};
use uuid::Uuid;

use crate::openhuman::config::Config;
use crate::openhuman::screen_intelligence::{global_engine, CaptureFrame};
use crate::rpc::RpcOutcome;

use super::store;
use super::types::{
    GameplayClipCandidate, GameplayFrameInput, GameplayHighlight, GameplayPlatformDraft,
    GameplayPresetInput, GameplayReviewAnalysis, GameplayReviewAnalysisInput,
    GameplayReviewClipInput, GameplayReviewPreset, GameplayReviewQuestionInput,
    GameplayReviewQuestionResult, GameplayReviewSession, GameplayReviewSessionInput, HighlightKind,
    SpoilerMode,
};

const DEFAULT_PLATFORMS: &[&str] = &["twitch", "kick", "youtube"];

pub async fn register_session(
    payload: GameplayReviewSessionInput,
) -> Result<RpcOutcome<GameplayReviewSession>, String> {
    debug!(
        "[gameplay_review][rpc] register_session start game_id={} frames={} spoiler_mode={}",
        payload.game_id,
        payload.frames.len(),
        payload.spoiler_mode.unwrap_or_default().as_str()
    );
    let session = build_session(payload)?;
    let workspace_dir = workspace_dir().await?;
    debug!(
        "[gameplay_review][rpc] register_session workspace_dir={} session_id={}",
        workspace_dir.display(),
        session.session_id
    );
    store::save_session(&workspace_dir, &session)?;
    debug!(
        "[gameplay_review][rpc] register_session complete session_id={} game_id={}",
        session.session_id, session.game_id
    );
    Ok(RpcOutcome::single_log(
        session,
        "gameplay review session registered",
    ))
}

pub async fn analyze_session(
    payload: GameplayReviewAnalysisInput,
) -> Result<RpcOutcome<GameplayReviewSession>, String> {
    debug!(
        "[gameplay_review][rpc] analyze_session start session_id={} max_highlights={:?} platform_overrides={}",
        payload.session_id,
        payload.max_highlights,
        payload.platforms.len()
    );
    let workspace_dir = workspace_dir().await?;
    let mut session = store::load_session(&workspace_dir, &payload.session_id)?
        .ok_or_else(|| format!("gameplay session not found: {}", payload.session_id))?;
    debug!(
        "[gameplay_review][rpc] analyze_session loaded session_id={} game_id={} frames={} preset_id={:?}",
        session.session_id,
        session.game_id,
        session.frames.len(),
        session.preset_id
    );
    let preset = match session.preset_id.as_deref() {
        Some(game_id) => {
            debug!(
                "[gameplay_review][rpc] analyze_session loading preset game_id={}",
                game_id
            );
            store::load_preset(&workspace_dir, game_id)?
        }
        None => {
            trace!(
                "[gameplay_review][rpc] analyze_session no preset configured session_id={}",
                session.session_id
            );
            None
        }
    };
    let analysis = analyze_session_frames(
        &session,
        preset.as_ref(),
        payload.max_highlights,
        &payload.platforms,
    )
    .await;
    session.analyzed_at_ms = Some(Utc::now().timestamp_millis());
    session.analysis = Some(analysis);
    store::save_session(&workspace_dir, &session)?;
    debug!(
        "[gameplay_review][rpc] analyze_session complete session_id={} highlights={} clips={} drafts={}",
        session.session_id,
        session
            .analysis
            .as_ref()
            .map(|analysis| analysis.highlights.len())
            .unwrap_or(0),
        session
            .analysis
            .as_ref()
            .map(|analysis| analysis.clip_candidates.len())
            .unwrap_or(0),
        session
            .analysis
            .as_ref()
            .map(|analysis| analysis.draft_metadata.len())
            .unwrap_or(0)
    );
    Ok(RpcOutcome::single_log(
        session,
        "gameplay review session analyzed",
    ))
}

pub async fn get_session(session_id: String) -> Result<RpcOutcome<GameplayReviewSession>, String> {
    debug!(
        "[gameplay_review][rpc] get_session start session_id={}",
        session_id
    );
    let workspace_dir = workspace_dir().await?;
    let session = store::load_session(&workspace_dir, &session_id)?
        .ok_or_else(|| format!("gameplay session not found: {session_id}"))?;
    debug!(
        "[gameplay_review][rpc] get_session complete session_id={} game_id={} analyzed={}",
        session.session_id,
        session.game_id,
        session.analysis.is_some()
    );
    Ok(RpcOutcome::single_log(
        session,
        "gameplay review session fetched",
    ))
}

pub async fn list_sessions(
    game_id: Option<String>,
) -> Result<RpcOutcome<Vec<GameplayReviewSession>>, String> {
    debug!(
        "[gameplay_review][rpc] list_sessions start game_id_filter={:?}",
        game_id
    );
    let workspace_dir = workspace_dir().await?;
    let mut sessions = store::list_sessions(&workspace_dir)?;
    if let Some(filter) = game_id.as_deref() {
        let before = sessions.len();
        sessions.retain(|session| session.game_id == filter);
        debug!(
            "[gameplay_review][rpc] list_sessions filtered game_id={} before={} after={}",
            filter,
            before,
            sessions.len()
        );
    }
    debug!(
        "[gameplay_review][rpc] list_sessions complete count={}",
        sessions.len()
    );
    Ok(RpcOutcome::single_log(
        sessions,
        "gameplay review sessions listed",
    ))
}

pub async fn set_preset(
    payload: GameplayPresetInput,
) -> Result<RpcOutcome<GameplayReviewPreset>, String> {
    debug!(
        "[gameplay_review][rpc] set_preset start game_id={} display_name={} focus_items={} spoiler_mode={}",
        payload.game_id,
        payload.display_name,
        payload.coaching_focus.len(),
        payload.spoiler_mode.as_str()
    );
    let workspace_dir = workspace_dir().await?;
    let preset = store::preset_from_input(payload);
    store::save_preset(&workspace_dir, &preset)?;
    debug!(
        "[gameplay_review][rpc] set_preset complete game_id={} path_hint={}",
        preset.game_id,
        store::preset_path(&workspace_dir, &preset.game_id).display()
    );
    Ok(RpcOutcome::single_log(
        preset,
        "gameplay review preset saved",
    ))
}

pub async fn list_presets() -> Result<RpcOutcome<Vec<GameplayReviewPreset>>, String> {
    debug!("[gameplay_review][rpc] list_presets start");
    let workspace_dir = workspace_dir().await?;
    let presets = store::list_presets(&workspace_dir)?;
    debug!(
        "[gameplay_review][rpc] list_presets complete count={}",
        presets.len()
    );
    Ok(RpcOutcome::single_log(
        presets,
        "gameplay review presets listed",
    ))
}

pub async fn ask_session(
    payload: GameplayReviewQuestionInput,
) -> Result<RpcOutcome<GameplayReviewQuestionResult>, String> {
    debug!(
        "[gameplay_review][rpc] ask_session start session_id={} question_len={}",
        payload.session_id,
        payload.question.len()
    );
    let workspace_dir = workspace_dir().await?;
    let session = store::load_session(&workspace_dir, &payload.session_id)?
        .ok_or_else(|| format!("gameplay session not found: {}", payload.session_id))?;
    let analysis = session
        .analysis
        .clone()
        .unwrap_or_else(|| GameplayReviewAnalysis {
            recap: build_recap(&session, &[], None),
            highlights: Vec::new(),
            clip_candidates: Vec::new(),
            draft_metadata: Vec::new(),
            follow_up_questions: default_follow_up_questions(&session),
            spoiler_note: None,
        });
    let answer = answer_question(&session, analysis.clone(), &payload.question);
    let matched_highlights = matched_highlights(&analysis.highlights, &payload.question);
    let suggested_follow_up = if analysis.follow_up_questions.is_empty() {
        default_follow_up_questions(&session)
    } else {
        analysis.follow_up_questions.clone()
    };
    debug!(
        "[gameplay_review][rpc] ask_session complete session_id={} matched_highlights={} suggested_follow_up={}",
        session.session_id,
        matched_highlights.len(),
        suggested_follow_up.len()
    );
    Ok(RpcOutcome::single_log(
        GameplayReviewQuestionResult {
            answer,
            matched_highlights,
            suggested_follow_up,
        },
        "gameplay review question answered",
    ))
}

pub async fn draft_clip_metadata(
    payload: GameplayReviewClipInput,
) -> Result<RpcOutcome<Vec<GameplayPlatformDraft>>, String> {
    debug!(
        "[gameplay_review][rpc] draft_clip_metadata start session_id={} platform={:?} highlight_id={:?}",
        payload.session_id,
        payload.platform,
        payload.highlight_id
    );
    let workspace_dir = workspace_dir().await?;
    let session = store::load_session(&workspace_dir, &payload.session_id)?
        .ok_or_else(|| format!("gameplay session not found: {}", payload.session_id))?;
    let analysis = session
        .analysis
        .as_ref()
        .ok_or_else(|| "session has not been analyzed yet".to_string())?;
    let drafts = draft_metadata(
        &session,
        analysis,
        payload.platform.as_deref(),
        payload.highlight_id.as_deref(),
    );
    debug!(
        "[gameplay_review][rpc] draft_clip_metadata complete session_id={} drafts={}",
        session.session_id,
        drafts.len()
    );
    Ok(RpcOutcome::single_log(
        drafts,
        "gameplay review clip metadata drafted",
    ))
}

async fn workspace_dir() -> Result<std::path::PathBuf, String> {
    debug!("[gameplay_review][rpc] workspace_dir load start");
    let config = Config::load_or_init()
        .await
        .map_err(|err| format!("gameplay review config load failed: {err}"))?;
    let workspace_dir = store::workspace_dir_from_config(&config);
    trace!(
        "[gameplay_review][rpc] workspace_dir resolved path={}",
        workspace_dir.display()
    );
    Ok(workspace_dir)
}

fn build_session(payload: GameplayReviewSessionInput) -> Result<GameplayReviewSession, String> {
    debug!(
        "[gameplay_review][rpc] build_session start game_id={} session_title={} frames={}",
        payload.game_id,
        payload.session_title,
        payload.frames.len()
    );
    if payload.game_id.trim().is_empty() {
        warn!("[gameplay_review][rpc] build_session rejected empty game_id");
        return Err("game_id is required".to_string());
    }
    if payload.session_title.trim().is_empty() {
        warn!("[gameplay_review][rpc] build_session rejected empty session_title");
        return Err("session_title is required".to_string());
    }
    if payload.frames.is_empty() {
        warn!("[gameplay_review][rpc] build_session rejected empty frames");
        return Err("at least one frame is required".to_string());
    }

    let now = Utc::now().timestamp_millis();
    let session_id = format!(
        "gameplay-{}-{}",
        store::slugify(&payload.game_id),
        Uuid::new_v4().simple()
    );
    Ok(GameplayReviewSession {
        session_id,
        game_id: payload.game_id.trim().to_string(),
        session_title: payload.session_title.trim().to_string(),
        source_label: payload
            .source_label
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty()),
        spoiler_mode: payload.spoiler_mode.unwrap_or_default(),
        preset_id: payload
            .preset_id
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty()),
        imported_at_ms: now,
        analyzed_at_ms: None,
        frames: payload.frames,
        analysis: None,
    })
}

async fn analyze_session_frames(
    session: &GameplayReviewSession,
    preset: Option<&GameplayReviewPreset>,
    max_highlights: Option<usize>,
    platform_overrides: &[String],
) -> GameplayReviewAnalysis {
    let limit = max_highlights.unwrap_or(5).clamp(1, 8);
    let platforms: Vec<String> = if platform_overrides.is_empty() {
        DEFAULT_PLATFORMS
            .iter()
            .map(|platform| (*platform).to_string())
            .collect()
    } else {
        platform_overrides.to_vec()
    };
    debug!(
        "[gameplay_review][analysis] start session_id={} game_id={} frames={} limit={} platforms={}",
        session.session_id,
        session.game_id,
        session.frames.len(),
        limit,
        platforms.len()
    );
    if let Some(preset) = preset {
        trace!(
            "[gameplay_review][analysis] using preset game_id={} coaching_focus={} audio_feedback={} spoiler_mode={}",
            preset.game_id,
            preset.coaching_focus.len(),
            preset.audio_feedback,
            preset.spoiler_mode.as_str()
        );
    } else {
        trace!(
            "[gameplay_review][analysis] no preset for session_id={}",
            session.session_id
        );
    }

    let mut highlights = Vec::new();
    let mut clip_candidates = Vec::new();

    for (index, frame) in session.frames.iter().enumerate() {
        trace!(
            "[gameplay_review][analysis] analyze frame session_id={} frame_index={} file_name={}",
            session.session_id,
            index,
            frame.file_name
        );
        let capture = CaptureFrame {
            captured_at_ms: frame
                .captured_at_ms
                .unwrap_or(session.imported_at_ms.saturating_add((index as i64) * 1000)),
            reason: "gameplay_review_import".to_string(),
            app_name: Some(session.game_id.clone()),
            window_title: Some(session.session_title.clone()),
            image_ref: Some(frame.image_ref.clone()),
        };

        let summary = match global_engine().analyze_and_persist_frame(capture).await {
            Ok(summary) => summary,
            Err(err) => {
                warn!(
                    "[gameplay_review][analysis] frame analysis fallback session_id={} frame_index={} error={}",
                    session.session_id,
                    index,
                    err
                );
                fallback_summary(session, frame, index)
            }
        };

        let kind = classify_kind(
            &summary.key_text,
            &summary.actionable_notes,
            session.spoiler_mode,
        );
        let title = highlight_title(&summary.key_text, &frame.file_name, kind);
        let rationale = if summary.actionable_notes.trim().is_empty() {
            summary.key_text.clone()
        } else {
            summary.actionable_notes.clone()
        };
        let confidence = summary.confidence.clamp(0.0, 1.0);
        let highlight = GameplayHighlight {
            id: format!("{}-{}", session.session_id, index),
            frame_index: index,
            captured_at_ms: Some(summary.captured_at_ms),
            title,
            rationale,
            confidence,
            kind,
        };
        highlights.push(highlight);
    }

    highlights.sort_by(|left, right| {
        right
            .confidence
            .partial_cmp(&left.confidence)
            .unwrap_or(Ordering::Equal)
            .then(left.frame_index.cmp(&right.frame_index))
    });
    highlights.truncate(limit);

    for highlight in &highlights {
        clip_candidates.push(clip_candidate_from_highlight(session, highlight));
    }

    let recap = build_recap(session, &highlights, preset);
    let draft_metadata = draft_metadata_from_highlights(session, &highlights, preset, &platforms);
    debug!(
        "[gameplay_review][analysis] complete session_id={} highlights={} clips={} drafts={}",
        session.session_id,
        highlights.len(),
        clip_candidates.len(),
        draft_metadata.len()
    );

    GameplayReviewAnalysis {
        recap,
        highlights,
        clip_candidates,
        draft_metadata,
        follow_up_questions: default_follow_up_questions(session),
        spoiler_note: spoiler_note(session.spoiler_mode),
    }
}

fn fallback_summary(
    session: &GameplayReviewSession,
    frame: &GameplayFrameInput,
    index: usize,
) -> crate::openhuman::screen_intelligence::VisionSummary {
    trace!(
        "[gameplay_review][analysis] fallback_summary session_id={} frame_index={} file_name={}",
        session.session_id,
        index,
        frame.file_name
    );
    let label = frame
        .file_name
        .rsplit_once('/')
        .map(|(_, tail)| tail)
        .unwrap_or(&frame.file_name)
        .trim();
    let captured_at_ms = frame
        .captured_at_ms
        .unwrap_or(session.imported_at_ms.saturating_add((index as i64) * 1000));
    crate::openhuman::screen_intelligence::VisionSummary {
        id: format!("gameplay-{}-{}", captured_at_ms, Uuid::new_v4()),
        captured_at_ms,
        app_name: Some(session.game_id.clone()),
        window_title: Some(session.session_title.clone()),
        ui_state: format!("Gameplay frame {index}: {label}"),
        key_text: format!("Gameplay moment from {}", label),
        actionable_notes: if session.spoiler_mode == SpoilerMode::Off {
            "Keep the coaching spoiler-safe.".to_string()
        } else {
            "Review the clip for highlights and mistakes.".to_string()
        },
        confidence: 0.65,
    }
}

fn classify_kind(key_text: &str, notes: &str, spoiler_mode: SpoilerMode) -> HighlightKind {
    let text = format!("{} {}", key_text, notes).to_ascii_lowercase();
    if text.contains("mistake") || text.contains("miss") || text.contains("throw") {
        return HighlightKind::Mistake;
    }
    if text.contains("clutch")
        || text.contains("highlight")
        || text.contains("fight")
        || text.contains("kill")
    {
        return HighlightKind::Highlight;
    }
    if matches!(spoiler_mode, SpoilerMode::Off) && text.contains("story") {
        return HighlightKind::Coaching;
    }
    HighlightKind::Coaching
}

fn highlight_title(key_text: &str, file_name: &str, kind: HighlightKind) -> String {
    let source = if key_text.trim().is_empty() {
        file_name
    } else {
        key_text
    };
    let base = source
        .split(['.', '!', '?', '\n'])
        .next()
        .unwrap_or(source)
        .trim();
    let prefix = match kind {
        HighlightKind::Highlight => "Highlight",
        HighlightKind::Mistake => "Mistake",
        HighlightKind::Coaching => "Review",
    };
    format!("{prefix}: {}", truncate(base, 72))
}

fn clip_candidate_from_highlight(
    session: &GameplayReviewSession,
    highlight: &GameplayHighlight,
) -> GameplayClipCandidate {
    let start_index = highlight.frame_index.saturating_sub(1);
    let end_index = (highlight.frame_index + 1).min(session.frames.len().saturating_sub(1));
    GameplayClipCandidate {
        id: format!("clip-{}", highlight.id),
        frame_index: highlight.frame_index,
        start_label: session
            .frames
            .get(start_index)
            .map(|frame| frame.file_name.clone())
            .unwrap_or_else(|| format!("frame-{start_index}")),
        end_label: session
            .frames
            .get(end_index)
            .map(|frame| frame.file_name.clone())
            .unwrap_or_else(|| format!("frame-{end_index}")),
        rationale: highlight.rationale.clone(),
        confidence: highlight.confidence,
    }
}

fn draft_metadata(
    session: &GameplayReviewSession,
    analysis: &GameplayReviewAnalysis,
    platform: Option<&str>,
    highlight_id: Option<&str>,
) -> Vec<GameplayPlatformDraft> {
    let filtered = highlight_id
        .and_then(|needle| {
            analysis
                .highlights
                .iter()
                .find(|highlight| highlight.id == needle)
        })
        .cloned()
        .or_else(|| analysis.highlights.first().cloned());
    let highlights = filtered.into_iter().collect::<Vec<_>>();
    let platforms = match platform {
        Some(value) if !value.trim().is_empty() => vec![value.trim().to_string()],
        _ => DEFAULT_PLATFORMS
            .iter()
            .map(|platform| (*platform).to_string())
            .collect(),
    };
    draft_metadata_from_highlights(session, &highlights, None, &platforms)
}

fn draft_metadata_from_highlights(
    session: &GameplayReviewSession,
    highlights: &[GameplayHighlight],
    preset: Option<&GameplayReviewPreset>,
    platforms: &[String],
) -> Vec<GameplayPlatformDraft> {
    let best = highlights.first();
    let best_title = best
        .map(|highlight| highlight.title.clone())
        .unwrap_or_else(|| format!("{} recap", session.session_title));
    let focus = preset
        .and_then(|preset| preset.coaching_focus.first())
        .cloned()
        .unwrap_or_else(|| "clean execution".to_string());

    platforms
        .iter()
        .map(|platform| GameplayPlatformDraft {
            platform: platform.clone(),
            title: format!(
                "{} — {} ({})",
                session.game_id,
                truncate(&best_title, 64),
                platform
            ),
            description: format!(
                "Session: {}\nFocus: {}\nTop moment: {}\nSpoiler mode: {}",
                session.session_title,
                focus,
                best.map(|highlight| highlight.rationale.clone())
                    .unwrap_or_else(|| "No highlight selected yet.".to_string()),
                session.spoiler_mode.as_str(),
            ),
            tags: build_tags(session, best, preset, platform),
        })
        .collect()
}

fn build_tags(
    session: &GameplayReviewSession,
    highlight: Option<&GameplayHighlight>,
    preset: Option<&GameplayReviewPreset>,
    platform: &str,
) -> Vec<String> {
    let mut tags = BTreeSet::new();
    tags.insert(store::slugify(&session.game_id));
    tags.insert(store::slugify(platform));
    if let Some(highlight) = highlight {
        tags.insert(store::slugify(&highlight.title));
    }
    if let Some(preset) = preset {
        for focus in &preset.coaching_focus {
            tags.insert(store::slugify(focus));
        }
    }
    tags.into_iter().take(6).collect()
}

fn build_recap(
    session: &GameplayReviewSession,
    highlights: &[GameplayHighlight],
    preset: Option<&GameplayReviewPreset>,
) -> String {
    let mut recap = format!(
        "Gameplay recap for {} ({})\nSpoiler mode: {}\nFrames reviewed: {}",
        session.session_title,
        session.game_id,
        session.spoiler_mode.as_str(),
        session.frames.len()
    );
    if let Some(preset) = preset {
        recap.push_str(&format!("\nPreset: {}", preset.display_name));
        if !preset.coaching_focus.is_empty() {
            recap.push_str(&format!(
                "\nCoaching focus: {}",
                preset.coaching_focus.join(", ")
            ));
        }
    }
    if highlights.is_empty() {
        recap.push_str("\nNo strong highlights were detected yet.");
    } else {
        recap.push_str("\nTop moments:");
        for highlight in highlights.iter().take(5) {
            recap.push_str(&format!(
                "\n- [{}] {} — {}",
                highlight.kind.as_str(),
                highlight.title,
                highlight.rationale
            ));
        }
    }
    recap
}

fn default_follow_up_questions(session: &GameplayReviewSession) -> Vec<String> {
    vec![
        format!("What were my best moments in {}?", session.session_title),
        format!("Where did I make mistakes in {}?", session.session_title),
        format!("What should I post from this {} session?", session.game_id),
    ]
}

fn spoiler_note(mode: SpoilerMode) -> Option<String> {
    match mode {
        SpoilerMode::Off => Some("Spoiler-safe mode is enabled; avoid story reveals.".to_string()),
        SpoilerMode::Light => {
            Some("Light spoiler filtering is enabled; keep story beats vague.".to_string())
        }
        SpoilerMode::Full => None,
    }
}

fn matched_highlights(highlights: &[GameplayHighlight], question: &str) -> Vec<GameplayHighlight> {
    let lowered = question.to_ascii_lowercase();
    let wants_mistakes =
        lowered.contains("mistake") || lowered.contains("throw") || lowered.contains("bad");
    let wants_highlights = lowered.contains("best")
        || lowered.contains("highlight")
        || lowered.contains("clip")
        || lowered.contains("post");
    highlights
        .iter()
        .filter(|highlight| {
            (wants_mistakes
                && matches!(
                    highlight.kind,
                    HighlightKind::Mistake | HighlightKind::Coaching
                ))
                || (wants_highlights && matches!(highlight.kind, HighlightKind::Highlight))
                || (!wants_mistakes && !wants_highlights)
        })
        .take(3)
        .cloned()
        .collect()
}

fn answer_question(
    session: &GameplayReviewSession,
    analysis: GameplayReviewAnalysis,
    question: &str,
) -> String {
    let lowered = question.to_ascii_lowercase();
    if lowered.contains("best") || lowered.contains("highlight") || lowered.contains("clip") {
        if let Some(highlight) = analysis.highlights.first() {
            return format!(
                "Best clip candidate for {}: {} — {}",
                session.session_title, highlight.title, highlight.rationale
            );
        }
    }
    if lowered.contains("mistake") || lowered.contains("throw") || lowered.contains("miss") {
        if let Some(highlight) = analysis.highlights.iter().find(|highlight| {
            matches!(
                highlight.kind,
                HighlightKind::Mistake | HighlightKind::Coaching
            )
        }) {
            return format!(
                "Review point for {}: {} — {}",
                session.session_title, highlight.title, highlight.rationale
            );
        }
    }
    if lowered.contains("post")
        || lowered.contains("title")
        || lowered.contains("description")
        || lowered.contains("tags")
    {
        if let Some(draft) = analysis.draft_metadata.first() {
            return format!(
                "{} draft for {}: {}. Tags: {}",
                draft.platform,
                session.session_title,
                draft.title,
                draft.tags.join(", ")
            );
        }
    }
    format!("{}\n\n{}", analysis.recap, question)
}

fn truncate(value: &str, max_chars: usize) -> String {
    let chars: Vec<char> = value.chars().collect();
    if chars.len() <= max_chars {
        value.to_string()
    } else {
        chars[..max_chars].iter().collect()
    }
}

impl HighlightKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Highlight => "highlight",
            Self::Mistake => "mistake",
            Self::Coaching => "coaching",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::config::TEST_ENV_LOCK as ENV_LOCK;
    use tempfile::tempdir;

    fn mock_frame(name: &str, image_ref: &str, captured_at_ms: i64) -> GameplayFrameInput {
        GameplayFrameInput {
            file_name: name.to_string(),
            image_ref: image_ref.to_string(),
            captured_at_ms: Some(captured_at_ms),
        }
    }

    #[tokio::test]
    async fn register_and_list_sessions_round_trip() {
        let _guard = ENV_LOCK.lock().unwrap();
        let tmp = tempdir().unwrap();
        unsafe {
            std::env::set_var("OPENHUMAN_WORKSPACE", tmp.path());
        }

        let registered = register_session(GameplayReviewSessionInput {
            game_id: "Apex Legends".to_string(),
            session_title: "Ranked climb".to_string(),
            source_label: Some("/recordings/apex".to_string()),
            spoiler_mode: Some(SpoilerMode::Light),
            preset_id: None,
            frames: vec![mock_frame("frame1.png", "data:image/png;base64,AAA", 1000)],
        })
        .await
        .expect("register")
        .value;

        assert_eq!(registered.game_id, "Apex Legends");
        let listed = list_sessions(None).await.expect("list").value;
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].session_id, registered.session_id);

        unsafe {
            std::env::remove_var("OPENHUMAN_WORKSPACE");
        }
    }

    #[test]
    fn question_matching_prefers_highlights_for_best_prompt() {
        let session = GameplayReviewSession {
            session_id: "session-1".to_string(),
            game_id: "Game".to_string(),
            session_title: "Session".to_string(),
            source_label: None,
            spoiler_mode: SpoilerMode::Light,
            preset_id: None,
            imported_at_ms: 1000,
            analyzed_at_ms: None,
            frames: vec![],
            analysis: None,
        };
        let analysis = GameplayReviewAnalysis {
            recap: "recap".to_string(),
            highlights: vec![GameplayHighlight {
                id: "h1".to_string(),
                frame_index: 0,
                captured_at_ms: Some(1),
                title: "Clutch finish".to_string(),
                rationale: "A clean finish".to_string(),
                confidence: 0.95,
                kind: HighlightKind::Highlight,
            }],
            clip_candidates: Vec::new(),
            draft_metadata: Vec::new(),
            follow_up_questions: vec!["What was the turning point?".to_string()],
            spoiler_note: None,
        };
        let answer = answer_question(&session, analysis.clone(), "What were my best plays?");
        assert!(answer.contains("Clutch finish"));
        assert_eq!(matched_highlights(&analysis.highlights, "best").len(), 1);
    }
}
