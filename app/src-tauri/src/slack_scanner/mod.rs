//! Slack Web scanner driven purely over the Chrome DevTools Protocol (CDP).
//!
//! Attaches to the embedded CEF webview via the in-process CDP transport
//! installed by `webview_accounts::open` (no TCP listener). One polling
//! loop per tracked Slack account:
//!
//!   * **IDB tick** (`IDB_SCAN_INTERVAL`, 30s) — walks every Slack-owned
//!     IndexedDB database via CDP (`IndexedDB.requestDatabaseNames`,
//!     `IndexedDB.requestDatabase`, `IndexedDB.requestData`), materialises
//!     `Runtime.RemoteObject` records into JSON with a fixed, Slack-agnostic
//!     serializer (`function(){return [this].concat(arguments);}`), and
//!     recursively extracts message / user / channel records from the
//!     Redux-persist snapshots Slack stores there. No in-page JavaScript
//!     runs beyond that one fixed serializer, and no DOM scraping.
//!
//! Emits `webview:event` ingest events (for any listening React UI) AND
//! POSTs `openhuman.memory_doc_ingest` directly to the core so memory is
//! populated whether or not the main window is open. Messages are grouped
//! by `channel_id` (one doc per channel; the transcript carries each
//! message's date inline so chronology stays readable). Per-day grouping
//! was specified for #1016 but is deferred — see #1016 follow-ups.
//!
//! Only built with the `cef` feature — wry has no remote-debugging port.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex;
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter, Runtime};
use tokio::task::AbortHandle;
use tokio::time::sleep;

mod dom_snapshot;
mod extract;
mod idb;

/// How often we walk IDB. Tune down for faster iteration during dev; the
/// walk itself is bounded by per-store record caps in `idb.rs`.
const IDB_SCAN_INTERVAL: Duration = Duration::from_secs(30);

/// Spawn a per-account CDP poller. Caller is expected to guard against
/// double-spawning via `ScannerRegistry`.
pub fn spawn_scanner<R: Runtime>(
    app: AppHandle<R>,
    account_id: String,
    url_prefix: String,
) -> Vec<AbortHandle> {
    let mut handles = Vec::with_capacity(2);
    handles.push(spawn_dom_poll(
        app.clone(),
        account_id.clone(),
        url_prefix.clone(),
    ));
    let task = tokio::spawn(async move {
        let fragment = crate::cdp::target_url_fragment(&account_id);
        log::info!(
            "[sl] scanner up account={} url_prefix={} fragment={} interval={:?}",
            account_id,
            url_prefix,
            fragment,
            IDB_SCAN_INTERVAL,
        );
        // Let Slack hydrate Redux from IDB before the first scan —
        // otherwise we'd race an empty store on cold start.
        sleep(Duration::from_secs(10)).await;

        // Account-stable target identifier discovered on the first tick
        // where the strict `#openhuman-account-<id>` fragment is still
        // present. Once set, subsequent ticks resolve the page target
        // by this id first so the relaxed same-origin fallback can
        // never bind us to a sibling Slack account's page in a
        // multi-account session (CodeRabbit #3162652711).
        let mut pinned_target_id: Option<String> = None;
        loop {
            match scan_once(
                &app,
                &account_id,
                &url_prefix,
                &fragment,
                &mut pinned_target_id,
            )
            .await
            {
                Ok(dump) => {
                    let team_id = infer_team_id(&dump);
                    let (messages, users, channels, workspace_name) = extract::harvest(&dump);
                    log::info!(
                        "[sl][{}] idb extract: {} msgs, {} users, {} channels, team={} workspace={}",
                        account_id,
                        messages.len(),
                        users.len(),
                        channels.len(),
                        team_id.as_deref().unwrap_or("?"),
                        workspace_name.as_deref().unwrap_or("?"),
                    );
                    if !messages.is_empty() {
                        emit_and_persist(
                            &app,
                            &account_id,
                            &messages,
                            &users,
                            &channels,
                            team_id.as_deref().unwrap_or(""),
                            workspace_name.as_deref().unwrap_or(""),
                        );
                    }
                }
                Err(e) => {
                    log::warn!("[sl][{}] idb scan failed: {}", account_id, e);
                }
            }
            sleep(IDB_SCAN_INTERVAL).await;
        }
    });
    handles.push(task.abort_handle());
    handles
}

/// Single scan cycle: open CDP, attach to the Slack page, walk IDB, detach.
///
/// `pinned_target_id` lets the caller persist the CDP `targetId` from the
/// first strict-fragment match across subsequent ticks. Once set, this
/// function resolves by id first so multi-account Slack sessions can't
/// accidentally cross-wire scanner A onto scanner B's page target after
/// Slack's router strips the `#openhuman-account-<id>` fragment.
async fn scan_once<R: Runtime>(
    app: &AppHandle<R>,
    account_id: &str,
    url_prefix: &str,
    url_fragment: &str,
    pinned_target_id: &mut Option<String>,
) -> Result<idb::IdbDump, String> {
    // Look up the in-process transport for this account, enumerate targets,
    // then attach to the chosen target via the canonical CdpConn. The
    // attach is manual (not via `connect_and_attach_matching_in_process`)
    // so the pin / strict-fragment / relaxed fallback hierarchy stays
    // intact.
    let mut cdp = crate::cdp::target::conn_for_account(app, account_id)?;
    let targets_v = cdp.call("Target.getTargets", json!({}), None).await?;
    let targets = crate::cdp::target::parse_targets(&targets_v);
    // Slack's client-side router does pushState to `/client/<workspace>/<channel>`
    // shortly after first load, which strips the `#openhuman-account-<id>` fragment.
    // The fragment is only reliable on the FIRST scan tick (immediately after
    // navigation) — by tick 2 it's gone.
    //
    // Resolution order:
    //   1. If we previously locked onto a `targetId` via a strict fragment
    //      match, prefer that exact id. This pins the scanner to the same
    //      account-tab even after the fragment is gone.
    //   2. Strict fragment match (`url_prefix` + `#openhuman-account-<id>`).
    //      On hit, persist the `targetId` for future ticks.
    //   3. Relaxed prefix-only match. Per-account `data_directory`
    //      isolation makes this safe in single-account setups, but in a
    //      multi-account Slack session it can bind to a sibling account's
    //      tab — only used as a last resort and never persisted.
    let page_target = pinned_target_id
        .as_ref()
        .and_then(|pid| targets.iter().find(|t| &t.id == pid && t.kind == "page"))
        .or_else(|| {
            targets.iter().find(|t| {
                t.kind == "page" && t.url.starts_with(url_prefix) && t.url.ends_with(url_fragment)
            })
        })
        .or_else(|| {
            targets
                .iter()
                .find(|t| t.kind == "page" && t.url.starts_with(url_prefix))
        })
        .ok_or_else(|| format!("no page target matching {url_prefix} fragment={url_fragment}"))?;

    // Persist the target id only when the strict fragment is still present
    // — that's the only signal that proves this target really belongs to
    // *this* account. Relaxed matches must never feed back into the pin.
    if pinned_target_id.is_none()
        && page_target.url.starts_with(url_prefix)
        && page_target.url.ends_with(url_fragment)
    {
        log::info!(
            "[sl][{}] pinned to target_id={} (strict fragment match)",
            account_id,
            page_target.id
        );
        *pinned_target_id = Some(page_target.id.clone());
    }

    let attach = cdp
        .call(
            "Target.attachToTarget",
            json!({ "targetId": page_target.id, "flatten": true }),
            None,
        )
        .await?;
    let session = attach
        .get("sessionId")
        .and_then(|x| x.as_str())
        .ok_or_else(|| "page attach missing sessionId".to_string())?
        .to_string();

    let result = idb::walk(&mut cdp, &session).await;

    let _ = cdp
        .call(
            "Target.detachFromTarget",
            json!({ "sessionId": session }),
            None,
        )
        .await;

    let dump = result?;
    log::info!(
        "[sl][{}] scan ok dbs={} total_records={}",
        account_id,
        dump.dbs.len(),
        dump.dbs
            .iter()
            .flat_map(|d| d.stores.iter())
            .map(|s| s.records.len())
            .sum::<usize>(),
    );
    Ok(dump)
}

/// Slack names its per-workspace DB `objectStore-<TEAM_ID>-<USER_ID>`.
/// Pull the `T…` token from the middle. Returns None if no such DB
/// exists — in which case we fall back to the `id`-shape match in
/// `extract::walk` (any record with `id.starts_with('T')`).
fn infer_team_id(dump: &idb::IdbDump) -> Option<String> {
    for db in &dump.dbs {
        if let Some(rest) = db.name.strip_prefix("objectStore-") {
            // e.g. "T01CWHNCJ9Z-U01CT9ADP6H"
            let team = rest.split('-').next().unwrap_or("");
            if team.starts_with('T')
                && team.len() >= 9
                && team.chars().all(|c| c.is_ascii_alphanumeric())
            {
                return Some(team.to_string());
            }
        }
    }
    None
}

/// Group messages by channel (no per-day split), emit one
/// `webview:event` per channel, and POST the same payload to
/// `openhuman.memory_doc_ingest`. One memory doc per channel — the
/// transcript inside can be long, each message line still carries its
/// date so the full chronology stays readable.
#[allow(clippy::too_many_arguments)]
fn emit_and_persist<R: Runtime>(
    app: &AppHandle<R>,
    account_id: &str,
    messages: &[extract::ExtractedMessage],
    users: &HashMap<String, String>,
    channels: &HashMap<String, String>,
    team_id: &str,
    workspace_name: &str,
) {
    #[derive(Default)]
    struct Group {
        rows: Vec<Value>,
    }
    let mut groups: HashMap<String, Group> = HashMap::new();
    for m in messages {
        if m.channel.is_empty() || m.ts.is_empty() {
            continue;
        }
        let ts_secs = parse_slack_ts(&m.ts).unwrap_or(0);
        if ts_secs <= 0 {
            continue;
        }
        let sender = users
            .get(&m.user)
            .cloned()
            .or_else(|| {
                if m.user.is_empty() {
                    None
                } else {
                    Some(m.user.clone())
                }
            })
            .unwrap_or_default();
        let row = json!({
            "ts": m.ts,
            "ts_secs": ts_secs,
            "sender": sender,
            "user_id": m.user,
            "body": m.text,
        });
        groups.entry(m.channel.clone()).or_default().rows.push(row);
    }

    let mut emitted = 0usize;
    for (channel_id, group) in groups {
        let mut rows = group.rows;
        rows.sort_by(|a, b| {
            a.get("ts_secs")
                .and_then(|v| v.as_i64())
                .unwrap_or(0)
                .cmp(&b.get("ts_secs").and_then(|v| v.as_i64()).unwrap_or(0))
        });
        // De-duplicate within the channel by `ts` (Slack messages are
        // unique per-channel per-ts). The walker can see the same record
        // in multiple Redux snapshots, so dedupe is not optional.
        let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
        rows.retain(|r| {
            let ts = r
                .get("ts")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            !ts.is_empty() && seen.insert(ts)
        });
        if rows.is_empty() {
            continue;
        }
        let channel_name = channels
            .get(&channel_id)
            .cloned()
            .unwrap_or_else(|| channel_id.clone());

        let payload = json!({
            "provider": "slack",
            "source": "cdp-idb",
            "teamId": team_id,
            "workspaceName": workspace_name,
            "channelId": channel_id,
            "channelName": channel_name,
            "messages": rows,
        });
        let envelope = json!({
            "account_id": account_id,
            "provider": "slack",
            "kind": "ingest",
            "payload": payload.clone(),
            "ts": chrono_now_millis(),
        });
        if let Err(e) = app.emit("webview:event", &envelope) {
            log::warn!("[sl][{}] ingest emit failed: {}", account_id, e);
        } else {
            emitted += 1;
        }
        let acct = account_id.to_string();
        tokio::spawn(async move {
            if let Err(e) = post_memory_doc_ingest(&acct, &payload).await {
                log::warn!("[sl][{}] memory write failed: {}", acct, e);
            }
        });
    }
    log::info!("[sl][{}] emitted {} channel doc(s)", account_id, emitted);
}

/// Parse Slack's `"unix_seconds.microseconds"` ts string to unix seconds.
pub(crate) fn parse_slack_ts(s: &str) -> Option<i64> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    s.split('.').next()?.parse::<i64>().ok()
}

/// Slack ts shape check: `<10 digits>.<1-8 digits>`.
pub(crate) fn looks_like_slack_ts(s: &str) -> bool {
    let bytes = s.as_bytes();
    let dot = match s.find('.') {
        Some(i) => i,
        None => return false,
    };
    if !(9..=11).contains(&dot) {
        return false;
    }
    if !bytes[..dot].iter().all(|b| b.is_ascii_digit()) {
        return false;
    }
    let frac = &bytes[dot + 1..];
    if frac.is_empty() || frac.len() > 8 {
        return false;
    }
    frac.iter().all(|b| b.is_ascii_digit())
}

/// Unix seconds → UTC `YYYY-MM-DD` (Howard Hinnant civil-from-days).
fn seconds_to_ymd(secs: i64) -> String {
    let days = secs.div_euclid(86_400);
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = (if mp < 10 { mp + 3 } else { mp - 9 }) as u32;
    let y_real = (if m <= 2 { y + 1 } else { y }) as i32;
    format!("{:04}-{:02}-{:02}", y_real, m, d)
}

fn chrono_now_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Build and POST the `openhuman.memory_doc_ingest` payload for a single
/// (channel, day) group. Mirrors `whatsapp_scanner::post_memory_doc_ingest`.
async fn post_memory_doc_ingest(account_id: &str, ingest: &Value) -> Result<(), String> {
    let channel_id = ingest
        .get("channelId")
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    let channel_name = ingest
        .get("channelName")
        .and_then(|v| v.as_str())
        .unwrap_or(channel_id);
    let team_id = ingest
        .get("teamId")
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    let workspace_name = ingest
        .get("workspaceName")
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    let empty: Vec<Value> = Vec::new();
    let msgs = ingest
        .get("messages")
        .and_then(|v| v.as_array())
        .unwrap_or(&empty);
    if channel_id.is_empty() || msgs.is_empty() {
        return Ok(());
    }

    let mut sorted: Vec<&Value> = msgs.iter().collect();
    sorted.sort_by_key(|m| m.get("ts_secs").and_then(|v| v.as_i64()).unwrap_or(0));

    let first_ts = sorted
        .first()
        .and_then(|m| m.get("ts_secs"))
        .and_then(|v| v.as_i64())
        .unwrap_or(0);
    let last_ts = sorted
        .last()
        .and_then(|m| m.get("ts_secs"))
        .and_then(|v| v.as_i64())
        .unwrap_or(0);

    // Full-channel transcript — every line carries its own date + time so
    // the reader can scan chronology without needing per-day splits.
    let transcript: String = sorted
        .iter()
        .map(|m| {
            let ts = m.get("ts_secs").and_then(|v| v.as_i64()).unwrap_or(0);
            let stamp = if ts > 0 {
                let day = seconds_to_ymd(ts);
                let secs_of_day = (ts.rem_euclid(86_400)) as u32;
                format!(
                    "{} {:02}:{:02}Z",
                    day,
                    secs_of_day / 3600,
                    (secs_of_day / 60) % 60
                )
            } else {
                "?".to_string()
            };
            let who = m
                .get("sender")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .unwrap_or("?");
            let body = m
                .get("body")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .replace(['\r', '\n'], " ");
            format!("[{stamp}] {who}: {body}")
        })
        .collect::<Vec<_>>()
        .join("\n");

    let first_day = if first_ts > 0 {
        seconds_to_ymd(first_ts)
    } else {
        String::new()
    };
    let last_day = if last_ts > 0 {
        seconds_to_ymd(last_ts)
    } else {
        String::new()
    };
    let header = format!(
        "# Slack — {workspace} · #{channel}\nchannel_id: {channel_id}\nteam_id: {team_id}\naccount_id: {account_id}\nmessages: {n}\nrange: {first_day} → {last_day}\n\n",
        workspace = if workspace_name.is_empty() {
            "workspace"
        } else {
            workspace_name
        },
        channel = channel_name,
        channel_id = channel_id,
        team_id = team_id,
        account_id = account_id,
        n = sorted.len(),
        first_day = first_day,
        last_day = last_day,
    );
    let content = format!("{header}{transcript}");

    // Key = channel name when available (what the user asked for),
    // falling back to the channel id for anonymous DMs / unnamed rooms.
    // `:` is reserved by the memory layer (it rewrites to `_`), other
    // characters pass through. Slack channel names are already lowercase
    // letters/digits/dashes/underscores, so no further sanitisation needed.
    let namespace = format!("slack-web:{account_id}");
    let key = if channels_key_looks_clean(channel_name) {
        channel_name.to_string()
    } else {
        channel_id.to_string()
    };
    let title = format!("Slack · #{channel_name}");

    let params = json!({
        "namespace": namespace,
        "key": key,
        "title": title,
        "content": content,
        "source_type": "slack-web",
        "priority": "medium",
        "tags": ["slack", "channel-transcript"],
        "metadata": {
            "provider": "slack",
            "account_id": account_id,
            "team_id": team_id,
            "workspace_name": workspace_name,
            "channel_id": channel_id,
            "channel_name": channel_name,
            "first_day": first_day,
            "last_day": last_day,
            "message_count": sorted.len(),
        },
        "category": "core",
    });
    let body = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "openhuman.memory_doc_ingest",
        "params": params,
    });

    let url = crate::core_rpc::core_rpc_url_value();
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .build()
        .map_err(|e| format!("http client: {e}"))?;
    let req = crate::core_rpc::apply_auth(client.post(&url))
        .map_err(|e| format!("prepare {url}: {e}"))?;
    let resp = req
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("POST {url}: {e}"))?;
    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("{status}: {body}"));
    }
    let v: Value = resp.json().await.map_err(|e| format!("decode: {e}"))?;
    if let Some(err) = v.get("error") {
        return Err(format!("rpc error: {err}"));
    }
    log::info!(
        "[sl][{}] memory upsert ok namespace={} key={} msgs={} range={}→{}",
        account_id,
        namespace,
        key,
        sorted.len(),
        first_day,
        last_day,
    );
    Ok(())
}

/// Allow a channel name as a memory-doc key only if it looks like a
/// Slack-style slug — lowercase letters, digits, `-`, `_`. Reject
/// anything with `:` (reserved by the memory layer), spaces, or other
/// surprises; those fall back to the stable channel id.
fn channels_key_looks_clean(name: &str) -> bool {
    if name.is_empty() {
        return false;
    }
    name.chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.')
}

const DOM_POLL_INTERVAL: Duration = Duration::from_secs(2);

fn spawn_dom_poll<R: Runtime>(
    app: AppHandle<R>,
    account_id: String,
    url_prefix: String,
) -> AbortHandle {
    let task = tokio::spawn(async move {
        let fragment = crate::cdp::target_url_fragment(&account_id);
        sleep(Duration::from_secs(8)).await;
        let mut last_hash: Option<u64> = None;
        let mut last_unread_by_channel: Option<HashMap<String, u32>> = None;
        // Same pin-on-strict-match contract as the IDB scanner — see
        // `scan_once` for rationale.
        let mut pinned_target_id: Option<String> = None;
        loop {
            match dom_scan_once(
                &app,
                &account_id,
                &url_prefix,
                &fragment,
                &mut pinned_target_id,
            )
            .await
            {
                Ok(scan) => {
                    let current_unread_by_channel: HashMap<String, u32> = scan
                        .rows
                        .iter()
                        .map(|row| (row.name.clone(), row.unread))
                        .collect();
                    if let Some(prev) = &last_unread_by_channel {
                        for row in &scan.rows {
                            let before = prev.get(&row.name).copied().unwrap_or(0);
                            if row.unread > before && row.unread > 0 {
                                let delta = row.unread - before;
                                let body = if delta == 1 {
                                    "1 new unread message".to_string()
                                } else {
                                    format!("{delta} new unread messages")
                                };
                                log::info!(
                                    "[sl][{}] notifying channel={} unread_before={} unread_after={}",
                                    account_id,
                                    row.name,
                                    before,
                                    row.unread
                                );
                                crate::webview_accounts::forward_synthetic_notification(
                                    &app,
                                    &account_id,
                                    "slack",
                                    format!("#{}", row.name),
                                    body,
                                );
                            }
                        }
                    }
                    last_unread_by_channel = Some(current_unread_by_channel);
                    if Some(scan.hash) != last_hash {
                        log::info!(
                            "[sl][{}] dom scan rows={} unread={} hash={:x}",
                            account_id,
                            scan.rows.len(),
                            scan.total_unread,
                            scan.hash
                        );
                        last_hash = Some(scan.hash);
                        let envelope = json!({
                            "account_id": account_id,
                            "provider": "slack",
                            "kind": "ingest",
                            "payload": dom_snapshot::ingest_payload(&scan),
                            "ts": chrono_now_millis(),
                        });
                        if let Err(e) = app.emit("webview:event", &envelope) {
                            log::warn!("[sl][{}] dom ingest emit failed: {}", account_id, e);
                        }
                    }
                }
                Err(e) => log::debug!("[sl][{}] dom scan: {}", account_id, e),
            }
            sleep(DOM_POLL_INTERVAL).await;
        }
    });
    task.abort_handle()
}

async fn dom_scan_once<R: Runtime>(
    app: &AppHandle<R>,
    account_id: &str,
    url_prefix: &str,
    url_fragment: &str,
    pinned_target_id: &mut Option<String>,
) -> Result<dom_snapshot::DomScan, String> {
    // Same pin-on-strict-match contract as `scan_once`. Resolution order:
    // pinned id → strict fragment → relaxed `/client` fallback. Pin is
    // only persisted when the strict fragment is still present so a
    // relaxed match can never feed back into the lock.
    //
    // We reuse the account's in-process CDP transport to enumerate
    // targets, then attach to the chosen target via the same handle —
    // no separate probe connection is needed.
    let mut cdp = crate::cdp::target::conn_for_account(app, account_id)?;
    let targets_v = cdp.call("Target.getTargets", json!({}), None).await?;
    let candidates = crate::cdp::target::parse_targets(&targets_v);

    let chosen = pinned_target_id
        .as_ref()
        .and_then(|pid| candidates.iter().find(|t| &t.id == pid && t.kind == "page"))
        .or_else(|| {
            candidates.iter().find(|t| {
                t.kind == "page" && t.url.starts_with(url_prefix) && t.url.ends_with(url_fragment)
            })
        })
        .or_else(|| {
            // Slack's router strips the fragment after `pushState` to
            // `/client/...`. Restrict the relaxed fallback to the
            // `/client` path so we never pick up the marketing page or
            // a login redirect for a sibling account.
            candidates.iter().find(|t| {
                t.kind == "page" && t.url.starts_with(url_prefix) && t.url.contains("/client")
            })
        })
        .ok_or_else(|| format!("no page target matching {url_prefix} fragment={url_fragment}"))?;

    let chosen_id = chosen.id.clone();
    let chosen_url = chosen.url.clone();

    if pinned_target_id.is_none()
        && chosen_url.starts_with(url_prefix)
        && chosen_url.ends_with(url_fragment)
    {
        log::info!(
            "[sl][{}] dom pinned to target_id={} (strict fragment match)",
            account_id,
            chosen_id
        );
        *pinned_target_id = Some(chosen_id.clone());
    }

    let attach = cdp
        .call(
            "Target.attachToTarget",
            json!({ "targetId": chosen_id, "flatten": true }),
            None,
        )
        .await?;
    let session = attach
        .get("sessionId")
        .and_then(|x| x.as_str())
        .ok_or_else(|| "page attach missing sessionId".to_string())?
        .to_string();
    let scan = dom_snapshot::scan(&mut cdp, &session).await;
    crate::cdp::detach_session(&mut cdp, &session).await;
    scan
}

/// Registry to prevent double-spawning scanners for the same account.
#[derive(Default)]
pub struct ScannerRegistry {
    started: Mutex<HashMap<String, Vec<AbortHandle>>>,
}

impl ScannerRegistry {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    pub fn ensure_scanner<R: Runtime>(
        &self,
        app: AppHandle<R>,
        account_id: String,
        url_prefix: String,
    ) {
        let mut g = self.started.lock();
        if g.contains_key(&account_id) {
            log::debug!("[sl] scanner already running for {}", account_id);
            return;
        }
        let handles = spawn_scanner(app, account_id.clone(), url_prefix);
        g.insert(account_id, handles);
    }

    pub fn forget(&self, account_id: &str) {
        let handles = self.started.lock().remove(account_id);
        if let Some(handles) = handles {
            let count = handles.len();
            for handle in handles {
                handle.abort();
            }
            log::info!("[sl] aborted {} scanner task(s) for {}", count, account_id);
        }
    }

    pub fn forget_all(&self) -> usize {
        let entries: Vec<_> = self.started.lock().drain().collect();
        let task_count = entries.iter().map(|(_, handles)| handles.len()).sum();
        for (account_id, handles) in entries {
            for handle in handles {
                handle.abort();
            }
            log::debug!("[sl] aborted scanner tasks for {}", account_id);
        }
        if task_count > 0 {
            log::info!("[sl] aborted {} scanner task(s)", task_count);
        }
        task_count
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn insert_pending_tasks(
        registry: &ScannerRegistry,
        account_id: &str,
        count: usize,
    ) -> Vec<tokio::task::JoinHandle<()>> {
        let mut tasks = Vec::with_capacity(count);
        let mut abort_handles = Vec::with_capacity(count);
        for _ in 0..count {
            let task = tokio::spawn(async {
                std::future::pending::<()>().await;
            });
            abort_handles.push(task.abort_handle());
            tasks.push(task);
        }
        registry
            .started
            .lock()
            .insert(account_id.to_string(), abort_handles);
        tasks
    }

    async fn assert_cancelled(task: tokio::task::JoinHandle<()>) {
        let err = tokio::time::timeout(Duration::from_secs(1), task)
            .await
            .expect("aborted scanner task should finish")
            .expect_err("scanner task should be cancelled");
        assert!(err.is_cancelled());
    }

    async fn assert_all_cancelled(tasks: Vec<tokio::task::JoinHandle<()>>) {
        for task in tasks {
            assert_cancelled(task).await;
        }
    }

    #[tokio::test]
    async fn registry_forget_aborts_all_handles_for_account_only() {
        let registry = ScannerRegistry::default();
        let account_tasks = insert_pending_tasks(&registry, "acct-1", 2);
        let survivor_tasks = insert_pending_tasks(&registry, "acct-2", 1);

        registry.forget("acct-1");

        {
            let guard = registry.started.lock();
            assert_eq!(guard.len(), 1);
            assert!(guard.contains_key("acct-2"));
        }
        assert_all_cancelled(account_tasks).await;
        assert!(
            !survivor_tasks[0].is_finished(),
            "forget(acct-1) must not abort acct-2"
        );

        assert_eq!(registry.forget_all(), 1);
        assert_all_cancelled(survivor_tasks).await;
    }

    #[tokio::test]
    async fn registry_forget_missing_account_is_noop() {
        let registry = ScannerRegistry::default();
        let mut tasks = insert_pending_tasks(&registry, "acct-1", 1);

        registry.forget("missing");

        {
            let guard = registry.started.lock();
            assert_eq!(guard.len(), 1);
            assert!(guard.contains_key("acct-1"));
        }
        assert!(
            !tasks[0].is_finished(),
            "forget(missing) must not abort existing scanners"
        );

        registry.forget("acct-1");
        assert_cancelled(tasks.pop().expect("task")).await;
    }

    #[tokio::test]
    async fn registry_forget_all_aborts_all_tasks_and_reports_handle_count() {
        let registry = ScannerRegistry::default();
        let task_a = insert_pending_tasks(&registry, "acct-1", 2);
        let task_b = insert_pending_tasks(&registry, "acct-2", 3);

        assert_eq!(registry.forget_all(), 5);

        assert!(registry.started.lock().is_empty());
        assert_all_cancelled(task_a).await;
        assert_all_cancelled(task_b).await;
    }

    #[tokio::test]
    async fn registry_forget_all_is_repeatable_noop_after_drain() {
        let registry = ScannerRegistry::default();
        assert_eq!(registry.forget_all(), 0);

        let tasks = insert_pending_tasks(&registry, "acct-1", 1);
        assert_eq!(registry.forget_all(), 1);
        assert_eq!(registry.forget_all(), 0);

        assert!(registry.started.lock().is_empty());
        assert_all_cancelled(tasks).await;
    }
}
