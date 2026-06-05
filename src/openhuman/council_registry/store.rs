use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::openhuman::config::Config;
use crate::openhuman::council_registry::types::{
    default_council, CouncilDefinition, DEFAULT_COUNCIL_ID, DEFAULT_MODEL,
};
use crate::rpc::RpcOutcome;

const STORE_DIR: &str = "council_registry";
const STORE_FILE: &str = "councils.json";
const MAX_JURY_COUNT: usize = 5;
const MIN_JURY_COUNT: usize = 1;
const MAX_DEBATE_ROUNDS: usize = 4;
const MIN_DEBATE_ROUNDS: usize = 2;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct CouncilStore {
    #[serde(default)]
    councils: Vec<CouncilDefinition>,
}

pub fn list_councils(config: &Config) -> Result<RpcOutcome<Vec<CouncilDefinition>>, String> {
    let path = store_path(config);
    let mut store = load_store_with_initial_default(&path)?;
    sort_councils(&mut store.councils);
    Ok(RpcOutcome::single_log(
        store.councils,
        "council registry listed",
    ))
}

pub fn get_council(
    config: &Config,
    id: &str,
) -> Result<RpcOutcome<Option<CouncilDefinition>>, String> {
    let path = store_path(config);
    let store = load_store_with_initial_default(&path)?;
    let council = store.councils.into_iter().find(|council| council.id == id);
    Ok(RpcOutcome::single_log(council, "council registry loaded"))
}

pub fn upsert_council(
    config: &Config,
    council: CouncilDefinition,
) -> Result<RpcOutcome<CouncilDefinition>, String> {
    let path = store_path(config);
    let mut store = load_store_with_initial_default(&path)?;

    let now_ms = now_ms();
    let mut normalized = normalize_council(council, now_ms);
    if let Some(existing) = store
        .councils
        .iter()
        .find(|candidate| candidate.id == normalized.id)
    {
        normalized.created_at_ms = existing.created_at_ms;
    }

    store
        .councils
        .retain(|candidate| candidate.id != normalized.id);
    store.councils.push(normalized.clone());
    sort_councils(&mut store.councils);
    save_store(&path, &store)?;
    Ok(RpcOutcome::single_log(normalized, "council registry saved"))
}

pub fn delete_council(config: &Config, id: &str) -> Result<RpcOutcome<bool>, String> {
    let path = store_path(config);
    let mut store = load_store(&path)?;
    let before = store.councils.len();
    store.councils.retain(|council| council.id != id);
    let deleted = before != store.councils.len() || (!path.exists() && id == DEFAULT_COUNCIL_ID);
    save_store(&path, &store)?;
    Ok(RpcOutcome::single_log(deleted, "council registry deleted"))
}

fn store_path(config: &Config) -> PathBuf {
    config.workspace_dir.join(STORE_DIR).join(STORE_FILE)
}

fn load_store(path: &Path) -> Result<CouncilStore, String> {
    if !path.exists() {
        return Ok(CouncilStore::default());
    }
    let contents = fs::read_to_string(path)
        .map_err(|e| format!("failed to read council registry {}: {e}", path.display()))?;
    if contents.trim().is_empty() {
        return Ok(CouncilStore::default());
    }
    serde_json::from_str(&contents)
        .map_err(|e| format!("failed to parse council registry {}: {e}", path.display()))
}

fn load_store_with_initial_default(path: &Path) -> Result<CouncilStore, String> {
    if !path.exists() {
        return Ok(CouncilStore {
            councils: vec![default_council(now_ms())],
        });
    }
    load_store(path)
}

fn save_store(path: &Path, store: &CouncilStore) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| {
            format!(
                "failed to create council registry dir {}: {e}",
                parent.display()
            )
        })?;
    }
    let contents = serde_json::to_string_pretty(store)
        .map_err(|e| format!("failed to serialize council registry: {e}"))?;
    let tmp_path = path.with_extension("json.tmp");
    fs::write(&tmp_path, contents).map_err(|e| {
        format!(
            "failed to write council registry {}: {e}",
            tmp_path.display()
        )
    })?;
    fs::rename(&tmp_path, path)
        .map_err(|e| format!("failed to replace council registry {}: {e}", path.display()))
}

fn normalize_council(mut council: CouncilDefinition, now_ms: i64) -> CouncilDefinition {
    council.id = if council.id.trim().is_empty() {
        format!("council-{}", uuid::Uuid::new_v4())
    } else {
        council.id.trim().to_string()
    };
    council.name = if council.name.trim().is_empty() {
        "Untitled council".to_string()
    } else {
        council.name.trim().to_string()
    };
    council.description = council.description.trim().to_string();
    council.jury_count = council.jury_count.clamp(MIN_JURY_COUNT, MAX_JURY_COUNT);
    council.debate_rounds = council
        .debate_rounds
        .clamp(MIN_DEBATE_ROUNDS, MAX_DEBATE_ROUNDS);
    council.seats.truncate(council.jury_count);
    for (index, seat) in council.seats.iter_mut().enumerate() {
        seat.name = if seat.name.trim().is_empty() {
            format!("Juror {}", index + 1)
        } else {
            seat.name.trim().to_string()
        };
        seat.mode = normalize_mode(&seat.mode);
        seat.profile_id = seat.profile_id.trim().to_string();
        seat.model = normalize_model(&seat.model);
        seat.brief = seat.brief.trim().to_string();
    }
    while council.seats.len() < council.jury_count {
        let id = council.seats.iter().map(|seat| seat.id).max().unwrap_or(0) + 1;
        council.seats.push(
            crate::openhuman::council_registry::types::CouncilSeatDefinition {
                id,
                mode: "default".to_string(),
                profile_id: String::new(),
                name: format!("Juror {}", council.seats.len() + 1),
                model: DEFAULT_MODEL.to_string(),
                brief: String::new(),
            },
        );
    }
    council.judge.mode = normalize_mode(&council.judge.mode);
    council.judge.profile_id = council.judge.profile_id.trim().to_string();
    council.judge.name = if council.judge.name.trim().is_empty() {
        "Chief Judge".to_string()
    } else {
        council.judge.name.trim().to_string()
    };
    council.judge.model = normalize_model(&council.judge.model);
    if council.shared_reasoning.trim().is_empty() {
        council.shared_reasoning =
            crate::openhuman::council_registry::types::DEFAULT_SHARED_REASONING.to_string();
    }
    if council.created_at_ms <= 0 {
        council.created_at_ms = now_ms;
    }
    council.updated_at_ms = now_ms;
    council
}

fn normalize_mode(value: &str) -> String {
    match value.trim() {
        "profile" => "profile".to_string(),
        "custom" => "custom".to_string(),
        _ => "default".to_string(),
    }
}

fn normalize_model(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        DEFAULT_MODEL.to_string()
    } else {
        trimmed.to_string()
    }
}

fn sort_councils(councils: &mut [CouncilDefinition]) {
    councils.sort_by(|a, b| {
        let default_order = (a.id != DEFAULT_COUNCIL_ID).cmp(&(b.id != DEFAULT_COUNCIL_ID));
        default_order
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
            .then_with(|| a.id.cmp(&b.id))
    });
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config_for_workspace(workspace: &Path) -> Config {
        let mut config = Config::default();
        config.workspace_dir = workspace.to_path_buf();
        config
    }

    #[test]
    fn list_returns_built_in_default_when_store_is_empty() {
        let tmp = tempfile::tempdir().unwrap();
        let config = config_for_workspace(tmp.path());

        let payload = list_councils(&config).unwrap().value;

        assert_eq!(payload.len(), 1);
        assert_eq!(payload[0].id, DEFAULT_COUNCIL_ID);
        assert_eq!(payload[0].jury_count, 3);
        assert_eq!(payload[0].seats.len(), 3);
        assert_eq!(payload[0].seats[0].model, DEFAULT_MODEL);
        assert_eq!(payload[0].judge.model, DEFAULT_MODEL);
    }

    #[test]
    fn upsert_get_and_delete_round_trip() {
        let tmp = tempfile::tempdir().unwrap();
        let config = config_for_workspace(tmp.path());
        let mut council = default_council(1);
        council.id.clear();
        council.name = " Product review ".to_string();
        council.jury_count = 8;
        council.debate_rounds = 1;

        let saved = upsert_council(&config, council).unwrap().value;
        assert!(saved.id.starts_with("council-"));
        assert_eq!(saved.name, "Product review");
        assert_eq!(saved.jury_count, MAX_JURY_COUNT);
        assert_eq!(saved.debate_rounds, MIN_DEBATE_ROUNDS);

        let fetched = get_council(&config, &saved.id).unwrap().value.unwrap();
        assert_eq!(fetched.id, saved.id);

        let deleted = delete_council(&config, &saved.id).unwrap().value;
        assert!(deleted);
        let missing = get_council(&config, &saved.id).unwrap().value;
        assert!(missing.is_none());
    }

    #[test]
    fn default_council_can_be_deleted_and_registry_stays_empty() {
        let tmp = tempfile::tempdir().unwrap();
        let config = config_for_workspace(tmp.path());

        let deleted = delete_council(&config, DEFAULT_COUNCIL_ID).unwrap().value;
        assert!(deleted);

        let payload = list_councils(&config).unwrap().value;
        assert!(payload.is_empty());
    }
}
