use super::*;

/// Messages that ask about the user: the gate must open on every one.
const OPENS: &[&str] = &[
    "who is my idol and why?",
    "what's my favourite colour",
    "What is my favorite cricketer?",
    "why do I admire virat?",
    "tell me my reason for choosing virat as idol",
    "remind me what my sister's name is",
    "do you remember what I told you about my trip?",
    "which web series do I like the most?",
    "what did I say my goals were?",
    "how do I usually take my coffee?",
    "where do I work?",
    "when is my birthday?",
    "what do I like to eat",
    "Do I have any allergies?",
    "can you tell me what my timezone is?",
    "what are my hobbies?",
    "recall what I said about my manager",
    "what's my dog's name?",
    "am I a morning person?",
    "who are my closest friends?",
    "what did i tell you about my startup",
    "Which city do I live in?",
    "how many kids do I have?",
    "what languages do I speak?",
    "what is my role at work?",
    "did I mention my favourite book?",
    "what\u{2019}s my idol\u{2019}s best quality?",
    "who did I say I look up to?",
    "is my favourite colour black?",
    "describe my ideal weekend based on what I've shared",
    "have I told you about my parents?",
    "list my favourite movies",
    "please remind me what my sister's name is",
    "Please tell me my favorite color",
    "hey, what's my idol's name",
    "ok so what do I usually order at cafés",
    "yo bro, can you tell me what my timezone is",
    "quick question, do I have any allergies",
];

/// Messages that are not about the user: the gate must stay closed.
const CLOSES: &[(&str, &str)] = &[
    ("what's the weather today", "no_first_person"),
    ("fix my code, it throws on line 12", "not_a_question"),
    ("summarise this article", "no_first_person"),
    ("thanks!", "no_first_person"),
    ("ok", "no_first_person"),
    ("write a haiku about autumn", "no_first_person"),
    ("what is the capital of france?", "no_first_person"),
    ("translate this to spanish: hello", "no_first_person"),
    ("how does tokio spawn work?", "no_first_person"),
    ("explain rust lifetimes", "no_first_person"),
    (
        "```rust\nfn main() {}\n```\nwhat does my code do?",
        "code_block",
    ),
    ("run the tests", "no_first_person"),
    ("who won the 2024 world cup?", "no_first_person"),
    ("generate an image of a cat", "no_first_person"),
    ("convert 10 miles to km", "no_first_person"),
    ("what time is it in tokyo?", "no_first_person"),
    ("give me a summary of the meeting notes", "not_a_question"),
    ("send my email to the team", "not_a_question"),
    ("open my calendar", "not_a_question"),
    ("search the web for rust async patterns", "no_first_person"),
    ("hello", "no_first_person"),
    ("why is the sky blue?", "no_first_person"),
    ("let's plan the sprint", "no_first_person"),
    ("i think we should refactor this module", "not_a_question"),
    ("I'll be back in 10 minutes", "not_a_question"),
    ("how are you?", "no_first_person"),
    ("what's 2+2?", "no_first_person"),
    ("does this function handle null?", "no_first_person"),
    ("deploy my branch to staging", "not_a_question"),
    ("please fix my code", "not_a_question"),
    ("hey, please summarise this thread", "no_first_person"),
    ("", "empty"),
    ("   \n  ", "empty"),
];

#[test]
fn opens_on_every_about_me_question() {
    let missed: Vec<&str> = OPENS
        .iter()
        .copied()
        .filter(|message| !gate_decision(message).is_open())
        .collect();
    assert!(
        missed.is_empty(),
        "gate stayed closed on about-me messages: {missed:?}"
    );
    assert!(OPENS.len() >= 30, "fixture must stay representative");
}

#[test]
fn closes_on_every_other_message_with_the_expected_reason() {
    let wrong: Vec<(&str, &str, GateDecision)> = CLOSES
        .iter()
        .copied()
        .filter_map(|(message, reason)| {
            let decision = gate_decision(message);
            (decision != GateDecision::Closed(reason)).then_some((message, reason, decision))
        })
        .collect();
    assert!(wrong.is_empty(), "unexpected gate decisions: {wrong:?}");
    assert!(CLOSES.len() >= 30, "fixture must stay representative");
}

#[test]
fn precision_and_recall_clear_the_bar() {
    // The acceptance bar is ≥ 90 % both ways; the fixtures above pin 100 %,
    // and this guards the bar itself if someone loosens an individual case.
    let recall = OPENS
        .iter()
        .filter(|message| gate_decision(message).is_open())
        .count() as f64
        / OPENS.len() as f64;
    let precision = CLOSES
        .iter()
        .filter(|(message, _)| !gate_decision(message).is_open())
        .count() as f64
        / CLOSES.len() as f64;
    assert!(recall >= 0.9, "recall {recall}");
    assert!(precision >= 0.9, "precision {precision}");
}

#[test]
fn long_messages_are_pastes_not_questions() {
    let long = format!("what is my {}?", "very ".repeat(200));
    assert_eq!(gate_decision(&long), GateDecision::Closed("too_long"));
}

#[test]
fn curly_apostrophes_do_not_hide_the_lead_word() {
    assert_eq!(
        gate_decision("what\u{2019}s my idol\u{2019}s name"),
        GateDecision::Open("lexical")
    );
}

#[test]
fn reason_and_is_open_agree() {
    let open = GateDecision::Open("lexical");
    let closed = GateDecision::Closed("empty");
    assert!(open.is_open());
    assert!(!closed.is_open());
    assert_eq!(open.reason(), "lexical");
    assert_eq!(closed.reason(), "empty");
}

#[test]
fn words_are_lowercased_and_stripped_of_apostrophes() {
    assert_eq!(
        words_of("I've Got My idol\u{2019}s NAME"),
        vec!["ive", "got", "my", "idols", "name"]
    );
}
