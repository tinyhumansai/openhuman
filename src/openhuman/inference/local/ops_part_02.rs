
/// Evaluates whether the assistant should add an emoji reaction to a user message.
///
/// This uses the local model to make a quick decision based on the message
/// content and the channel context.
pub async fn local_ai_should_react(
    config: &Config,
    message: &str,
    channel_type: &str,
) -> Result<RpcOutcome<ReactionDecision>, String> {
    tracing::debug!(
        channel_type,
        msg_len = message.len(),
        "[local_ai:should_react] evaluating reaction"
    );

    if message.trim().is_empty() {
        return Ok(RpcOutcome::single_log(
            ReactionDecision {
                should_react: false,
                emoji: None,
            },
            "empty message — no reaction",
        ));
    }

    let service = local_ai::global(config);
    let status = service.status();
    if !matches!(status.state.as_str(), "ready") {
        tracing::debug!("[local_ai:should_react] local model not ready, skipping");
        return Ok(RpcOutcome::single_log(
            ReactionDecision {
                should_react: false,
                emoji: None,
            },
            "local model not ready",
        ));
    }

    let prompt = format!(
        "You decide whether an AI assistant should react to a user message with a single emoji. \
         Consider the channel context: casual channels (discord, telegram) get more frequent \
         reactions with playful emojis, while professional channels (web, slack, email) are more \
         reserved — only react to clearly emotional or noteworthy messages.\n\n\
         Channel: {channel_type}\nUser message: {message}\n\n\
         Reply with EXACTLY one word: either NONE (no reaction) or a single emoji character."
    );

    let output = service.prompt(config, &prompt, Some(8), true).await;

    let decision = match output {
        Ok(raw) => {
            let trimmed = raw.trim();
            tracing::debug!(
                output_len = trimmed.len(),
                "[local_ai:should_react] model response"
            );
            if trimmed.eq_ignore_ascii_case("NONE") || trimmed.is_empty() {
                ReactionDecision {
                    should_react: false,
                    emoji: None,
                }
            } else {
                // Extract the first emoji-like character(s) from the response
                let emoji = extract_first_emoji(trimmed);
                match emoji {
                    Some(e) => ReactionDecision {
                        should_react: true,
                        emoji: Some(e),
                    },
                    None => ReactionDecision {
                        should_react: false,
                        emoji: None,
                    },
                }
            }
        }
        Err(e) => {
            tracing::debug!(error = %e, "[local_ai:should_react] inference failed, skipping");
            ReactionDecision {
                should_react: false,
                emoji: None,
            }
        }
    };

    tracing::debug!(
        should_react = decision.should_react,
        emoji = ?decision.emoji,
        "[local_ai:should_react] decision"
    );
    Ok(RpcOutcome::single_log(
        decision,
        "reaction decision completed",
    ))
}

/// Extract the first emoji from a string. Handles common emoji codepoints
/// including flag sequences (pairs of regional indicator symbols).
fn extract_first_emoji(text: &str) -> Option<String> {
    let mut chars = text.chars();
    while let Some(ch) = chars.next() {
        // Regional indicator pair → flag emoji (e.g. 🇺🇸 = U+1F1FA U+1F1F8)
        if is_regional_indicator(ch) {
            let mut emoji = String::new();
            emoji.push(ch);
            // Consume consecutive regional indicators (flags are pairs)
            for next in chars.by_ref() {
                if is_regional_indicator(next) {
                    emoji.push(next);
                } else {
                    break;
                }
            }
            return Some(emoji);
        }

        if is_emoji_start(ch) {
            let mut emoji = String::new();
            emoji.push(ch);
            // Consume joiners and variation selectors that extend the emoji
            for next in chars.by_ref() {
                if next == '\u{FE0F}'     // variation selector
                    || next == '\u{200D}'  // zero-width joiner
                    || ('\u{1F3FB}'..='\u{1F3FF}').contains(&next) // skin tones
                    || is_emoji_start(next) && emoji.contains('\u{200D}')
                {
                    emoji.push(next);
                } else {
                    break;
                }
            }
            return Some(emoji);
        }
    }
    None
}

fn is_regional_indicator(ch: char) -> bool {
    ('\u{1F1E6}'..='\u{1F1FF}').contains(&ch)
}

fn is_emoji_start(ch: char) -> bool {
    matches!(ch,
        '\u{203C}' | '\u{2049}'       // exclamation marks
        | '\u{2139}'                   // information
        | '\u{2194}'..='\u{2199}'      // arrows
        | '\u{21A9}'..='\u{21AA}'      // arrows
        | '\u{231A}'..='\u{231B}'      // watch, hourglass
        | '\u{23E9}'..='\u{23F3}'      // media controls
        | '\u{23F8}'..='\u{23FA}'      // media controls
        | '\u{24C2}'                   // circled M
        | '\u{25AA}'..='\u{25AB}'      // squares
        | '\u{25B6}' | '\u{25C0}'     // play buttons
        | '\u{25FB}'..='\u{25FE}'      // squares
        | '\u{2328}' | '\u{23CF}'     // keyboard, eject
        | '\u{2600}'..='\u{27BF}'      // misc symbols, dingbats
        | '\u{2934}'..='\u{2935}'      // arrows
        | '\u{2B05}'..='\u{2B07}'      // arrows
        | '\u{2B1B}'..='\u{2B1C}'      // squares
        | '\u{2B50}' | '\u{2B55}'     // star, circle
        | '\u{FE00}'..='\u{FE0F}'      // variation selectors
        | '\u{1F300}'..='\u{1F9FF}'    // misc symbols, emoticons, transport, supplemental
        | '\u{1FA00}'..='\u{1FA6F}'    // chess symbols, extended-A
        | '\u{1FA70}'..='\u{1FAFF}'    // symbols extended-A
        | '\u{200D}'                   // ZWJ
    )
}
