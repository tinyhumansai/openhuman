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
 *   - `/memory/answer` answers deterministically from the pack it is given:
 *     "grounded answer for <question>" followed by the pack's top event text.
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

export async function handleMemory(ctx) {
  const { method, url, res, req, parsedBody } = ctx;
  const path = url.split("?")[0];
  if (!path.startsWith("/memory/")) return false;
  let route = path.slice("/memory/".length).replace(/\/+$/, "");
  let eventId = null;
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
  ]);
  if (!known.has(route)) return false;

  const token = bearerOf(req);
  if (!token) {
    fail(res, 401, "UNAUTHORIZED", "missing bearer");
    return true;
  }
  const forced = Number(behavior().memoryForceStatus || 0);
  if (forced >= 400) {
    fail(res, forced, STATUS_CODES[forced] || "UPSTREAM_ERROR", "forced by mock");
    return true;
  }

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
