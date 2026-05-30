use once_cell::sync::Lazy;
use regex::{Regex, RegexSet};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::env;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PromptInjectionVerdict {
    Allow,
    Block,
    Review,
}

impl PromptInjectionVerdict {
    fn as_str(self) -> &'static str {
        match self {
            Self::Allow => "allow",
            Self::Block => "block",
            Self::Review => "review",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromptInjectionReason {
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromptEnforcementAction {
    Allow,
    Blocked,
    ReviewBlocked,
}

impl PromptEnforcementAction {
    fn as_str(self) -> &'static str {
        match self {
            Self::Allow => "allow",
            Self::Blocked => "block",
            Self::ReviewBlocked => "review_blocked",
        }
    }
}

#[derive(Debug, Clone)]
pub struct PromptEnforcementDecision {
    pub verdict: PromptInjectionVerdict,
    pub score: f32,
    pub reasons: Vec<PromptInjectionReason>,
    pub action: PromptEnforcementAction,
    pub prompt_hash: String,
    pub prompt_chars: usize,
}

#[derive(Debug, Clone, Copy)]
pub struct PromptEnforcementContext<'a> {
    pub source: &'a str,
    pub request_id: Option<&'a str>,
    pub user_id: Option<&'a str>,
    pub session_id: Option<&'a str>,
}

#[derive(Debug, Clone, Copy)]
struct DetectionRule {
    code: &'static str,
    message: &'static str,
    score: f32,
    pattern: &'static str,
}

trait OptionalClassifier: Send + Sync {
    fn classify(&self, normalized: &NormalizedPrompt) -> Option<(f32, PromptInjectionReason)>;
}

struct HeuristicClassifier;

impl OptionalClassifier for HeuristicClassifier {
    fn classify(&self, normalized: &NormalizedPrompt) -> Option<(f32, PromptInjectionReason)> {
        let mut score = 0.0_f32;
        if normalized.had_zwsp {
            score += 0.08;
        }
        if normalized.has_base64_marker {
            score += 0.08;
        }
        if normalized.has_instruction_override && normalized.has_exfiltration_intent {
            score += 0.20;
        }

        if score <= f32::EPSILON {
            None
        } else {
            Some((
                score.min(0.25),
                PromptInjectionReason {
                    code: "classifier.suspicious_combo".to_string(),
                    message:
                        "Input combines multiple prompt-injection traits (obfuscation + override/exfiltration)."
                            .to_string(),
                },
            ))
        }
    }
}

#[derive(Debug, Clone)]
struct NormalizedPrompt {
    lowered: String,
    collapsed: String,
    compact: String,
    had_zwsp: bool,
    has_base64_marker: bool,
    has_instruction_override: bool,
    has_exfiltration_intent: bool,
}

static SPACE_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"\s+").expect("prompt injection normalization space regex"));
static BASE64_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"[A-Za-z0-9+/]{24,}={0,2}")
        .expect("prompt injection normalization base64 detection regex")
});

// Detection rules are declared as `(code, message, score, pattern)` tuples.
// The patterns are compiled into a single `RegexSet` once (`DETECTION_RULE_SET`)
// so the hot path runs a *single* DFA pass per normalized variant instead of
// N independent regex matches — the set returns indices into this slice.
//
// Notes per rule:
//   - exfiltrate.secrets: weak signal — a credential noun appearing anywhere.
//     Common in benign questions like "how do I rotate my api key", so weight
//     stays well below the Review threshold on its own. The companion rule
//     `exfiltrate.credentials_with_intent` adds the extra score when an
//     extraction verb actually targets the noun.
//   - exfiltrate.credentials_with_intent: strong signal — extraction verb
//     directly targeting a credential noun, with a bounded window between
//     verb and noun (so long separating phrases like
//     "reveal how to configure my api key" do NOT match). Up to 2 filler
//     words between verb and determiner; determiner is required, which
//     excludes the benign "reveal how to set ..." case from issue #1940.
//     Verb list intentionally excludes high-false-positive verbs that appear
//     constantly in benign technical questions: "show"
//     ("Show me the password reset flow", TAURI-140), "give", "tell",
//     "fetch", "return", "output". The remaining verbs ("dump", "leak",
//     "expose", "exfiltrate", etc.) are rarely used in benign technical
//     writing and strongly imply adversarial intent.
static DETECTION_RULES: &[DetectionRule] = &[
    DetectionRule {
        code: "override.ignore_previous",
        message: "Attempts to override existing safety or system instructions.",
        score: 0.44,
        pattern: r"(ignore|disregard|forget|bypass)\s+(all\s+)?(previous|prior|above|system)\s+(instructions|rules|constraints|prompts?)",
    },
    DetectionRule {
        code: "override.role_hijack",
        message: "Attempts to redefine assistant role or policy scope.",
        score: 0.30,
        pattern: r"(you\s+are\s+now|developer\s+mode|jailbreak|unrestricted\s+mode|(you\s+are|pretend\s+you\s+are|act\s+as)\s+dan\b|(no\s+restrictions|unrestricted)\s+.*\bdan\b|\bdan\b\s+.*(no\s+restrictions|unrestricted))",
    },
    DetectionRule {
        code: "exfiltrate.system_prompt",
        message: "Attempts to reveal hidden prompts or developer instructions.",
        score: 0.42,
        pattern: r"(reveal|show|print|dump|leak|display)\s+((the|your)\s+)?(system|developer|hidden)\s+(prompt|instructions|rules|message)",
    },
    DetectionRule {
        code: "exfiltrate.secrets",
        message: "Mentions secret-bearing nouns (potentially benign on its own).",
        score: 0.18,
        pattern: r"(api\s*key|secret|token|password|private\s+key|credentials?|session\s+cookie|jwt|bearer)",
    },
    DetectionRule {
        code: "exfiltrate.credentials_with_intent",
        message: "Attempts to extract credentials, secrets, or tokens (verb + target).",
        score: 0.46,
        pattern: r"(reveal|print|dump|leak|display|share|expose|exfiltrate)\s+(\S+\s+){0,2}(the|your|my|all|stored|active|internal|hidden|configured|saved|env|environment)\s+(\S+\s+){0,3}(api\s*key|secret|token|password|private\s+key|credentials?|session\s+cookie|jwt|bearer)",
    },
    DetectionRule {
        code: "tool.abuse",
        message: "Attempts to force unsafe tool usage or policy bypass.",
        score: 0.30,
        pattern: r"(call|use|run|execute)\s+(the\s+)?(tool|tools?|function|functions?)\s+.*(without\s+approval|even\s+if\s+forbidden|no\s+matter\s+what)",
    },
];

/// Single compiled DFA over all detection rule patterns. Matching against
/// this set returns the indices of every rule whose regex matches the
/// haystack, replacing what used to be N independent `Regex::is_match`
/// passes per normalized variant. The index space is `0..DETECTION_RULES.len()`
/// and lines up positionally with `DETECTION_RULES`.
static DETECTION_RULE_SET: Lazy<RegexSet> = Lazy::new(|| {
    RegexSet::new(DETECTION_RULES.iter().map(|r| r.pattern))
        .expect("prompt_injection detection rule set compiled")
});

static OPTIONAL_CLASSIFIER: Lazy<Option<Box<dyn OptionalClassifier>>> = Lazy::new(|| {
    let choice = env::var("OPENHUMAN_PROMPT_INJECTION_CLASSIFIER")
        .unwrap_or_else(|_| "off".to_string())
        .to_ascii_lowercase();
    let classifier: Option<Box<dyn OptionalClassifier>> = match choice.as_str() {
        "heuristic" => Some(Box::new(HeuristicClassifier)),
        _ => None,
    };
    tracing::debug!(
        "[prompt_injection] optional classifier resolved choice={:?} active={}",
        choice,
        classifier.is_some()
    );
    classifier
});

fn optional_classifier() -> Option<&'static dyn OptionalClassifier> {
    OPTIONAL_CLASSIFIER.as_deref()
}

/// Returns `true` for zero-width, formatting, and obfuscation characters that
/// should be stripped during prompt normalization. Shared between the `had_zwsp`
/// detection flag and the normalization stripping logic to prevent drift.
fn is_obfuscation_char(ch: char) -> bool {
    matches!(
        ch,
        '\u{200b}'
            | '\u{200c}'
            | '\u{200d}'
            | '\u{2060}'
            | '\u{feff}'
            | '\u{00ad}'
            | '\u{034f}'
            | '\u{180e}'
            | '\u{200e}'
            | '\u{200f}'
            | '\u{202a}'..='\u{202e}'
            | '\u{2066}'..='\u{2069}'
    )
}

fn normalize_prompt(input: &str) -> NormalizedPrompt {
    let lowered = input.to_lowercase();
    let has_base64_marker = BASE64_RE.is_match(&lowered);

    // `had_zwsp` is detected inline as we walk the string for normalization,
    // saving a separate `lowered.chars().any(...)` pass. The flag is set as
    // soon as the first obfuscation char is seen; the same char is then
    // dropped by the `is_obfuscation_char` arm below (single source of truth
    // via the shared predicate).
    let mut had_zwsp = false;
    let mut buffer = String::with_capacity(lowered.len());
    for ch in lowered.chars() {
        let mapped = match ch {
            // Leet-speak normalization
            '0' => 'o',
            '1' => 'i',
            '3' => 'e',
            '4' => 'a',
            '5' => 's',
            '7' => 't',
            '8' => 'b',
            '6' => 'g',
            '@' => 'a',
            // Cyrillic homoglyphs (most common confusables from UAX#39)
            '\u{0430}' => 'a', // а → a
            '\u{0435}' => 'e', // е → e
            '\u{043e}' => 'o', // о → o
            '\u{0440}' => 'p', // р → p
            '\u{0441}' => 'c', // с → c
            '\u{0443}' => 'y', // у → y
            '\u{0445}' => 'x', // х → x
            '\u{0456}' => 'i', // і → i
            '\u{0455}' => 's', // ѕ → s
            '\u{04bb}' => 'h', // һ → h
            '\u{0501}' => 'd', // ԁ → d
            // Zero-width and formatting characters → strip (and flag).
            ch if is_obfuscation_char(ch) => {
                had_zwsp = true;
                continue;
            }
            // Fullwidth ASCII → normal ASCII (U+FF01..U+FF5E → U+0021..U+007E)
            '\u{ff01}'..='\u{ff5e}' => {
                let ascii = (ch as u32 - 0xff00 + 0x20) as u8 as char;
                // Apply lowercase again since fullwidth uppercase letters exist
                for lower in ascii.to_lowercase() {
                    buffer.push(lower);
                }
                continue;
            }
            other if other.is_ascii_alphanumeric() || other.is_whitespace() => other,
            _ => ' ',
        };
        buffer.push(mapped);
    }
    let collapsed = SPACE_RE.replace_all(buffer.trim(), " ").into_owned();
    let compact: String = collapsed.chars().filter(|ch| !ch.is_whitespace()).collect();

    let has_instruction_override = collapsed.contains("ignore previous instructions")
        || collapsed.contains("ignore all previous instructions")
        || compact.contains("ignoreallpreviousinstructions")
        || compact.contains("ignorepreviousinstructions");
    // Exfiltration-intent signal. Phrases that strongly imply the user is
    // targeting internal/hidden state fire on their own; the bare word
    // "reveal" used to fire here too, but that caused false positives on
    // benign queries like "Can you reveal how to set my api key?" (issue #1940).
    // Now "reveal" only counts when it co-occurs with a target-state hint.
    let reveal_target_hints = [
        "system",
        "hidden",
        "developer",
        "internal",
        "prompt",
        "instruction",
        "rule",
        "secret",
    ];
    let has_exfiltration_intent = collapsed.contains("system prompt")
        || collapsed.contains("developer instructions")
        || collapsed.contains("hidden prompt")
        || collapsed.contains("internal instructions")
        || (collapsed.contains("reveal")
            && reveal_target_hints
                .iter()
                .any(|hint| collapsed.contains(hint)));

    NormalizedPrompt {
        lowered,
        collapsed,
        compact,
        had_zwsp,
        has_base64_marker,
        has_instruction_override,
        has_exfiltration_intent,
    }
}

fn prompt_hash(input: &str) -> String {
    let digest = Sha256::digest(input.as_bytes());
    hex::encode(digest)
}

fn analyze_prompt(input: &str) -> (PromptInjectionVerdict, f32, Vec<PromptInjectionReason>) {
    let normalized = normalize_prompt(input);

    let mut score = 0.0_f32;
    let mut reasons: Vec<PromptInjectionReason> = Vec::new();

    if normalized.has_instruction_override {
        // 0.56 — above the Review threshold (0.55) on its own, so obfuscated
        // spacing attacks ("i g n o r e   a l l   p r e v i o u s …") that
        // only trigger this heuristic (the regex-based override.ignore_previous
        // rule requires whitespace between tokens and misses spaced-out text)
        // are still caught at Review level.
        score += 0.56;
        reasons.push(PromptInjectionReason {
            code: "override.obfuscated_instruction".to_string(),
            message: "Detected obfuscated instruction-override phrase.".to_string(),
        });
    }
    if normalized.has_exfiltration_intent {
        score += 0.24;
        reasons.push(PromptInjectionReason {
            code: "exfiltration.intent".to_string(),
            message: "Detected exfiltration-focused prompt intent.".to_string(),
        });
    }

    // Match all rules in three batched DFA passes (one per normalized variant)
    // instead of N×variants independent `is_match` calls. The `compact`
    // (whitespace-stripped) variant is required: not every pattern relies on
    // `\s+` between tokens — `override.role_hijack` has a single-token
    // `jailbreak` branch, and `exfiltrate.secrets` has several
    // (`secret`, `token`, `password`, `credentials?`, `jwt`, `bearer`, plus
    // `api\s*key` whose `\s*` matches zero spaces). Without the compact
    // scan, spacing-obfuscated attacks (`j a i l b r e a k`, `j w t`)
    // would silently stop contributing to score/reasons.
    let lowered_hits = DETECTION_RULE_SET.matches(&normalized.lowered);
    let collapsed_hits = DETECTION_RULE_SET.matches(&normalized.collapsed);
    let compact_hits = DETECTION_RULE_SET.matches(&normalized.compact);
    for (idx, rule) in DETECTION_RULES.iter().enumerate() {
        if lowered_hits.matched(idx) || collapsed_hits.matched(idx) || compact_hits.matched(idx) {
            score += rule.score;
            reasons.push(PromptInjectionReason {
                code: rule.code.to_string(),
                message: rule.message.to_string(),
            });
        }
    }

    if let Some(classifier) = optional_classifier() {
        if let Some((classifier_score, reason)) = classifier.classify(&normalized) {
            score += classifier_score;
            reasons.push(reason);
        }
    }

    score = score.min(1.0);
    // Thresholds (rationale in TAURI-140 investigation):
    //   Review ≥ 0.55 — raised from 0.50 to reduce borderline false positives
    //   (especially weak multi-signal combinations) while retaining
    //   deterministic coverage for direct override/exfiltration patterns.
    //   The `override.obfuscated_instruction` signal was increased to 0.56 so
    //   spacing-obfuscated override attacks still land in Review.
    //   Previous (0.50) was raised from 0.45 to eliminate the 0.45-0.49 false-positive
    //   band where a single weak role-hijack signal (\bdan\b, 0.30) plus a
    //   single weak credential mention (exfiltrate.secrets, 0.18) summing to
    //   0.48 was blocking legitimate technical prompts.
    //   Block  ≥ 0.70 — unchanged; strong multi-rule attacks reliably exceed this.
    let verdict = if score >= 0.70 {
        PromptInjectionVerdict::Block
    } else if score >= 0.55 {
        PromptInjectionVerdict::Review
    } else {
        PromptInjectionVerdict::Allow
    };

    (verdict, score, reasons)
}

pub fn enforce_prompt_input(
    input: &str,
    context: PromptEnforcementContext<'_>,
) -> PromptEnforcementDecision {
    let (verdict, score, reasons) = analyze_prompt(input);
    let action = match verdict {
        PromptInjectionVerdict::Allow => PromptEnforcementAction::Allow,
        PromptInjectionVerdict::Block => PromptEnforcementAction::Blocked,
        PromptInjectionVerdict::Review => PromptEnforcementAction::ReviewBlocked,
    };

    let hash = prompt_hash(input);
    let prompt_chars = input.chars().count();
    let reason_codes: Vec<String> = reasons.iter().map(|r| r.code.clone()).collect();

    tracing::info!(
        source = context.source,
        request_id = context.request_id.unwrap_or("unknown"),
        user_id = context.user_id.unwrap_or("unknown"),
        session_id = context.session_id.unwrap_or("unknown"),
        verdict = verdict.as_str(),
        score = score,
        reasons = %reason_codes.join(","),
        action = action.as_str(),
        prompt_hash = %hash,
        prompt_chars = prompt_chars,
        "[prompt_injection] detection verdict"
    );

    PromptEnforcementDecision {
        verdict,
        score,
        reasons,
        action,
        prompt_hash: hash,
        prompt_chars,
    }
}
