import assert from "node:assert/strict";
import test from "node:test";

import { resetMockBehavior, setMockBehavior } from "../../state.mjs";
import { handleMemory, resetMockMemory } from "../memory.mjs";

function createRes() {
  return {
    statusCode: 0,
    headers: {},
    body: "",
    writeHead(status, headers = {}) {
      this.statusCode = status;
      this.headers = headers;
    },
    setHeader(name, value) {
      this.headers[name] = value;
    },
    end(chunk = "") {
      this.body += String(chunk);
    },
  };
}

async function call(method, url, body, headers = { authorization: "Bearer tok-a" }) {
  const res = createRes();
  const handled = await handleMemory({
    method,
    url,
    res,
    req: { headers },
    parsedBody: body ?? null,
  });
  return { handled, status: res.statusCode, json: res.body ? JSON.parse(res.body) : null };
}

const write = (scope, key, text, headers) =>
  call(
    "POST",
    "/memory/experience",
    { scope, idempotency_key: key, modality: "text", content: { text } },
    headers,
  );

test.beforeEach(() => {
  resetMockBehavior();
  resetMockMemory();
});

test("non-memory paths are not handled", async () => {
  const r = await call("GET", "/health");
  assert.equal(r.handled, false);
});

test("a missing bearer is a 401 UNAUTHORIZED envelope", async () => {
  const r = await call("GET", "/memory/scopes", null, {});
  assert.equal(r.status, 401);
  assert.equal(r.json.success, false);
  assert.equal(r.json.errorCode, "UNAUTHORIZED");
});

test("experience then recall round-trips, prefixing the speaker", async () => {
  const w = await write("ns/a", "k1", "the sky is blue");
  assert.equal(w.status, 200);
  assert.equal(w.json.data.event_id, "evt_1");
  const r = await call("POST", "/memory/recall", { scope: "ns/a", query: "SKY" });
  assert.equal(r.json.data.layers.events[0].content.text, "[user] the sky is blue");
});

test("stores are isolated per bearer token", async () => {
  await write("ns/a", "k1", "secret");
  const r = await call("GET", "/memory/scopes", null, { authorization: "Bearer tok-b" });
  assert.deepEqual(r.json.data.items, []);
});

test("a reused Idempotency-Key header is a 409 CONFLICT", async () => {
  const headers = { authorization: "Bearer tok-a", "idempotency-key": "claim-1" };
  assert.equal((await write("s", "k1", "one", headers)).status, 200);
  const again = await write("s", "k2", "two", headers);
  assert.equal(again.status, 409);
  assert.equal(again.json.errorCode, "CONFLICT");
});

test("a reused body key with different text is IDEMPOTENCY_CONFLICT", async () => {
  await write("s", "k1", "one");
  const r = await write("s", "k1", "two");
  assert.equal(r.status, 409);
  assert.equal(r.json.errorCode, "IDEMPOTENCY_CONFLICT");
});

test("a reused body key with a different scope is IDEMPOTENCY_CONFLICT", async () => {
  await write("scope-a", "k1", "same text");
  const r = await write("scope-b", "k1", "same text");
  assert.equal(r.status, 409);
  assert.equal(r.json.errorCode, "IDEMPOTENCY_CONFLICT");
});

test("scopes honours limit and caps at 50 without it", async () => {
  for (let i = 0; i < 60; i += 1) await write(`scope/${String(i).padStart(2, "0")}`, `k${i}`, "x");
  const all = await call("GET", "/memory/scopes?limit=100");
  assert.equal(all.json.data.items.length, 60);
  const capped = await call("GET", "/memory/scopes");
  assert.equal(capped.json.data.items.length, 50);
  const two = await call("GET", "/memory/scopes?limit=2");
  assert.equal(two.json.data.items.length, 2);
});

test("memoryForceStatus=402 fails every call with USER_INSUFFICIENT_CREDITS", async () => {
  setMockBehavior("memoryForceStatus", "402");
  const r = await write("s", "k1", "x");
  assert.equal(r.status, 402);
  assert.equal(r.json.errorCode, "USER_INSUFFICIENT_CREDITS");
});

test("forget refuses an empty selector without confirm_all", async () => {
  const r = await call("POST", "/memory/forget", { scope: "s", selector: {} });
  assert.equal(r.status, 422);
  await write("s", "k1", "x");
  const del = await call("POST", "/memory/forget", { scope: "s", selector: {}, confirm_all: true });
  assert.equal(del.json.data.deleted.events, 1);
});


test("experience rejects missing or blank idempotency keys without poisoning later writes", async () => {
  for (const key of [undefined, "", "  "]) {
    const response = await call("POST", "/memory/experience", {
      scope: "s", idempotency_key: key, modality: "text", content: { text: "x" },
    });
    assert.equal(response.status, 400);
    assert.equal(response.json.errorCode, "MISSING_IDEMPOTENCY_KEY");
  }
  const valid = await write("s", "k1", "x");
  assert.equal(valid.status, 200);
  assert.equal(valid.json.data.event_id, "evt_1");
});

test("forget rejects unsupported selectors and scopes ID deletion", async () => {
  const first = await write("scope-a", "k1", "one");
  await write("scope-b", "k2", "two");
  const unsupported = await call("POST", "/memory/forget", {
    scope: "scope-a", selector: { about_subject: "person" },
  });
  assert.equal(unsupported.status, 400);
  assert.equal(unsupported.json.errorCode, "UNSUPPORTED_SELECTOR");
  const del = await call("POST", "/memory/forget", {
    scope: "scope-a", selector: { memory_ids: [first.json.data.event_id, "evt_2"] },
  });
  assert.equal(del.json.data.deleted.events, 1);
  const otherScope = await call("POST", "/memory/recall", { scope: "scope-b" });
  assert.equal(otherScope.json.data.layers.events.length, 1);
});

const writeLabelled = (scope, key, text, labels, role = "user") =>
  call("POST", "/memory/experience", {
    scope,
    idempotency_key: key,
    modality: "observation",
    content: { kind: "message", role, text },
    context: { labels },
  });

test("events keep context labels and the labels filter keeps any match", async () => {
  await writeLabelled("s", "k1", "alpha", ["item:a"]);
  await writeLabelled("s", "k2", "beta", ["item:b"]);
  await writeLabelled("s", "k3", "gamma", ["item:c", "item:a"]);
  const all = await call("GET", "/memory/events?scope=s&limit=50");
  assert.equal(all.json.data.items.length, 6, "every event is emitted twice");
  const some = await call("GET", "/memory/events?scope=s&labels=item%3Aa%2Citem%3Ab");
  const ids = new Set(some.json.data.items.map((e) => e.id));
  assert.deepEqual([...ids].sort(), ["evt_1", "evt_2", "evt_3"]);
  const onlyB = await call("GET", "/memory/events?scope=s&labels=item%3Ab");
  assert.deepEqual([...new Set(onlyB.json.data.items.map((e) => e.id))], ["evt_2"]);
  assert.deepEqual(onlyB.json.data.items[0].context.labels, ["item:b"]);
});

test("events/:id returns one event and 404s an unknown id", async () => {
  await writeLabelled("s", "k1", "alpha", ["item:a"]);
  const hit = await call("GET", "/memory/events/evt_1");
  assert.equal(hit.status, 200);
  assert.equal(hit.json.data.content.text, "alpha");
  const miss = await call("GET", "/memory/events/evt_99");
  assert.equal(miss.status, 404);
  assert.equal(miss.json.errorCode, "NOT_FOUND");
});

test("recall ranks by query words, honours labels, budgets and descend", async () => {
  await writeLabelled("app:tinymemory/app:learnings", "k1", "Alice prefers dark roast coffee", ["l:1"], "user");
  await writeLabelled("app:tinymemory/app:learnings", "k2", "Bob likes green tea", ["l:2"], "assistant");
  await writeLabelled("app:tinymemory/app:documents", "k3", "coffee machine manual", ["d:1"]);
  const exact = await call("POST", "/memory/recall", {
    scope: "app:tinymemory/app:learnings",
    query: "what coffee does Alice prefer?",
  });
  assert.equal(exact.json.data.layers.events.length, 1);
  assert.equal(exact.json.data.layers.events[0].content.text, "[user] Alice prefers dark roast coffee");
  const descend = await call("POST", "/memory/recall", {
    scope: "app:tinymemory",
    view: "descend",
    query: "coffee",
  });
  assert.equal(descend.json.data.layers.events.length, 2);
  const flat = await call("POST", "/memory/recall", { scope: "app:tinymemory", query: "coffee" });
  assert.equal(flat.json.data.layers.events.length, 0);
  const filtered = await call("POST", "/memory/recall", {
    scope: "app:tinymemory",
    view: "descend",
    query: "",
    filters: { metadata: { labels: ["l:2"] } },
  });
  assert.equal(filtered.json.data.layers.events[0].content.text, "[assistant] Bob likes green tea");
  const budget = await call("POST", "/memory/recall", {
    scope: "app:tinymemory",
    view: "descend",
    query: "",
    budgets: { per_layer_limits: { events: 1 } },
  });
  assert.equal(budget.json.data.layers.events.length, 1);
});

test("answer is grounded in the pack it is given and refuses an unknown pack", async () => {
  await writeLabelled("s", "k1", "The deploy key rotates monthly", ["l:1"]);
  const pack = await call("POST", "/memory/recall", { scope: "s", query: "deploy key" });
  const packId = pack.json.data.pack_id;
  const answered = await call("POST", "/memory/answer", {
    scope: "s",
    question: "when does the deploy key rotate?",
    use_pack_id: packId,
  });
  assert.equal(answered.status, 200);
  assert.match(answered.json.data.answer, /rotates monthly/);
  assert.equal(answered.json.data.citations[0].id, "evt_1");
  const bad = await call("POST", "/memory/answer", { scope: "s", question: "q", use_pack_id: "pack_nope" });
  assert.equal(bad.status, 400);
  assert.equal(bad.json.errorCode, "MISSING_PACK");
});

test("scopes honours the prefix filter", async () => {
  await write("app:tinymemory/app:learnings", "k1", "x");
  await write("other:scope", "k2", "y");
  const r = await call("GET", "/memory/scopes?prefix=app%3Atinymemory");
  assert.deepEqual(r.json.data.items.map((i) => i.path), ["app:tinymemory/app:learnings"]);
});
