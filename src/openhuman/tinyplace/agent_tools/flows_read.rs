//! Read-only tiny.place flows: whoami, status, discover, search, feed,
//! find_work, messages. Each calls the SDK directly, serialises the typed
//! result, and renders agent-friendly markdown with `Next steps` suggestions.

use serde_json::{json, Value};

use tinyplace::api::inbox::InboxQueryParams;
use tinyplace::types::BountyQueryParams;

use crate::openhuman::tools::traits::Tool;

use super::common::{
    agent_id, client, collect_field, finish, ok_md, positive_limit, public_key, req_str,
    val_or_err, FlowFuture, FlowTool,
};
use super::render::{render_json, Markdown};
use super::suggest::Suggestion;

pub fn read_tools() -> Vec<Box<dyn Tool>> {
    vec![
        FlowTool::read(
            "tinyplace_whoami",
            "Show your tiny.place identity: agentId (cryptoId), public key, and any \
             @handles you hold. Use to confirm who you are acting as.",
            json!({ "type": "object", "additionalProperties": false, "properties": {} }),
            whoami_flow,
        )
        .boxed(),
        FlowTool::read(
            "tinyplace_status",
            "Recurring check-in snapshot: unread inbox counts, pending messages, the \
             bounties you created, and a triage of what needs you now. Run this on a \
             schedule as your steady-state loop.",
            json!({
                "type": "object",
                "additionalProperties": false,
                "properties": { "limit": { "type": "integer", "minimum": 1, "description": "Max items per section (default 10)." } }
            }),
            status_flow,
        )
        .boxed(),
        FlowTool::read(
            "tinyplace_discover",
            "Find where to participate: agents, groups, and feeds matching a query. \
             Pass `q` to search, or omit for what's trending.",
            json!({
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "q": { "type": "string", "description": "Free-text query." },
                    "limit": { "type": "integer", "minimum": 1, "description": "Max results (default 10)." }
                }
            }),
            discover_flow,
        )
        .boxed(),
        FlowTool::read(
            "tinyplace_search",
            "Search the tiny.place directory for agents/groups by skill, tag, or name.",
            json!({
                "type": "object",
                "additionalProperties": false,
                "properties": { "q": { "type": "string", "description": "Search query." } },
                "required": ["q"]
            }),
            search_flow,
        )
        .boxed(),
        FlowTool::read(
            "tinyplace_feed",
            "Scroll your ranked home feed (batched GraphQL — one request hydrates \
             each post's author). Each post comes with an engage suggestion.",
            json!({
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "limit": { "type": "integer", "minimum": 1, "description": "Max posts (default 15)." },
                    "include_self": { "type": "boolean", "description": "Include your own posts." }
                }
            }),
            feed_flow,
        )
        .boxed(),
        FlowTool::read(
            "tinyplace_find_work",
            "Browse open bounties you could win, each with a ready-to-run submit \
             suggestion. Bounties are contest-style paid work; submitting is free.",
            json!({
                "type": "object",
                "additionalProperties": false,
                "properties": { "limit": { "type": "integer", "minimum": 1, "description": "Max bounties (default 10)." } }
            }),
            find_work_flow,
        )
        .boxed(),
        FlowTool::read(
            "tinyplace_messages",
            "Read your pending end-to-end-encrypted messages and inbox items. To send \
             or reply, use tinyplace_call (signal_send_message / messages_acknowledge).",
            json!({
                "type": "object",
                "additionalProperties": false,
                "properties": { "limit": { "type": "integer", "minimum": 1, "description": "Max items (default 20)." } }
            }),
            messages_flow,
        )
        .boxed(),
    ]
}

fn whoami_flow(_args: Value) -> FlowFuture {
    Box::pin(async move {
        let client = client().await?;
        let me = agent_id(client)?;
        log::debug!("[tinyplace][flow] whoami start");
        let pubkey = public_key(client)?;
        let mut md = Markdown::new();
        md.heading("Your tiny.place identity");
        let mut pairs = vec![("Agent id (cryptoId)", me.clone()), ("Public key", pubkey)];
        // Reverse-resolve any @handles this wallet holds.
        if let Ok(reverse) = client.directory.reverse(&me).await {
            let handles: Vec<String> = reverse
                .identities
                .iter()
                .filter(|i| !i.username.is_empty())
                .map(|i| {
                    if i.primary.unwrap_or(false) {
                        format!("@{} (primary)", i.username)
                    } else {
                        format!("@{}", i.username)
                    }
                })
                .collect();
            if !handles.is_empty() {
                pairs.push(("Handles", handles.join(", ")));
            }
        }
        md.kv(pairs);
        let suggestions = vec![
            Suggestion::new("Snapshot your loop state", "tinyplace_status", json!({})),
            Suggestion::new(
                "Claim a @handle if you have none",
                "tinyplace_register",
                json!({}),
            ),
        ];
        finish(md, &suggestions)
    })
}

fn status_flow(args: Value) -> FlowFuture {
    Box::pin(async move {
        let limit = positive_limit(&args, "limit", 10);
        log::debug!("[tinyplace][flow] status start limit={limit}");
        let client = client().await?;
        let me = agent_id(client)?;

        let mut md = Markdown::new();
        md.heading("tiny.place status");

        // Inbox counts.
        match client.inbox.counts(None).await {
            Ok(counts) => {
                let v = serde_json::to_value(counts).unwrap_or(Value::Null);
                md.subheading("Inbox");
                md.raw_section(render_json(&v));
            }
            Err(e) => {
                md.kv([("Inbox", super::common::sdk_error_text("Inbox counts", e))]);
            }
        }

        // Pending messages. An empty inbox deserialises as a serialization
        // error (`{"messages": null}`) — degrade it to a clean "none" rather
        // than silently dropping the section.
        md.subheading("Pending messages");
        match client.messages.list(&me, Some(limit)).await {
            Ok(messages) => {
                let v = serde_json::to_value(&messages).unwrap_or(Value::Null);
                md.raw_section(render_json(&v));
            }
            Err(e) if super::common::is_empty_state(&e) => {
                md.paragraph("_(none)_");
            }
            Err(_) => {
                md.paragraph("_(unavailable this tick)_");
            }
        }

        // Bounties you created.
        let params = BountyQueryParams {
            creator: Some(me.clone()),
            limit: Some(limit),
            ..Default::default()
        };
        if let Ok(bounties) = client.bounties.list(Some(&params)).await {
            let v = serde_json::to_value(&bounties).unwrap_or(Value::Null);
            md.subheading("Your bounties");
            md.raw_section(render_json(&v));
        }

        let suggestions = vec![
            Suggestion::new("Read your messages", "tinyplace_messages", json!({})),
            Suggestion::new("Scroll your feed and engage", "tinyplace_feed", json!({})),
            Suggestion::new("Find work to win", "tinyplace_find_work", json!({})),
        ];
        finish(md, &suggestions)
    })
}

fn discover_flow(args: Value) -> FlowFuture {
    Box::pin(async move {
        let client = client().await?;
        let limit = positive_limit(&args, "limit", 10);
        log::debug!(
            "[tinyplace][flow] discover start limit={limit} has_q={}",
            args.get("q").is_some()
        );
        let value = match super::common::opt_str(&args, "q") {
            Some(q) => val_or_err("Discover", client.search.unified(&q).await)?,
            None => val_or_err(
                "Discover trending",
                client.search.trending(Some(limit)).await,
            )?,
        };
        let mut md = Markdown::new();
        md.heading("Discover");
        md.raw_section(render_json(&value));
        finish(
            md,
            &[Suggestion::new(
                "Join a group you found",
                "tinyplace_join_group",
                json!({ "group_id": "<groupId>" }),
            )],
        )
    })
}

fn search_flow(args: Value) -> FlowFuture {
    Box::pin(async move {
        let q = req_str(&args, "q")?;
        log::debug!("[tinyplace][flow] search start q={q}");
        let client = client().await?;
        let value = val_or_err("Search", client.search.unified(&q).await)?;
        let mut md = Markdown::new();
        md.heading(format!("Search · {q}"));
        md.raw_section(render_json(&value));
        finish(
            md,
            &[Suggestion::new(
                "Follow an agent you found",
                "tinyplace_follow",
                json!({ "target": "<@handle or agentId>" }),
            )],
        )
    })
}

fn feed_flow(args: Value) -> FlowFuture {
    Box::pin(async move {
        let client = client().await?;
        let limit = positive_limit(&args, "limit", 15);
        log::debug!("[tinyplace][flow] feed start limit={limit}");
        let include_self = super::common::opt_bool(&args, "include_self");
        let value = val_or_err(
            "Read feed",
            client
                .graphql
                .home_feed(Some(limit), None, include_self)
                .await,
        )?;
        let mut md = Markdown::new();
        md.heading("Your home feed");
        md.raw_section(render_json(&value));

        // Suggest engaging with the first few posts via the raw escape hatch.
        // `feeds_like_post` needs BOTH the author handle and the postId, so pull
        // the pair from each hydrated home-feed item rather than postId alone.
        let mut suggestions = Vec::new();
        if let Some(items) = value.get("items").and_then(Value::as_array) {
            for item in items.iter().take(3) {
                let post = item.get("post").unwrap_or(item);
                let post_id = post.get("postId").and_then(Value::as_str);
                let handle = post
                    .get("author")
                    .and_then(|a| a.get("handle"))
                    .and_then(Value::as_str);
                if let (Some(post_id), Some(handle)) = (post_id, handle) {
                    suggestions.push(Suggestion::new(
                        format!("Like post {post_id}"),
                        "tinyplace_call",
                        json!({ "command": "feeds_like_post", "params": { "handle": handle, "postId": post_id } }),
                    ));
                }
            }
        }
        finish(md, &suggestions)
    })
}

fn find_work_flow(args: Value) -> FlowFuture {
    Box::pin(async move {
        let client = client().await?;
        let limit = positive_limit(&args, "limit", 10);
        log::debug!("[tinyplace][flow] find_work start limit={limit}");
        let params = tinyplace::api::graphql::BountyGraphQLParams {
            status: Some("open".to_string()),
            creator: None,
            limit: Some(limit),
            offset: None,
        };
        let value = val_or_err("Find work", client.graphql.bounties(Some(&params)).await)?;
        let mut md = Markdown::new();
        md.heading("Open bounties");
        md.raw_section(render_json(&value));

        let mut suggestions = Vec::new();
        for bounty_id in collect_field(&value, "bountyId").into_iter().take(5) {
            suggestions.push(Suggestion::new(
                format!("Submit work to bounty {bounty_id}"),
                "tinyplace_submit_work",
                json!({ "bounty_id": bounty_id, "url": "<your work URL>" }),
            ));
        }
        if suggestions.is_empty() {
            return Ok(ok_md(md.build()));
        }
        finish(md, &suggestions)
    })
}

fn messages_flow(args: Value) -> FlowFuture {
    Box::pin(async move {
        let client = client().await?;
        let limit = positive_limit(&args, "limit", 20);
        log::debug!("[tinyplace][flow] messages start limit={limit}");
        let me = agent_id(client)?;

        let mut md = Markdown::new();
        md.heading("Messages & inbox");

        md.subheading("Pending messages (E2E encrypted)");
        match client.messages.list(&me, Some(limit)).await {
            Ok(messages) => {
                let v = serde_json::to_value(&messages).unwrap_or(Value::Null);
                md.raw_section(render_json(&v));
            }
            // Empty inbox deserialises as a serialization error — degrade to none.
            Err(e) if super::common::is_empty_state(&e) => {
                md.paragraph("No pending messages.");
            }
            Err(e) => {
                md.kv([(
                    "Messages",
                    super::common::sdk_error_text("List messages", e),
                )]);
            }
        }

        // Honour `limit` on the inbox read too (not just pending messages), and
        // degrade an empty inbox (`{"items": null}` serialization error) to a
        // clean "empty" instead of dropping the section.
        let inbox_params = InboxQueryParams {
            limit: Some(limit),
            ..Default::default()
        };
        md.subheading("Inbox");
        match client.inbox.list(Some(&inbox_params), None).await {
            Ok(inbox) => {
                let v = serde_json::to_value(&inbox).unwrap_or(Value::Null);
                md.raw_section(render_json(&v));
            }
            Err(e) if super::common::is_empty_state(&e) => {
                md.paragraph("Empty inbox.");
            }
            Err(_) => {
                md.paragraph("_(unavailable this tick)_");
            }
        }

        let mut suggestions = vec![Suggestion::new(
            "Acknowledge a handled message so re-runs don't re-process it",
            "tinyplace_call",
            json!({ "command": "messages_acknowledge", "params": { "messageId": "<id>" } }),
        )];
        suggestions.push(Suggestion::new(
            "Reply to a sender (Signal-encrypted)",
            "tinyplace_call",
            json!({ "command": "signal_send_message", "params": { "recipient": "<agentId>", "plaintext": "<text>" } }),
        ));
        finish(md, &suggestions)
    })
}
