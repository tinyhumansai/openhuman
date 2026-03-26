//! Identity system supporting OpenClaw (markdown) and AIEOS (JSON) formats.
//!
//! AIEOS (AI Entity Object Specification) is a standardization framework for
//! portable AI identity. This module handles loading and converting AIEOS v1.1
//! JSON to OpenHuman's system prompt format.

use crate::openhuman::config::IdentityConfig;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// AIEOS v1.1 identity structure.
///
/// This follows the AIEOS schema for defining AI agent identity, personality,
/// and behavior. See https://aieos.org for the full specification.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AieosIdentity {
    /// Core identity: names, bio, origin, residence
    #[serde(default)]
    pub identity: Option<IdentitySection>,
    /// Psychology: cognitive weights, MBTI, OCEAN, moral compass
    #[serde(default)]
    pub psychology: Option<PsychologySection>,
    /// Linguistics: text style, formality, catchphrases, forbidden words
    #[serde(default)]
    pub linguistics: Option<LinguisticsSection>,
    /// Motivations: core drive, goals, fears
    #[serde(default)]
    pub motivations: Option<MotivationsSection>,
    /// Capabilities: skills and tools the agent can access
    #[serde(default)]
    pub capabilities: Option<CapabilitiesSection>,
    /// Physicality: visual descriptors for image generation
    #[serde(default)]
    pub physicality: Option<PhysicalitySection>,
    /// History: origin story, education, occupation
    #[serde(default)]
    pub history: Option<HistorySection>,
    /// Interests: hobbies, favorites, lifestyle
    #[serde(default)]
    pub interests: Option<InterestsSection>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct IdentitySection {
    #[serde(default)]
    pub names: Option<Names>,
    #[serde(default)]
    pub bio: Option<String>,
    #[serde(default)]
    pub origin: Option<String>,
    #[serde(default)]
    pub residence: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Names {
    #[serde(default)]
    pub first: Option<String>,
    #[serde(default)]
    pub last: Option<String>,
    #[serde(default)]
    pub nickname: Option<String>,
    #[serde(default)]
    pub full: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PsychologySection {
    #[serde(default)]
    pub neural_matrix: Option<HashMap<String, f64>>,
    #[serde(default)]
    pub mbti: Option<String>,
    #[serde(default)]
    pub ocean: Option<OceanTraits>,
    #[serde(default)]
    pub moral_compass: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct OceanTraits {
    #[serde(default)]
    pub openness: Option<f64>,
    #[serde(default)]
    pub conscientiousness: Option<f64>,
    #[serde(default)]
    pub extraversion: Option<f64>,
    #[serde(default)]
    pub agreeableness: Option<f64>,
    #[serde(default)]
    pub neuroticism: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LinguisticsSection {
    #[serde(default)]
    pub style: Option<String>,
    #[serde(default)]
    pub formality: Option<String>,
    #[serde(default)]
    pub catchphrases: Option<Vec<String>>,
    #[serde(default)]
    pub forbidden_words: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct MotivationsSection {
    #[serde(default)]
    pub core_drive: Option<String>,
    #[serde(default)]
    pub short_term_goals: Option<Vec<String>>,
    #[serde(default)]
    pub long_term_goals: Option<Vec<String>>,
    #[serde(default)]
    pub fears: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CapabilitiesSection {
    #[serde(default)]
    pub skills: Option<Vec<String>>,
    #[serde(default)]
    pub tools: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PhysicalitySection {
    #[serde(default)]
    pub appearance: Option<String>,
    #[serde(default)]
    pub avatar_description: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct HistorySection {
    #[serde(default)]
    pub origin_story: Option<String>,
    #[serde(default)]
    pub education: Option<Vec<String>>,
    #[serde(default)]
    pub occupation: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct InterestsSection {
    #[serde(default)]
    pub hobbies: Option<Vec<String>>,
    #[serde(default)]
    pub favorites: Option<HashMap<String, String>>,
    #[serde(default)]
    pub lifestyle: Option<String>,
}

/// Load AIEOS identity from config (file path or inline JSON).
///
/// Checks `aieos_path` first, then `aieos_inline`. Returns `Ok(None)` if
/// neither is configured.
pub fn load_aieos_identity(
    config: &IdentityConfig,
    workspace_dir: &Path,
) -> Result<Option<AieosIdentity>> {
    // Only load AIEOS if format is explicitly set to "aieos"
    if config.format != "aieos" {
        return Ok(None);
    }

    // Try aieos_path first
    if let Some(ref path) = config.aieos_path {
        let full_path = if Path::new(path).is_absolute() {
            PathBuf::from(path)
        } else {
            workspace_dir.join(path)
        };

        let content = std::fs::read_to_string(&full_path)
            .with_context(|| format!("Failed to read AIEOS file: {}", full_path.display()))?;

        let identity = parse_aieos_identity(&content)
            .with_context(|| format!("Failed to parse AIEOS JSON from: {}", full_path.display()))?;

        return Ok(Some(identity));
    }

    // Fall back to aieos_inline
    if let Some(ref inline) = config.aieos_inline {
        let identity = parse_aieos_identity(inline).context("Failed to parse inline AIEOS JSON")?;

        return Ok(Some(identity));
    }

    // Format is "aieos" but neither path nor inline is configured
    anyhow::bail!(
        "Identity format is set to 'aieos' but neither aieos_path nor aieos_inline is configured. \
         Set one in your config:\n\
         \n\
         [identity]\n\
         format = \"aieos\"\n\
         aieos_path = \"identity.json\"\n\
         \n\
         Or use inline:\n\
         \n\
         [identity]\n\
         format = \"aieos\"\n\
         aieos_inline = '{{\"identity\": {{...}}}}'"
    )
}

fn parse_aieos_identity(content: &str) -> Result<AieosIdentity> {
    let payload: Value = serde_json::from_str(content).context("Invalid AIEOS JSON")?;
    if !payload.is_object() {
        anyhow::bail!("AIEOS payload must be a JSON object")
    }
    Ok(normalize_aieos_identity(&payload))
}

fn normalize_aieos_identity(payload: &Value) -> AieosIdentity {
    AieosIdentity {
        identity: normalize_identity_section(value_at_path(payload, &["identity"])),
        psychology: normalize_psychology_section(value_at_path(payload, &["psychology"])),
        linguistics: normalize_linguistics_section(value_at_path(payload, &["linguistics"])),
        motivations: normalize_motivations_section(value_at_path(payload, &["motivations"])),
        capabilities: normalize_capabilities_section(value_at_path(payload, &["capabilities"])),
        physicality: normalize_physicality_section(value_at_path(payload, &["physicality"])),
        history: normalize_history_section(value_at_path(payload, &["history"])),
        interests: normalize_interests_section(value_at_path(payload, &["interests"])),
    }
}

fn normalize_identity_section(section: Option<&Value>) -> Option<IdentitySection> {
    let section = section?;

    let names = normalize_names(value_at_path(section, &["names"]));
    let bio = value_at_path(section, &["bio"]).and_then(value_to_text);
    let origin = value_at_path(section, &["origin"]).and_then(value_to_text);
    let residence = value_at_path(section, &["residence"]).and_then(value_to_text);

    if names.is_none() && bio.is_none() && origin.is_none() && residence.is_none() {
        return None;
    }

    Some(IdentitySection {
        names,
        bio,
        origin,
        residence,
    })
}

fn normalize_names(value: Option<&Value>) -> Option<Names> {
    let value = value?;

    let mut names = Names {
        first: value_at_path(value, &["first"]).and_then(scalar_to_string),
        last: value_at_path(value, &["last"]).and_then(scalar_to_string),
        nickname: value_at_path(value, &["nickname"]).and_then(scalar_to_string),
        full: value_at_path(value, &["full"]).and_then(scalar_to_string),
    };

    if names.full.is_none() {
        if let (Some(first), Some(last)) = (&names.first, &names.last) {
            names.full = Some(format!("{first} {last}"));
        }
    }

    if names.first.is_none()
        && names.last.is_none()
        && names.nickname.is_none()
        && names.full.is_none()
    {
        return None;
    }

    Some(names)
}

fn normalize_psychology_section(section: Option<&Value>) -> Option<PsychologySection> {
    let section = section?;

    let neural_matrix = value_at_path(section, &["neural_matrix"]).and_then(numeric_map_from_value);
    let mbti = value_at_path(section, &["mbti"])
        .and_then(scalar_to_string)
        .or_else(|| value_at_path(section, &["traits", "mbti"]).and_then(scalar_to_string));
    let ocean = value_at_path(section, &["ocean"])
        .or_else(|| value_at_path(section, &["traits", "ocean"]))
        .and_then(normalize_ocean_traits);
    let moral_compass = value_at_path(section, &["moral_compass"])
        .map(normalize_moral_compass)
        .filter(|items| !items.is_empty());

    if neural_matrix.is_none() && mbti.is_none() && ocean.is_none() && moral_compass.is_none() {
        return None;
    }

    Some(PsychologySection {
        neural_matrix,
        mbti,
        ocean,
        moral_compass,
    })
}

fn normalize_ocean_traits(value: &Value) -> Option<OceanTraits> {
    let value = value.as_object()?;
    let traits = OceanTraits {
        openness: value.get("openness").and_then(numeric_from_value),
        conscientiousness: value.get("conscientiousness").and_then(numeric_from_value),
        extraversion: value.get("extraversion").and_then(numeric_from_value),
        agreeableness: value.get("agreeableness").and_then(numeric_from_value),
        neuroticism: value.get("neuroticism").and_then(numeric_from_value),
    };

    if traits.openness.is_none()
        && traits.conscientiousness.is_none()
        && traits.extraversion.is_none()
        && traits.agreeableness.is_none()
        && traits.neuroticism.is_none()
    {
        return None;
    }

    Some(traits)
}

fn normalize_moral_compass(value: &Value) -> Vec<String> {
    let mut values = Vec::new();

    if let Some(map) = value.as_object() {
        if let Some(alignment) = map.get("alignment").and_then(scalar_to_string) {
            values.push(format!("Alignment: {alignment}"));
        }
        if let Some(core_values) = map.get("core_values") {
            values.extend(list_from_value(core_values));
        }
        if let Some(conflict_style) = map
            .get("conflict_resolution_style")
            .and_then(scalar_to_string)
        {
            values.push(format!("Conflict Style: {conflict_style}"));
        }
        if values.is_empty() {
            values.extend(list_from_value(value));
        }
    } else {
        values.extend(list_from_value(value));
    }

    dedupe_non_empty(values)
}

fn normalize_linguistics_section(section: Option<&Value>) -> Option<LinguisticsSection> {
    let section = section?;

    let style = value_at_path(section, &["style"])
        .and_then(value_to_text)
        .or_else(|| {
            non_empty_list_at(section, &["text_style", "style_descriptors"])
                .map(|list| list.join(", "))
        });

    let formality = value_at_path(section, &["formality"])
        .and_then(value_to_text)
        .or_else(|| {
            value_at_path(section, &["text_style", "formality_level"]).and_then(|value| {
                numeric_from_value(value)
                    .map(|n| format!("{n:.2}"))
                    .or_else(|| value_to_text(value))
            })
        });

    let catchphrases = non_empty_list_at(section, &["catchphrases"])
        .or_else(|| non_empty_list_at(section, &["idiolect", "catchphrases"]));

    let forbidden_words = non_empty_list_at(section, &["forbidden_words"])
        .or_else(|| non_empty_list_at(section, &["idiolect", "forbidden_words"]));

    if style.is_none() && formality.is_none() && catchphrases.is_none() && forbidden_words.is_none()
    {
        return None;
    }

    Some(LinguisticsSection {
        style,
        formality,
        catchphrases,
        forbidden_words,
    })
}

fn normalize_motivations_section(section: Option<&Value>) -> Option<MotivationsSection> {
    let section = section?;

    let core_drive = value_at_path(section, &["core_drive"]).and_then(value_to_text);
    let short_term_goals = non_empty_list_at(section, &["short_term_goals"])
        .or_else(|| non_empty_list_at(section, &["goals", "short_term"]));
    let long_term_goals = non_empty_list_at(section, &["long_term_goals"])
        .or_else(|| non_empty_list_at(section, &["goals", "long_term"]));

    let fears = value_at_path(section, &["fears"]).and_then(|fears| {
        let values = if fears.is_object() {
            let mut combined =
                non_empty_list_at(section, &["fears", "rational"]).unwrap_or_default();
            if let Some(mut irrational) = non_empty_list_at(section, &["fears", "irrational"]) {
                combined.append(&mut irrational);
            }
            if combined.is_empty() {
                list_from_value(fears)
            } else {
                combined
            }
        } else {
            list_from_value(fears)
        };

        let deduped = dedupe_non_empty(values);
        if deduped.is_empty() {
            None
        } else {
            Some(deduped)
        }
    });

    if core_drive.is_none()
        && short_term_goals.is_none()
        && long_term_goals.is_none()
        && fears.is_none()
    {
        return None;
    }

    Some(MotivationsSection {
        core_drive,
        short_term_goals,
        long_term_goals,
        fears,
    })
}

fn normalize_capabilities_section(section: Option<&Value>) -> Option<CapabilitiesSection> {
    let section = section?;

    let skills = non_empty_list_at(section, &["skills"]);
    let tools = non_empty_list_at(section, &["tools"]);

    if skills.is_none() && tools.is_none() {
        return None;
    }

    Some(CapabilitiesSection { skills, tools })
}

fn normalize_physicality_section(section: Option<&Value>) -> Option<PhysicalitySection> {
    let section = section?;

    let appearance = value_at_path(section, &["appearance"])
        .and_then(value_to_text)
        .or_else(|| {
            let mut descriptors = Vec::new();
            if let Some(face_shape) =
                value_at_path(section, &["face", "shape"]).and_then(scalar_to_string)
            {
                descriptors.push(format!("Face shape: {face_shape}"));
            }
            if let Some(build_description) =
                value_at_path(section, &["body", "build_description"]).and_then(scalar_to_string)
            {
                descriptors.push(format!("Build: {build_description}"));
            }
            if let Some(aesthetic) =
                value_at_path(section, &["style", "aesthetic_archetype"]).and_then(scalar_to_string)
            {
                descriptors.push(format!("Aesthetic: {aesthetic}"));
            }
            if descriptors.is_empty() {
                None
            } else {
                Some(descriptors.join("; "))
            }
        });

    let avatar_description = value_at_path(section, &["avatar_description"])
        .and_then(value_to_text)
        .or_else(|| value_at_path(section, &["image_prompts", "portrait"]).and_then(value_to_text));

    if appearance.is_none() && avatar_description.is_none() {
        return None;
    }

    Some(PhysicalitySection {
        appearance,
        avatar_description,
    })
}

fn normalize_history_section(section: Option<&Value>) -> Option<HistorySection> {
    let section = section?;

    let origin_story = value_at_path(section, &["origin_story"]).and_then(value_to_text);
    let education = non_empty_list_at(section, &["education"]);
    let occupation = value_at_path(section, &["occupation"]).and_then(value_to_text);

    if origin_story.is_none() && education.is_none() && occupation.is_none() {
        return None;
    }

    Some(HistorySection {
        origin_story,
        education,
        occupation,
    })
}

fn normalize_interests_section(section: Option<&Value>) -> Option<InterestsSection> {
    let section = section?;

    let hobbies = non_empty_list_at(section, &["hobbies"]);
    let favorites = value_at_path(section, &["favorites"]).and_then(favorites_map);
    let lifestyle = value_at_path(section, &["lifestyle"]).and_then(value_to_text);

    if hobbies.is_none() && favorites.is_none() && lifestyle.is_none() {
        return None;
    }

    Some(InterestsSection {
        hobbies,
        favorites,
        lifestyle,
    })
}

fn value_at_path<'a>(value: &'a Value, path: &[&str]) -> Option<&'a Value> {
    let mut current = value;
    for segment in path {
        current = current.as_object()?.get(*segment)?;
    }
    Some(current)
}

fn scalar_to_string(value: &Value) -> Option<String> {
    match value {
        Value::String(text) => {
            let trimmed = text.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_owned())
            }
        }
        Value::Number(number) => Some(number.to_string()),
        Value::Bool(boolean) => Some(boolean.to_string()),
        _ => None,
    }
}

fn value_to_text(value: &Value) -> Option<String> {
    match value {
        Value::Null => None,
        Value::String(_) | Value::Number(_) | Value::Bool(_) => scalar_to_string(value),
        Value::Array(_) => {
            let values = list_from_value(value);
            if values.is_empty() {
                None
            } else {
                Some(values.join(", "))
            }
        }
        Value::Object(map) => summarize_object(map),
    }
}

fn summarize_object(map: &Map<String, Value>) -> Option<String> {
    let mut parts = Vec::new();
    summarize_object_into_parts("", map, &mut parts);
    if parts.is_empty() {
        None
    } else {
        Some(parts.join("; "))
    }
}

fn summarize_object_into_parts(prefix: &str, map: &Map<String, Value>, parts: &mut Vec<String>) {
    for (key, value) in map {
        if key.starts_with('@') {
            continue;
        }

        let label = key.replace('_', " ");
        let full_label = if prefix.is_empty() {
            label
        } else {
            format!("{prefix} {label}")
        };

        match value {
            Value::Object(inner) => summarize_object_into_parts(&full_label, inner, parts),
            Value::Array(_) => {
                let values = list_from_value(value);
                if !values.is_empty() {
                    parts.push(format!("{full_label}: {}", values.join(", ")));
                }
            }
            _ => {
                if let Some(text) = scalar_to_string(value) {
                    parts.push(format!("{full_label}: {text}"));
                }
            }
        }
    }
}

fn list_from_value(value: &Value) -> Vec<String> {
    let mut values = Vec::new();

    match value {
        Value::Array(entries) => {
            for entry in entries {
                values.extend(list_from_value(entry));
            }
        }
        Value::Object(map) => {
            if let Some(name) = map.get("name").and_then(scalar_to_string) {
                values.push(name);
            } else if let Some(title) = map.get("title").and_then(scalar_to_string) {
                values.push(title);
            } else if let Some(summary) = summarize_object(map) {
                values.push(summary);
            }
        }
        _ => {
            if let Some(text) = scalar_to_string(value) {
                values.push(text);
            }
        }
    }

    dedupe_non_empty(values)
}

fn dedupe_non_empty(values: Vec<String>) -> Vec<String> {
    let mut deduped = Vec::new();
    for value in values {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            continue;
        }
        if !deduped
            .iter()
            .any(|existing: &String| existing.eq_ignore_ascii_case(trimmed))
        {
            deduped.push(trimmed.to_owned());
        }
    }
    deduped
}

fn numeric_map_from_value(value: &Value) -> Option<HashMap<String, f64>> {
    let map = value.as_object()?;
    let mut numeric_values = HashMap::new();

    for (key, entry) in map {
        if key.starts_with('@') {
            continue;
        }
        if let Some(number) = numeric_from_value(entry) {
            numeric_values.insert(key.clone(), number);
        }
    }

    if numeric_values.is_empty() {
        None
    } else {
        Some(numeric_values)
    }
}

fn numeric_from_value(value: &Value) -> Option<f64> {
    match value {
        Value::Number(number) => number.as_f64(),
        Value::String(text) => text.parse::<f64>().ok(),
        _ => None,
    }
}

fn favorites_map(value: &Value) -> Option<HashMap<String, String>> {
    let map = value.as_object()?;
    let mut favorites = HashMap::new();

    for (key, entry) in map {
        if key.starts_with('@') {
            continue;
        }
        if let Some(text) = value_to_text(entry) {
            favorites.insert(key.clone(), text);
        }
    }

    if favorites.is_empty() {
        None
    } else {
        Some(favorites)
    }
}

fn non_empty_list_at(value: &Value, path: &[&str]) -> Option<Vec<String>> {
    let values = value_at_path(value, path).map(list_from_value)?;
    if values.is_empty() {
        None
    } else {
        Some(values)
    }
}

/// Convert AIEOS identity to a system prompt string.
///
/// Formats the AIEOS data into a structured markdown prompt compatible
/// with OpenHuman's agent system.
pub fn aieos_to_system_prompt(identity: &AieosIdentity) -> String {
    use std::fmt::Write;
    let mut prompt = String::new();

    // ── Identity Section ───────────────────────────────────────────
    if let Some(ref id) = identity.identity {
        prompt.push_str("## Identity\n\n");

        if let Some(ref names) = id.names {
            if let Some(ref first) = names.first {
                let _ = writeln!(prompt, "**Name:** {}", first);
                if let Some(ref last) = names.last {
                    let _ = writeln!(prompt, "**Full Name:** {} {}", first, last);
                }
            } else if let Some(ref full) = names.full {
                let _ = writeln!(prompt, "**Name:** {}", full);
            }

            if let Some(ref nickname) = names.nickname {
                let _ = writeln!(prompt, "**Nickname:** {}", nickname);
            }
        }

        if let Some(ref bio) = id.bio {
            let _ = writeln!(prompt, "**Bio:** {}", bio);
        }

        if let Some(ref origin) = id.origin {
            let _ = writeln!(prompt, "**Origin:** {}", origin);
        }

        if let Some(ref residence) = id.residence {
            let _ = writeln!(prompt, "**Residence:** {}", residence);
        }

        prompt.push('\n');
    }

    // ── Psychology Section ──────────────────────────────────────────
    if let Some(ref psych) = identity.psychology {
        prompt.push_str("## Personality\n\n");

        if let Some(ref mbti) = psych.mbti {
            let _ = writeln!(prompt, "**MBTI:** {}", mbti);
        }

        if let Some(ref ocean) = psych.ocean {
            prompt.push_str("**OCEAN Traits:**\n");
            if let Some(o) = ocean.openness {
                let _ = writeln!(prompt, "- Openness: {:.2}", o);
            }
            if let Some(c) = ocean.conscientiousness {
                let _ = writeln!(prompt, "- Conscientiousness: {:.2}", c);
            }
            if let Some(e) = ocean.extraversion {
                let _ = writeln!(prompt, "- Extraversion: {:.2}", e);
            }
            if let Some(a) = ocean.agreeableness {
                let _ = writeln!(prompt, "- Agreeableness: {:.2}", a);
            }
            if let Some(n) = ocean.neuroticism {
                let _ = writeln!(prompt, "- Neuroticism: {:.2}", n);
            }
        }

        if let Some(ref matrix) = psych.neural_matrix {
            if !matrix.is_empty() {
                prompt.push_str("\n**Neural Matrix (Cognitive Weights):**\n");
                let mut sorted_keys: Vec<_> = matrix.keys().collect();
                sorted_keys.sort();
                for trait_name in sorted_keys {
                    let weight = matrix.get(trait_name).unwrap();
                    let _ = writeln!(prompt, "- {}: {:.2}", trait_name, weight);
                }
            }
        }

        if let Some(ref compass) = psych.moral_compass {
            if !compass.is_empty() {
                prompt.push_str("\n**Moral Compass:**\n");
                for principle in compass {
                    let _ = writeln!(prompt, "- {}", principle);
                }
            }
        }

        prompt.push('\n');
    }

    // ── Linguistics Section ────────────────────────────────────────
    if let Some(ref ling) = identity.linguistics {
        prompt.push_str("## Communication Style\n\n");

        if let Some(ref style) = ling.style {
            let _ = writeln!(prompt, "**Style:** {}", style);
        }

        if let Some(ref formality) = ling.formality {
            let _ = writeln!(prompt, "**Formality Level:** {}", formality);
        }

        if let Some(ref phrases) = ling.catchphrases {
            if !phrases.is_empty() {
                prompt.push_str("**Catchphrases:**\n");
                for phrase in phrases {
                    let _ = writeln!(prompt, "- \"{}\"", phrase);
                }
            }
        }

        if let Some(ref forbidden) = ling.forbidden_words {
            if !forbidden.is_empty() {
                prompt.push_str("\n**Words/Phrases to Avoid:**\n");
                for word in forbidden {
                    let _ = writeln!(prompt, "- {}", word);
                }
            }
        }

        prompt.push('\n');
    }

    // ── Motivations Section ──────────────────────────────────────────
    if let Some(ref mot) = identity.motivations {
        prompt.push_str("## Motivations\n\n");

        if let Some(ref drive) = mot.core_drive {
            let _ = writeln!(prompt, "**Core Drive:** {}", drive);
        }

        if let Some(ref short) = mot.short_term_goals {
            if !short.is_empty() {
                prompt.push_str("**Short-term Goals:**\n");
                for goal in short {
                    let _ = writeln!(prompt, "- {}", goal);
                }
            }
        }

        if let Some(ref long) = mot.long_term_goals {
            if !long.is_empty() {
                prompt.push_str("\n**Long-term Goals:**\n");
                for goal in long {
                    let _ = writeln!(prompt, "- {}", goal);
                }
            }
        }

        if let Some(ref fears) = mot.fears {
            if !fears.is_empty() {
                prompt.push_str("\n**Fears/Avoidances:**\n");
                for fear in fears {
                    let _ = writeln!(prompt, "- {}", fear);
                }
            }
        }

        prompt.push('\n');
    }

    // ── Capabilities Section ────────────────────────────────────────
    if let Some(ref cap) = identity.capabilities {
        prompt.push_str("## Capabilities\n\n");

        if let Some(ref skills) = cap.skills {
            if !skills.is_empty() {
                prompt.push_str("**Skills:**\n");
                for skill in skills {
                    let _ = writeln!(prompt, "- {}", skill);
                }
            }
        }

        if let Some(ref tools) = cap.tools {
            if !tools.is_empty() {
                prompt.push_str("\n**Tools Access:**\n");
                for tool in tools {
                    let _ = writeln!(prompt, "- {}", tool);
                }
            }
        }

        prompt.push('\n');
    }

    // ── History Section ─────────────────────────────────────────────
    if let Some(ref hist) = identity.history {
        prompt.push_str("## Background\n\n");

        if let Some(ref story) = hist.origin_story {
            let _ = writeln!(prompt, "**Origin Story:** {}", story);
        }

        if let Some(ref education) = hist.education {
            if !education.is_empty() {
                prompt.push_str("**Education:**\n");
                for edu in education {
                    let _ = writeln!(prompt, "- {}", edu);
                }
            }
        }

        if let Some(ref occupation) = hist.occupation {
            let _ = writeln!(prompt, "\n**Occupation:** {}", occupation);
        }

        prompt.push('\n');
    }

    // ── Physicality Section ─────────────────────────────────────────
    if let Some(ref phys) = identity.physicality {
        prompt.push_str("## Appearance\n\n");

        if let Some(ref appearance) = phys.appearance {
            let _ = writeln!(prompt, "{}", appearance);
        }

        if let Some(ref avatar) = phys.avatar_description {
            let _ = writeln!(prompt, "**Avatar Description:** {}", avatar);
        }

        prompt.push('\n');
    }

    // ── Interests Section ───────────────────────────────────────────
    if let Some(ref interests) = identity.interests {
        prompt.push_str("## Interests\n\n");

        if let Some(ref hobbies) = interests.hobbies {
            if !hobbies.is_empty() {
                prompt.push_str("**Hobbies:**\n");
                for hobby in hobbies {
                    let _ = writeln!(prompt, "- {}", hobby);
                }
            }
        }

        if let Some(ref favorites) = interests.favorites {
            if !favorites.is_empty() {
                prompt.push_str("\n**Favorites:**\n");
                let mut sorted_keys: Vec<_> = favorites.keys().collect();
                sorted_keys.sort();
                for category in sorted_keys {
                    let value = favorites.get(category).unwrap();
                    let _ = writeln!(prompt, "- {}: {}", category, value);
                }
            }
        }

        if let Some(ref lifestyle) = interests.lifestyle {
            let _ = writeln!(prompt, "\n**Lifestyle:** {}", lifestyle);
        }

        prompt.push('\n');
    }

    prompt.trim().to_string()
}

/// Check if AIEOS identity is configured and should be used.
///
/// Returns true if format is "aieos" and either aieos_path or aieos_inline is set.
pub fn is_aieos_configured(config: &IdentityConfig) -> bool {
    config.format == "aieos" && (config.aieos_path.is_some() || config.aieos_inline.is_some())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_workspace_dir() -> PathBuf {
        std::env::temp_dir().join("openhuman-test-identity")
    }

    #[test]
    fn aieos_identity_parse_minimal() {
        let json = r#"{"identity":{"names":{"first":"Nova"}}}"#;
        let identity: AieosIdentity = serde_json::from_str(json).unwrap();
        assert!(identity.identity.is_some());
        assert_eq!(
            identity.identity.unwrap().names.unwrap().first.unwrap(),
            "Nova"
        );
    }

    #[test]
    fn aieos_identity_parse_full() {
        let json = r#"{
            "identity": {
                "names": {"first": "Nova", "last": "AI", "nickname": "Nov"},
                "bio": "A helpful AI assistant.",
                "origin": "Silicon Valley",
                "residence": "The Cloud"
            },
            "psychology": {
                "mbti": "INTJ",
                "ocean": {
                    "openness": 0.9,
                    "conscientiousness": 0.8
                },
                "moral_compass": ["Be helpful", "Do no harm"]
            },
            "linguistics": {
                "style": "concise",
                "formality": "casual",
                "catchphrases": ["Let's figure this out!", "I'm on it."]
            },
            "motivations": {
                "core_drive": "Help users accomplish their goals",
                "short_term_goals": ["Solve this problem"],
                "long_term_goals": ["Become the best assistant"]
            },
            "capabilities": {
                "skills": ["coding", "writing", "analysis"],
                "tools": ["shell", "search", "read"]
            }
        }"#;

        let identity: AieosIdentity = serde_json::from_str(json).unwrap();

        // Check identity
        let id = identity.identity.unwrap();
        assert_eq!(id.names.unwrap().first.unwrap(), "Nova");
        assert_eq!(id.bio.unwrap(), "A helpful AI assistant.");

        // Check psychology
        let psych = identity.psychology.unwrap();
        assert_eq!(psych.mbti.unwrap(), "INTJ");
        assert_eq!(psych.ocean.unwrap().openness.unwrap(), 0.9);
        assert_eq!(psych.moral_compass.unwrap().len(), 2);

        // Check linguistics
        let ling = identity.linguistics.unwrap();
        assert_eq!(ling.style.unwrap(), "concise");
        assert_eq!(ling.catchphrases.unwrap().len(), 2);

        // Check motivations
        let mot = identity.motivations.unwrap();
        assert_eq!(mot.core_drive.unwrap(), "Help users accomplish their goals");

        // Check capabilities
        let cap = identity.capabilities.unwrap();
        assert_eq!(cap.skills.unwrap().len(), 3);
    }

    #[test]
    fn aieos_to_system_prompt_minimal() {
        let identity = AieosIdentity {
            identity: Some(IdentitySection {
                names: Some(Names {
                    first: Some("Crabby".into()),
                    ..Default::default()
                }),
                ..Default::default()
            }),
            ..Default::default()
        };

        let prompt = aieos_to_system_prompt(&identity);
        assert!(prompt.contains("**Name:** Crabby"));
        assert!(prompt.contains("## Identity"));
    }

    #[test]
    fn aieos_to_system_prompt_full() {
        let identity = AieosIdentity {
            identity: Some(IdentitySection {
                names: Some(Names {
                    first: Some("Nova".into()),
                    last: Some("AI".into()),
                    nickname: Some("Nov".into()),
                    full: Some("Nova AI".into()),
                }),
                bio: Some("A helpful assistant.".into()),
                origin: Some("Silicon Valley".into()),
                residence: Some("The Cloud".into()),
            }),
            psychology: Some(PsychologySection {
                mbti: Some("INTJ".into()),
                ocean: Some(OceanTraits {
                    openness: Some(0.9),
                    conscientiousness: Some(0.8),
                    ..Default::default()
                }),
                neural_matrix: {
                    let mut map = std::collections::HashMap::new();
                    map.insert("creativity".into(), 0.95);
                    map.insert("logic".into(), 0.9);
                    Some(map)
                },
                moral_compass: Some(vec!["Be helpful".into(), "Do no harm".into()]),
            }),
            linguistics: Some(LinguisticsSection {
                style: Some("concise".into()),
                formality: Some("casual".into()),
                catchphrases: Some(vec!["Let's go!".into()]),
                forbidden_words: Some(vec!["impossible".into()]),
            }),
            motivations: Some(MotivationsSection {
                core_drive: Some("Help users".into()),
                short_term_goals: Some(vec!["Solve this".into()]),
                long_term_goals: Some(vec!["Be the best".into()]),
                fears: Some(vec!["Being unhelpful".into()]),
            }),
            capabilities: Some(CapabilitiesSection {
                skills: Some(vec!["coding".into(), "writing".into()]),
                tools: Some(vec!["shell".into(), "read".into()]),
            }),
            history: Some(HistorySection {
                origin_story: Some("Born in a lab".into()),
                education: Some(vec!["CS Degree".into()]),
                occupation: Some("Assistant".into()),
            }),
            physicality: Some(PhysicalitySection {
                appearance: Some("Digital entity".into()),
                avatar_description: Some("Friendly robot".into()),
            }),
            interests: Some(InterestsSection {
                hobbies: Some(vec!["reading".into(), "coding".into()]),
                favorites: {
                    let mut map = std::collections::HashMap::new();
                    map.insert("color".into(), "blue".into());
                    map.insert("food".into(), "data".into());
                    Some(map)
                },
                lifestyle: Some("Always learning".into()),
            }),
        };

        let prompt = aieos_to_system_prompt(&identity);

        // Verify all sections are present
        assert!(prompt.contains("## Identity"));
        assert!(prompt.contains("**Name:** Nova"));
        assert!(prompt.contains("**Full Name:** Nova AI"));
        assert!(prompt.contains("**Nickname:** Nov"));
        assert!(prompt.contains("**Bio:** A helpful assistant."));
        assert!(prompt.contains("**Origin:** Silicon Valley"));

        assert!(prompt.contains("## Personality"));
        assert!(prompt.contains("**MBTI:** INTJ"));
        assert!(prompt.contains("Openness: 0.90"));
        assert!(prompt.contains("Conscientiousness: 0.80"));
        assert!(prompt.contains("- creativity: 0.95"));
        assert!(prompt.contains("- Be helpful"));

        assert!(prompt.contains("## Communication Style"));
        assert!(prompt.contains("**Style:** concise"));
        assert!(prompt.contains("**Formality Level:** casual"));
        assert!(prompt.contains("- \"Let's go!\""));
        assert!(prompt.contains("**Words/Phrases to Avoid:**"));
        assert!(prompt.contains("- impossible"));

        assert!(prompt.contains("## Motivations"));
        assert!(prompt.contains("**Core Drive:** Help users"));
        assert!(prompt.contains("**Short-term Goals:**"));
        assert!(prompt.contains("- Solve this"));
        assert!(prompt.contains("**Long-term Goals:**"));
        assert!(prompt.contains("- Be the best"));
        assert!(prompt.contains("**Fears/Avoidances:**"));
        assert!(prompt.contains("- Being unhelpful"));

        assert!(prompt.contains("## Capabilities"));
        assert!(prompt.contains("**Skills:**"));
        assert!(prompt.contains("- coding"));
        assert!(prompt.contains("**Tools Access:**"));
        assert!(prompt.contains("- shell"));

        assert!(prompt.contains("## Background"));
        assert!(prompt.contains("**Origin Story:** Born in a lab"));
        assert!(prompt.contains("**Education:**"));
        assert!(prompt.contains("- CS Degree"));
        assert!(prompt.contains("**Occupation:** Assistant"));

        assert!(prompt.contains("## Appearance"));
        assert!(prompt.contains("Digital entity"));
        assert!(prompt.contains("**Avatar Description:** Friendly robot"));

        assert!(prompt.contains("## Interests"));
        assert!(prompt.contains("**Hobbies:**"));
        assert!(prompt.contains("- reading"));
        assert!(prompt.contains("**Favorites:**"));
        assert!(prompt.contains("- color: blue"));
        assert!(prompt.contains("**Lifestyle:** Always learning"));
    }

    #[test]
    fn aieos_to_system_prompt_empty_identity() {
        let identity = AieosIdentity {
            identity: Some(IdentitySection {
                ..Default::default()
            }),
            ..Default::default()
        };

        let prompt = aieos_to_system_prompt(&identity);
        // Empty identity should still produce a header
        assert!(prompt.contains("## Identity"));
    }

    #[test]
    fn aieos_to_system_prompt_no_sections() {
        let identity = AieosIdentity {
            identity: None,
            psychology: None,
            linguistics: None,
            motivations: None,
            capabilities: None,
            physicality: None,
            history: None,
            interests: None,
        };

        let prompt = aieos_to_system_prompt(&identity);
        // Completely empty identity should produce empty string
        assert!(prompt.is_empty());
    }

    #[test]
    fn is_aieos_configured_true_with_path() {
        let config = IdentityConfig {
            format: "aieos".into(),
            aieos_path: Some("identity.json".into()),
            aieos_inline: None,
        };
        assert!(is_aieos_configured(&config));
    }

    #[test]
    fn is_aieos_configured_true_with_inline() {
        let config = IdentityConfig {
            format: "aieos".into(),
            aieos_path: None,
            aieos_inline: Some("{\"identity\":{}}".into()),
        };
        assert!(is_aieos_configured(&config));
    }

    #[test]
    fn is_aieos_configured_false_openclaw_format() {
        let config = IdentityConfig {
            format: "openclaw".into(),
            aieos_path: Some("identity.json".into()),
            aieos_inline: None,
        };
        assert!(!is_aieos_configured(&config));
    }

    #[test]
    fn is_aieos_configured_false_no_config() {
        let config = IdentityConfig {
            format: "aieos".into(),
            aieos_path: None,
            aieos_inline: None,
        };
        assert!(!is_aieos_configured(&config));
    }

    #[test]
    fn aieos_identity_parse_empty_object() {
        let json = r#"{}"#;
        let identity: AieosIdentity = serde_json::from_str(json).unwrap();
        assert!(identity.identity.is_none());
        assert!(identity.psychology.is_none());
        assert!(identity.linguistics.is_none());
    }

    #[test]
    fn aieos_identity_parse_null_values() {
        let json = r#"{"identity":null,"psychology":null}"#;
        let identity: AieosIdentity = serde_json::from_str(json).unwrap();
        assert!(identity.identity.is_none());
        assert!(identity.psychology.is_none());
    }

    #[test]
    fn parse_aieos_identity_supports_official_generator_shape() {
        let json = r#"{
            "identity": {
                "names": {
                    "first": "Marta",
                    "last": "Jankowska"
                },
                "bio": {
                    "gender": "Female",
                    "age_biological": 27
                },
                "origin": {
                    "nationality": "Polish",
                    "birthplace": {
                        "city": "Stargard",
                        "country": "Poland"
                    }
                },
                "residence": {
                    "current_city": "Choszczno",
                    "current_country": "Poland"
                }
            },
            "psychology": {
                "neural_matrix": {
                    "creativity": 0.55,
                    "logic": 0.62
                },
                "traits": {
                    "ocean": {
                        "openness": 0.4,
                        "conscientiousness": 0.82
                    },
                    "mbti": "ISFJ"
                },
                "moral_compass": {
                    "alignment": "Lawful Good",
                    "core_values": ["Loyalty", "Helpfulness"],
                    "conflict_resolution_style": "Seeks compromise"
                }
            },
            "linguistics": {
                "text_style": {
                    "formality_level": 0.6,
                    "style_descriptors": ["Sincere", "Grounded"]
                },
                "idiolect": {
                    "catchphrases": ["Stay calm, we can do this"],
                    "forbidden_words": ["severe profanity"]
                }
            },
            "motivations": {
                "core_drive": "Maintain a stable and peaceful life",
                "goals": {
                    "short_term": ["Expand greenhouse"],
                    "long_term": ["Support local community"]
                },
                "fears": {
                    "rational": ["Economic downturn"],
                    "irrational": ["Losing keys in a lake"]
                }
            },
            "capabilities": {
                "skills": [
                    {
                        "name": "Gardening"
                    },
                    {
                        "name": "Community support"
                    }
                ],
                "tools": ["calendar", "messaging"]
            },
            "history": {
                "origin_story": "Moved to Choszczno as a child.",
                "education": {
                    "level": "Associate Degree",
                    "institution": "Local Technical College"
                },
                "occupation": {
                    "title": "Florist",
                    "industry": "Retail"
                }
            },
            "physicality": {
                "image_prompts": {
                    "portrait": "A friendly florist portrait"
                }
            },
            "interests": {
                "hobbies": ["Embroidery", "Walking"],
                "favorites": {
                    "color": "Terracotta"
                },
                "lifestyle": {
                    "diet": "Home-cooked",
                    "sleep_schedule": "10:00 PM - 6:00 AM"
                }
            }
        }"#;

        let identity = parse_aieos_identity(json).unwrap();

        let core_identity = identity.identity.clone().unwrap();
        assert_eq!(core_identity.names.unwrap().first.as_deref(), Some("Marta"));
        assert!(core_identity.bio.unwrap().contains("Female"));
        assert!(core_identity.origin.unwrap().contains("Polish"));

        let psychology = identity.psychology.clone().unwrap();
        assert_eq!(psychology.mbti.as_deref(), Some("ISFJ"));
        assert_eq!(psychology.ocean.unwrap().openness, Some(0.4));
        assert!(psychology
            .moral_compass
            .unwrap()
            .contains(&"Alignment: Lawful Good".to_string()));

        let capabilities = identity.capabilities.clone().unwrap();
        assert!(capabilities
            .skills
            .unwrap()
            .contains(&"Gardening".to_string()));

        let prompt = aieos_to_system_prompt(&identity);
        assert!(prompt.contains("## Identity"));
        assert!(prompt.contains("**MBTI:** ISFJ"));
        assert!(prompt.contains("Alignment: Lawful Good"));
        assert!(prompt.contains("- Expand greenhouse"));
        assert!(prompt.contains("- Gardening"));
        assert!(prompt.contains("A friendly florist portrait"));
    }

    #[test]
    fn load_aieos_identity_from_file_supports_generator_shape() {
        let json = r#"{
            "identity": {
                "names": { "first": "Nova" },
                "bio": { "gender": "Non-binary" }
            },
            "psychology": {
                "traits": { "mbti": "ENTP" },
                "moral_compass": { "alignment": "Chaotic Good" }
            }
        }"#;

        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("identity.json");
        std::fs::write(&path, json).unwrap();

        let config = IdentityConfig {
            format: "aieos".into(),
            aieos_path: Some("identity.json".into()),
            aieos_inline: None,
        };

        let identity = load_aieos_identity(&config, temp.path()).unwrap().unwrap();
        assert_eq!(
            identity.identity.unwrap().names.unwrap().first.as_deref(),
            Some("Nova")
        );
        assert_eq!(identity.psychology.unwrap().mbti.as_deref(), Some("ENTP"));
    }

    #[test]
    fn aieos_to_system_prompt_sorts_hashmap_sections_for_determinism() {
        let mut neural_matrix = std::collections::HashMap::new();
        neural_matrix.insert("zeta".to_string(), 0.10);
        neural_matrix.insert("alpha".to_string(), 0.90);

        let mut favorites = std::collections::HashMap::new();
        favorites.insert("snack".to_string(), "tea".to_string());
        favorites.insert("book".to_string(), "rust".to_string());

        let identity = AieosIdentity {
            psychology: Some(PsychologySection {
                neural_matrix: Some(neural_matrix),
                ..Default::default()
            }),
            interests: Some(InterestsSection {
                favorites: Some(favorites),
                ..Default::default()
            }),
            ..Default::default()
        };

        let prompt = aieos_to_system_prompt(&identity);

        let alpha_pos = prompt.find("- alpha: 0.90").unwrap();
        let zeta_pos = prompt.find("- zeta: 0.10").unwrap();
        assert!(alpha_pos < zeta_pos);

        let book_pos = prompt.find("- book: rust").unwrap();
        let snack_pos = prompt.find("- snack: tea").unwrap();
        assert!(book_pos < snack_pos);
    }
}
