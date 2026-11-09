import { json } from "../http.mjs";
import { behavior } from "../state.mjs";

/**
 * Mock of the TinyHumans backend's hosted CortexDB proxy (`/memory/*`), the
 * surface the core's `tinyhumans` memory engine speaks (tinymemory-remote,
 * `CortexWire::TinyHumans`).
 *
 * Every response is the backend's `{success,data}` envelope; failures are
 * `{success:false,error,errorCode}`. Each bearer token owns a separate
 * append-only event log, mirroring the real per-user isolation.
 *
 * Behaviour switches (via `setMockBehavior`):
 *   memoryForceStatus  "402" | "401" | "429" | ...  every /memory/* call fails
 *                      with that status and its canonical errorCode.
 *   memoryForceCode    with memoryForceStatus, the errorCode to answer instead,
 *                      e.g. "403" + "MEMORY_PLAN_REQUIRED": memory is not in
 *                      the account's plan (#8081).
 *
 * Like the real memory API:
 *   - a reused `Idempotency-Key` header is a 409 `CONFLICT`, never forwarded;
 *   - a reused body `idempotency_key` with different text is a 409
 *     `IDEMPOTENCY_CONFLICT`, and forgetting does not release the key;
 *   - `GET /memory/scopes` honours `limit` and caps at 50 without it;
 *   - `/memory/events` lists newest first with every record emitted twice,
 *     honours `scope`, `limit`, an offset `cursor` and a comma-separated
 *     `labels` filter (events carrying ANY one of the labels);
 *   - `GET /memory/events/:id` returns one event (404 NOT_FOUND otherwise);
 *   - `/memory/recall` ranks a scope's events by how many query words (3+
 *     letters) they hold, honours `view: "descend"`, the metadata `labels`
 *     filter and `budgets.per_layer_limits.events`, and mints a `pack_<n>` id
 *     that `/memory/answer` must be given as `use_pack_id`;
 *   - `/memory/forget` honours `cascade`: the default, `derived_only`, keeps
 *     the events (it only drops what was derived from them), `redact_events`
 *     removes them, anything else is a 400 `INVALID_CASCADE`;
 *   - `POST /memory/v1/erasures` is memory-api's scoped erasure, answered
 *     unwrapped: `{scope, audit_note}` only (else 400 `UNKNOWN_FIELD`), the
 *     root is a 422 `ROOT_ERASURE_REFUSED`, and it erases the scope and every
 *     scope below it synchronously: `{erased, scope, scopes, erasure_ids}`;
 *     `GET /memory/v1/erasures/:id` reads one back (404 otherwise);
 *   - `/memory/answer` answers deterministically from the pack it is given:
 *     "grounded answer for <question>" followed by the pack's top event text;
 *   - `DELETE /memory` erases the caller's whole store and answers
 *     `{erased: true, scopes}`.
 *
 * Everything is in memory and deterministic; `resetMockMemory()` clears it.
 */

const DEFAULT_SCOPE_PAGE = 50;

const ANSWER_KEYS = new Set([
  "scope",
  "question",
  "question_type",
  "question_date",
  "temporal",
  "filters",
  "answer_max_tokens",
  "answer_instructions",
  "cite_sources",
  "include_context",
  "use_pack_id",
]);

/** token -> { events, idempotency, claims, packs, nextOffset, nextId, nextPack } */
const stores = new Map();

export function resetMockMemory() {
  stores.clear();
}

function storeFor(token) {
  let store = stores.get(token);
  if (!store) {
    store = {
      events: [],
      idempotency: new Map(),
      claims: new Set(),
      packs: new Map(),
      nextOffset: 0,
      nextId: 0,
      nextPack: 0,
    };
    stores.set(token, store);
  }
  return store;
}

const STATUS_CODES = {
  401: "UNAUTHORIZED",
  402: "USER_INSUFFICIENT_CREDITS",
  403: "FORBIDDEN",
  409: "CONFLICT",
  429: "RATE_LIMITED",
  503: "UNAVAILABLE",
};

function fail(res, status, code, error) {
  json(res, status, {
    success: false,
    error: error || `failed: ${code}`,
    errorCode: code,
  });
}

/** Fails `res` as `memoryForceStatus` asks; false when it asks nothing. */
function failForced(res) {
  const forced = Number(behavior().memoryForceStatus || 0);
  if (forced < 400) return false;
  const code = behavior().memoryForceCode || STATUS_CODES[forced] || "UPSTREAM_ERROR";
  const error =
    code === "MEMORY_PLAN_REQUIRED"
      ? "memory is not included in this account's plan; nothing was processed"
      : "forced by mock";
  fail(res, forced, code, error);
  return true;
}

function ok(res, data, status = 200) {
  json(res, status, { success: true, data });
}

function bearerOf(req) {
  const header = String(req.headers?.authorization || "");
  return header.startsWith("Bearer ") ? header.slice(7).trim() : "";
}

function queryOf(url) {
  const index = url.indexOf("?");
  return new URLSearchParams(index === -1 ? "" : url.slice(index + 1));
}

function textOf(event) {
  return event?.content?.text;
}

/** Whether `event` carries any one of `wanted` (an empty list keeps all). */
function labelled(event, wanted) {
  if (!wanted.length) return true;
  const labels = Array.isArray(event?.context?.labels) ? event.context.labels : [];
  return labels.some((label) => wanted.includes(label));
}

/** Lower-cased query words of 3+ alphanumeric characters. */
function wordsOf(query) {
  return String(query || "")
    .split(/\s+/)
    .map((w) => w.replace(/^[^\p{L}\p{N}]+|[^\p{L}\p{N}]+$/gu, "").toLowerCase())
    .filter((w) => w.length >= 3);
}

/** Render an event's text for a reader, prefixing the speaker. */
function rendered(event) {
  const text = textOf(event);
  if (typeof text !== "string") return event;
  const role = String(event?.content?.role || "user");
  return { ...event, content: { ...event.content, text: `[${role}] ${text}` } };
}

/**
 * `DELETE /memory`: erases the caller's entire hosted memory (every scope of
 * this bearer's store) and reports how many scopes held anything.
 */
function eraseAll({ req, res }) {
  const token = bearerOf(req);
  if (!token) {
    fail(res, 401, "UNAUTHORIZED", "missing bearer");
    return true;
  }
  if (failForced(res)) return true;
  const store = storeFor(token);
  const scopes = new Set(store.events.map((e) => e.scope)).size;
  store.events = [];
  store.packs.clear();
  store.idempotency.clear();
  store.claims.clear();
  ok(res, { erased: true, scopes });
  return true;
}

export async function handleMemory(ctx) {
  const { method, url, res, req, parsedBody } = ctx;
  const path = url.split("?")[0];
  if ((path === "/memory" || path === "/memory/") && method === "DELETE") {
    return eraseAll(ctx);
  }
  if (!path.startsWith("/memory/")) return false;
  let route = path.slice("/memory/".length).replace(/\/+$/, "");
  let eventId = null;
  let erasureId = null;
  if (route.startsWith("v1/erasures/")) {
    erasureId = decodeURIComponent(route.slice("v1/erasures/".length));
    route = "v1/erasures";
  }
  if (route.startsWith("events/")) {
    eventId = decodeURIComponent(route.slice("events/".length));
    route = "events";
  }
  const known = new Set([
    "experience",
    "events",
    "recall",
    "forget",
    "scopes",
    "answer",
    "v1/erasures",
  ]);
  if (!known.has(route)) return false;

  const token = bearerOf(req);
  if (!token) {
    fail(res, 401, "UNAUTHORIZED", "missing bearer");
    return true;
  }
  if (failForced(res)) return true;

  const store = storeFor(token);
  const body = parsedBody && typeof parsedBody === "object" ? parsedBody : {};

  if (route === "experience" && method === "POST") {
    const key = String(body.idempotency_key || "").trim();
    if (!key) {
      fail(res, 400, "MISSING_IDEMPOTENCY_KEY", "idempotency_key is required");
      return true;
    }
    const claim = req.headers?.["idempotency-key"];
    if (claim) {
      if (store.claims.has(claim)) {
        fail(res, 409, "CONFLICT", "already claimed");
        return true;
      }
      store.claims.add(claim);
    }
    const text = String(textOf(body) ?? "");
    const scope = String(body.scope || "");
    const modality = String(body.modality || "");
    const content = body.content ?? {};
    const seen = store.idempotency.get(key);
    if (seen) {
      if (
        seen.text !== text ||
        seen.scope !== scope ||
        seen.modality !== modality ||
        JSON.stringify(seen.content) !== JSON.stringify(content)
      ) {
        fail(res, 409, "IDEMPOTENCY_CONFLICT", "idempotency key reused");
        return true;
      }
      ok(res, { event_id: seen.id, replayed_from_idempotency: true }, 200);
      return true;
    }
    store.nextOffset += 2;
    store.nextId += 1;
    const id = `evt_${store.nextId}`;
    store.idempotency.set(key, { text, scope, modality, content, id });
    const given = body.context && typeof body.context === "object" ? body.context : {};
    store.events.push({
      id,
      scope,
      modality,
      wal_offset: store.nextOffset,
      content,
      context: { ...given, recorded_at: new Date().toISOString() },
    });
    ok(res, { event_id: id, status: "captured", replayed_from_idempotency: false });
    return true;
  }

  if (route === "events" && method === "GET" && eventId !== null) {
    const found = store.events.find((e) => e.id === eventId);
    if (!found) {
      fail(res, 404, "NOT_FOUND", "event not found");
      return true;
    }
    ok(res, found);
    return true;
  }

  if (route === "events" && method === "GET") {
    const params = queryOf(url);
    const scope = params.get("scope") || "";
    const cursor = Number(params.get("cursor") || 0) || 0;
    const limit = Number(params.get("limit") || 50) || 50;
    const wanted = (params.get("labels") || "")
      .split(",")
      .map((l) => l.trim())
      .filter(Boolean);
    const stream = [];
    for (const event of [...store.events].reverse()) {
      if (event.scope !== scope || !labelled(event, wanted)) continue;
      stream.push(event, event);
    }
    const items = stream.slice(cursor, cursor + limit);
    const next = cursor + items.length;
    ok(res, {
      items,
      has_more: next < stream.length,
      next_cursor: String(next),
    });
    return true;
  }

  if (route === "recall" && method === "POST") {
    const scope = String(body.scope || "");
    const descend = body.view === "descend";
    const wanted = Array.isArray(body.filters?.metadata?.labels)
      ? body.filters.metadata.labels.map(String)
      : [];
    const words = wordsOf(body.query);
    const budgetRaw = body.budgets?.per_layer_limits?.events;
    const budget = Number.isFinite(Number(budgetRaw)) && budgetRaw !== undefined
      ? Number(budgetRaw)
      : Infinity;
    const inScope = (e) => e.scope === scope || (descend && e.scope.startsWith(`${scope}/`));
    const scored = [];
    // Newest first, so equal scores rank the most recent event highest.
    for (const e of [...store.events].reverse()) {
      if (!inScope(e) || !labelled(e, wanted)) continue;
      const text = String(textOf(e) ?? "").toLowerCase();
      const score = words.filter((w) => text.includes(w)).length;
      if (words.length && score === 0) continue;
      scored.push({ score, e });
    }
    scored.sort((a, b) => b.score - a.score);
    const events = scored.slice(0, budget).map(({ e }) => rendered(e));
    store.nextPack += 1;
    const packId = `pack_${store.nextPack}`;
    store.packs.set(packId, events);
    ok(res, { pack_id: packId, layers: { events } });
    return true;
  }

  if (route === "forget" && method === "POST") {
    const scope = String(body.scope || "");
    const ids = Array.isArray(body.selector?.memory_ids)
      ? body.selector.memory_ids.map(String)
      : [];
    const unsupported = ["about_subject", "about_entity", "predicate"].some(
      (f) => body.selector && body.selector[f] !== undefined,
    );
    if (unsupported) {
      fail(res, 400, "UNSUPPORTED_SELECTOR", "the memory mock supports only memory_ids selectors");
      return true;
    }
    const selective = ids.length > 0;
    if (selective && body.confirm_all === true) {
      fail(res, 400, "AMBIGUOUS_SELECTOR_CONFIRM_ALL");
      return true;
    }
    if (!selective && body.confirm_all !== true) {
      fail(res, 422, "EMPTY_SELECTOR_WITHOUT_CONFIRMATION");
      return true;
    }
    const cascade = body.cascade === undefined ? "derived_only" : body.cascade;
    if (cascade !== "derived_only" && cascade !== "redact_events") {
      fail(res, 400, "INVALID_CASCADE");
      return true;
    }
    if (cascade === "derived_only") {
      // Only what was derived goes; the events stay.
      ok(res, { deleted: { events: 0 }, requested: ids.length, matched: 0 });
      return true;
    }
    const before = store.events.length;
    const requestedIds = new Set(ids);
    store.events = selective
      ? store.events.filter((e) => e.scope !== scope || !requestedIds.has(e.id))
      : store.events.filter((e) => e.scope !== scope);
    const deleted = before - store.events.length;
    ok(res, {
      deleted: { events: deleted },
      requested: ids.length,
      matched: deleted,
    });
    return true;
  }

  if (route === "v1/erasures" && method === "GET" && erasureId !== null) {
    const job = store.erasures?.get(erasureId);
    if (!job) {
      json(res, 404, { error_code: "NOT_FOUND" });
      return true;
    }
    json(res, 200, job);
    return true;
  }

  if (route === "v1/erasures" && method === "POST" && erasureId === null) {
    const unknown = Object.keys(body).find((k) => k !== "scope" && k !== "audit_note");
    if (unknown) {
      json(res, 400, { error_code: "UNKNOWN_FIELD", message: unknown });
      return true;
    }
    const scope = String(body.scope ?? "").replace(/\/+$/, "");
    if (!scope || scope === "/") {
      json(res, 422, { error_code: "ROOT_ERASURE_REFUSED" });
      return true;
    }
    const below = `${scope}/`;
    const held = [
      ...new Set(
        store.events
          .map((e) => e.scope)
          .filter((s) => s === scope || s.startsWith(below)),
      ),
    ];
    store.events = store.events.filter((e) => !held.includes(e.scope));
    store.erasures ??= new Map();
    const ids = held.map((s) => {
      const id = `erasure_${store.erasures.size + 1}`;
      store.erasures.set(id, { erasure_id: id, scope: s, status: "completed", phase: "done" });
      return id;
    });
    json(res, 200, { erased: true, scope, scopes: ids.length, erasure_ids: ids });
    return true;
  }

  if (route === "scopes" && method === "GET") {
    const limit = Number(queryOf(url).get("limit") || DEFAULT_SCOPE_PAGE) || DEFAULT_SCOPE_PAGE;
    const prefix = queryOf(url).get("prefix") || "";
    const paths = [...new Set(store.events.map((e) => e.scope))]
      .filter((p) => !prefix || p === prefix || p.startsWith(prefix))
      .sort();
    ok(res, { items: paths.slice(0, limit).map((path) => ({ path })) });
    return true;
  }

  if (route === "answer" && method === "POST") {
    const unknown = Object.keys(body).find((k) => !ANSWER_KEYS.has(k));
    if (unknown) {
      fail(res, 400, "VALIDATION_ERROR", `unknown key ${unknown}`);
      return true;
    }
    let events = [];
    if (body.use_pack_id !== undefined) {
      const pack = store.packs.get(String(body.use_pack_id));
      if (!pack) {
        fail(res, 400, "MISSING_PACK", "unknown use_pack_id");
        return true;
      }
      events = pack;
    }
    const top = events.find((e) => typeof textOf(e) === "string");
    const question = String(body.question || "");
    ok(res, {
      answer: top
        ? `grounded answer for ${question}: ${textOf(top)}`
        : `grounded answer for ${question}`,
      citations: events.slice(0, 5).map((e, i) => ({
        id: e.id,
        key: e.id,
        content: String(textOf(e) ?? ""),
        score: Number((1 / (1 + i)).toFixed(4)),
      })),
      context_block: events.map((e) => String(textOf(e) ?? "")).join("\n"),
      diagnostics: { answer_model: "mock" },
    });
    return true;
  }

  return false;
}
