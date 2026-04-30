#!/usr/bin/env node
// Phase 1 (data collection & standardization) — drive the memory layer with
// emails fetched from Composio's GMAIL_FETCH_EMAILS action.
//
// Inputs:
//   - A JSON file with the slim post-processed shape produced by
//     `src/openhuman/composio/providers/gmail/post_process.rs`. Each entry
//     under `messages[]` looks like:
//       { id, threadId, subject, from, to, date, labels, markdown, attachments }
//     Default fixture: tests/fixtures/memory/composio_gmail_inbox.json
//
// Behaviour:
//   - Groups messages by `threadId` so a single ingest call covers a whole
//     email thread (this is what the canonicaliser expects — one
//     EmailThread per source_id).
//   - For each thread calls `openhuman.memory_tree_ingest` with
//     source_kind=email + an EmailThread payload (see
//     src/openhuman/memory/tree/canonicalize/email.rs).
//   - Verifies via `openhuman.memory_tree_list_chunks` that chunks landed.
//
// Pre-reqs: the core server must already be serving JSON-RPC on $RPC_URL
// (default http://127.0.0.1:7810/rpc). Start it with:
//
//   cargo run --bin openhuman -- serve
//
// Usage:
//   node scripts/test-memory-email-ingest.mjs [path/to/inbox.json]
//
// Env:
//   RPC_URL  override the JSON-RPC endpoint
//   OWNER    owner string stamped on every chunk (default: stevent95@gmail.com)
//   PROVIDER provider tag emitted in EmailThread.provider (default: gmail)

import { readFile } from "node:fs/promises";
import { resolve } from "node:path";
import process from "node:process";

const RPC_URL = process.env.RPC_URL || "http://127.0.0.1:7810/rpc";
const OWNER = process.env.OWNER || "stevent95@gmail.com";
const PROVIDER = process.env.PROVIDER || "gmail";
const FIXTURE =
  process.argv[2] ||
  resolve("tests/fixtures/memory/composio_gmail_inbox.json");

let rpcId = 0;
async function rpc(method, params) {
  rpcId += 1;
  const res = await fetch(RPC_URL, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ jsonrpc: "2.0", id: rpcId, method, params }),
  });
  if (!res.ok) throw new Error(`${method}: HTTP ${res.status} ${await res.text()}`);
  const body = await res.json();
  if (body.error) {
    throw new Error(`${method}: ${body.error.message || JSON.stringify(body.error)}`);
  }
  return body.result;
}

function parseEmailDate(raw) {
  if (!raw) return Date.now();
  if (typeof raw === "number") return raw < 1e12 ? raw * 1000 : raw;
  const ms = Date.parse(raw);
  return Number.isFinite(ms) ? ms : Date.now();
}

function splitAddresses(value) {
  if (!value) return [];
  if (Array.isArray(value)) return value.filter(Boolean);
  return String(value)
    .split(/[;,]/)
    .map((s) => s.trim())
    .filter(Boolean);
}

function toEmailMessage(slim) {
  return {
    from: slim.from || "unknown@unknown",
    to: splitAddresses(slim.to),
    cc: splitAddresses(slim.cc),
    subject: slim.subject || "(no subject)",
    sent_at: parseEmailDate(slim.date),
    body: slim.markdown || "",
    source_ref: slim.id ? `gmail://message/${slim.id}` : null,
  };
}

function groupByThread(messages) {
  const threads = new Map();
  for (const m of messages) {
    const tid = m.threadId || m.id || "unknown-thread";
    if (!threads.has(tid)) threads.set(tid, []);
    threads.get(tid).push(m);
  }
  return [...threads.entries()].map(([threadId, msgs]) => {
    msgs.sort((a, b) => parseEmailDate(a.date) - parseEmailDate(b.date));
    return {
      threadId,
      subject: msgs[0]?.subject || "(no subject)",
      messages: msgs.map(toEmailMessage),
    };
  });
}

async function main() {
  console.log(`[memory-email-ingest] fixture=${FIXTURE}`);
  console.log(`[memory-email-ingest] rpc_url=${RPC_URL}`);

  // Sanity-check that the core is up.
  await rpc("openhuman.health_snapshot", {}).catch((err) => {
    throw new Error(
      `core not reachable at ${RPC_URL} — start it with \`cargo run --bin openhuman -- serve\`. (${err.message})`,
    );
  });

  const raw = await readFile(FIXTURE, "utf8");
  const inbox = JSON.parse(raw);
  const messages = Array.isArray(inbox.messages) ? inbox.messages : [];
  if (messages.length === 0) {
    console.error("[memory-email-ingest] no messages in fixture, nothing to do");
    process.exit(1);
  }
  console.log(`[memory-email-ingest] loaded ${messages.length} email(s)`);

  const threads = groupByThread(messages);
  console.log(`[memory-email-ingest] grouped into ${threads.length} thread(s)`);

  let chunksWritten = 0;
  let chunksDropped = 0;
  for (const t of threads) {
    const sourceId = `gmail:${t.threadId}`;
    const params = {
      source_kind: "email",
      source_id: sourceId,
      owner: OWNER,
      tags: ["gmail", "ingested", "phase1"],
      payload: {
        provider: PROVIDER,
        thread_subject: t.subject,
        messages: t.messages,
      },
    };
    process.stdout.write(
      `  · ${sourceId}  (${t.messages.length} msg, subject="${t.subject.slice(0, 60)}") … `,
    );
    try {
      const result = await rpc("openhuman.memory_tree_ingest", params);
      const r = result?.result || result || {};
      chunksWritten += r.chunks_written || 0;
      chunksDropped += r.chunks_dropped || 0;
      console.log(
        `ok  written=${r.chunks_written ?? 0} dropped=${r.chunks_dropped ?? 0}`,
      );
    } catch (err) {
      console.log(`FAIL ${err.message}`);
    }
  }

  console.log(
    `[memory-email-ingest] summary: threads=${threads.length} chunks_written=${chunksWritten} chunks_dropped=${chunksDropped}`,
  );

  // Quick verification — pull email chunks back out and print a count.
  const list = await rpc("openhuman.memory_tree_list_chunks", {
    source_kind: "email",
    owner: OWNER,
    limit: 100,
  });
  const chunks = list?.result?.chunks || list?.chunks || [];
  console.log(
    `[memory-email-ingest] verify: list_chunks(source_kind=email, owner=${OWNER}) returned ${chunks.length} chunk(s)`,
  );
  for (const c of chunks.slice(0, 5)) {
    const ts = c.metadata?.timestamp
      ? new Date(c.metadata.timestamp).toISOString()
      : "?";
    const preview = (c.content || "").replace(/\s+/g, " ").slice(0, 80);
    console.log(`    - ${c.id}  ${c.metadata?.source_id}  ${ts}  ${preview}`);
  }
}

main().catch((err) => {
  console.error(`[memory-email-ingest] fatal: ${err.message}`);
  process.exit(1);
});
