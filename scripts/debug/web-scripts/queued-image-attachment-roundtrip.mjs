import { readdirSync, readFileSync, statSync } from "node:fs";
import { join } from "node:path";

const FIRST_REPLY = "The first turn is complete.";
const UI_QUEUED_PROMPT = "Queue this follow-up through the composer.";
const QUEUED_PROMPT = "Describe this queued image.";
const QUEUED_REPLY = "The queued image is attached.";
const FILE_NAME = "queued-roundtrip.png";
const IMAGE_BYTES = Buffer.from(
  "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=",
  "base64",
);

function unwrap(value) {
  for (let depth = 0; depth < 6 && value && typeof value === "object"; depth += 1) {
    if (Object.hasOwn(value, "error")) {
      throw new Error(`RPC returned an error: ${JSON.stringify(value.error)}`);
    }
    const wrapper = ["result", "data", "value"].find((key) =>
      Object.hasOwn(value, key),
    );
    if (!wrapper) break;
    value = value[wrapper];
  }
  return value;
}

function findStoredBytes(root, expected) {
  for (const name of readdirSync(root)) {
    const path = join(root, name);
    const stat = statSync(path);
    if (stat.isDirectory()) {
      const found = findStoredBytes(path, expected);
      if (found) return found;
    } else if (stat.isFile() && stat.size === expected.length) {
      if (readFileSync(path).equals(expected)) return expected;
    }
  }
  return null;
}

function hasSourceAuthority(value) {
  if (Array.isArray(value)) return value.some(hasSourceAuthority);
  if (!value || typeof value !== "object") return false;
  return Object.entries(value).some(([key, child]) =>
    (["path", "url", "localCachePath"].includes(key) &&
      typeof child === "string" &&
      child.length > 0) ||
      hasSourceAuthority(child),
  );
}

async function waitUntil(predicate, label, timeoutMs = 10_000) {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    const value = await predicate();
    if (value) return value;
    await new Promise((resolve) => setTimeout(resolve, 50));
  }
  throw new Error(`timed out waiting for ${label}`);
}

export default async function queuedImageAttachmentRoundtrip({
  page,
  mock,
  log,
  logDir,
}) {
  const actionDir = process.env.OPENHUMAN_ACTION_DIR;
  if (!actionDir) throw new Error("OPENHUMAN_ACTION_DIR must isolate staged uploads");
  const rpcCalls = [];
  const rpcResponses = [];
  let rpcUrl;
  let authorization;
  page.on("request", async (request) => {
    if (!request.url().endsWith("/rpc")) return;
    rpcUrl = request.url();
    authorization = (await request.allHeaders()).authorization;
    try {
      const body = JSON.parse(request.postData() ?? "{}");
      rpcCalls.push({ method: body.method, params: body.params });
    } catch {}
  });
  page.on("response", async (response) => {
    if (!response.url().endsWith("/rpc")) return;
    try {
      const request = response.request();
      const body = JSON.parse(request.postData() ?? "{}");
      const responseBody = await response.json();
      try {
        rpcResponses.push({
          method: body.method,
          params: body.params,
          result: unwrap(responseBody),
        });
      } catch (error) {
        rpcResponses.push({
          method: body.method,
          params: body.params,
          error: String(error),
        });
      }
    } catch (error) {
      rpcResponses.push({ error: String(error) });
    }
  });

  // The mock captures a script when each model request starts. Hold the first
  // request before its first token, then change the script for the queued turn.
  mock.set("llmStreamScript", [
    ...Array.from({ length: 60 }, (_, index) => ({
      thinking: `Holding the first turn ${index + 1}.`,
      delayMs: 1_000,
    })),
    { text: FIRST_REPLY },
    { finish: "stop" },
  ]);
  await page.goto(new URL("/#/chat", page.url()).toString());
  const composer = page.getByRole("textbox", { name: "Message input" });
  await composer.waitFor({ state: "visible", timeout: 30_000 });
  let threadId = await page.evaluate(
    () =>
      window.__OPENHUMAN_STORE__?.getState?.().thread?.selectedThreadId ?? null,
  );
  if (!threadId) {
    await page.getByRole("button", { name: /new conversation/i }).click();
    threadId = await page
      .waitForFunction(
        () => window.__OPENHUMAN_STORE__?.getState?.().thread?.selectedThreadId,
        null,
        { timeout: 15_000 },
      )
      .then((handle) => handle.jsonValue());
  }
  if (!threadId) throw new Error("new chat thread was not selected");

  await composer.fill("Hold this turn while I queue an image.");
  await page.waitForTimeout(250);
  const firstSend = page.getByTestId("send-message-button");
  if (!(await firstSend.count())) {
    const value = await composer.inputValue().catch(() => "<not an input>");
    await page.screenshot({ path: join(logDir, "queued-first-send-missing.png") });
    throw new Error(`first send button missing after fill (draft=${JSON.stringify(value)})`);
  }
  await firstSend.click();
  await waitUntil(
    () =>
      mock
        .requests()
        .some((request) => /(?:^|\/)chat\/completions(?:\?|$)/.test(request.url)),
    "the first model request",
    30_000,
  );
  const firstAck = await waitUntil(
    () => rpcResponses.find((call) => call.method === "openhuman.channel_web_chat" && call.params?.message === "Hold this turn while I queue an image."),
    "first channel_web_chat response",
  );
  if (
    !firstAck.result ||
    firstAck.result.accepted !== true ||
    typeof firstAck.result.request_id !== "string"
  ) {
    throw new Error(`first chat response was not a started acknowledgement: ${JSON.stringify(firstAck)}`);
  }
  await page.waitForFunction(
    (id) =>
      window.__OPENHUMAN_STORE__?.getState?.().thread?.activeThreadIds?.[id] ===
      true,
    threadId,
    { timeout: 10_000 },
  );

  // The queued turn will use this fast script; the already-running request has
  // retained its delayed script. Exercise the actual assistant-ui queue send
  // control and ACK-to-Redux consumer first. The image follow-up below uses the
  // same public RPC directly because attachment picking is disabled mid-turn.
  mock.set("llmStreamScript", [
    { text: QUEUED_REPLY },
    { finish: "stop" },
  ]);
  await composer.fill(UI_QUEUED_PROMPT);
  await page.getByTestId("queue-message-button").click();
  const uiQueuedCall = await waitUntil(
    () =>
      rpcResponses.find(
        (call) =>
          call.method === "openhuman.channel_web_chat" &&
          call.params?.message === UI_QUEUED_PROMPT,
      ),
    "composer follow-up response",
  );
  const uiQueueAck = uiQueuedCall.result;
  if (
    uiQueueAck?.queued !== true ||
    typeof uiQueueAck.request_id !== "string" ||
    uiQueueAck.queued_attachments?.length !== 0
  ) {
    throw new Error(
      `composer follow-up did not produce a valid queued ACK: ${JSON.stringify(uiQueuedCall)}`,
    );
  }
  const uiPending = await page.waitForFunction(
    ({ id, requestId }) =>
      window.__OPENHUMAN_STORE__?.getState?.().queue?.pendingFollowupsByThread?.[
        id
      ]?.find((entry) => entry.requestId === requestId),
    { id: threadId, requestId: uiQueueAck.request_id },
    { timeout: 10_000 },
  );
  const uiPendingValue = await uiPending.jsonValue();
  if (
    uiPendingValue?.message?.content !== UI_QUEUED_PROMPT ||
    uiPendingValue?.attachments?.length !== 0
  ) {
    throw new Error(
      `composer ACK was not consumed into its matching pending UI entry: ${JSON.stringify(uiPendingValue)}`,
    );
  }
  const uiQueuedRequest = rpcCalls.find(
    (call) =>
      call.method === "openhuman.channel_web_chat" &&
      call.params?.message === UI_QUEUED_PROMPT,
  );
  if (uiQueuedRequest?.params?.queue_mode !== "followup") {
    throw new Error("composer queue control did not send queue_mode=followup");
  }

  const firstRequest = rpcCalls.find(
    (call) =>
      call.method === "openhuman.channel_web_chat" &&
      call.params?.message === "Hold this turn while I queue an image.",
  );
  if (!firstRequest?.params?.client_id) {
    throw new Error("the first chat RPC did not expose its socket client ID");
  }
  const queuedResponse = await page.evaluate(
    async ({ url, token, clientId, id, imageBase64 }) => {
      const response = await fetch(url, {
        method: "POST",
        headers: { authorization: token, "content-type": "application/json" },
        body: JSON.stringify({
          jsonrpc: "2.0",
          id: "queued-image-admission",
          method: "openhuman.channel_web_chat",
          params: {
            client_id: clientId,
            thread_id: id,
            message: "Describe this queued image.",
            queue_mode: "followup",
            attachments: [
              {
                kind: "image",
                altText: null,
                caption: null,
                media: {
                  kind: "image",
                  url: `data:image/png;base64,${imageBase64}`,
                  contentType: "image/png",
                  fileName: "queued-roundtrip.png",
                  sizeBytes: atob(imageBase64).length,
                  codec: null,
                  durationMs: null,
                  height: null,
                  localCachePath: null,
                  path: null,
                  platformMessageId: null,
                  transcribed: false,
                  width: null,
                },
              },
            ],
          },
        }),
      });
      const body = await response.json();
      if (!response.ok) throw new Error(`chat RPC failed with HTTP ${response.status}`);
      if (body.error) throw new Error(`chat RPC failed: ${JSON.stringify(body.error)}`);
      return body.result;
    },
    {
      url: rpcUrl,
      token: authorization,
      clientId: firstRequest.params.client_id,
      id: threadId,
      imageBase64: IMAGE_BYTES.toString("base64"),
    },
  );
  const ack = unwrap(queuedResponse);
  if (ack?.queued !== true || !ack.request_id) {
    throw new Error(
      `queued channel_web_chat ACK is malformed: ${JSON.stringify(ack)}`,
    );
  }
  if (
    ack.queued_attachments?.length !== 1 ||
    ack.queued_attachments[0]?.kind !== "image" ||
    !ack.queued_attachments[0]?.media?.opaqueId
  ) {
    throw new Error(
      `queued ACK did not retain the core's opaque image ref: ${JSON.stringify(ack)}`,
    );
  }
  if (
    /data:[^\s\"]+|base64,/i.test(JSON.stringify(ack.queued_attachments)) ||
    hasSourceAuthority(ack.queued_attachments)
  ) {
    throw new Error("queued ACK retained upload bytes or a source path");
  }

  const earlyRows = await page.evaluate(
    async ({ url, token, id }) => {
      const response = await fetch(url, {
        method: "POST",
        headers: { authorization: token, "content-type": "application/json" },
        body: JSON.stringify({
          jsonrpc: "2.0",
          id: "queued-image-before-flush",
          method: "openhuman.threads_messages_list",
          params: { thread_id: id },
        }),
      });
      const body = await response.json();
      if (!response.ok) throw new Error(`thread list failed with HTTP ${response.status}`);
      if (body.error) throw new Error(`thread list RPC failed: ${JSON.stringify(body.error)}`);
      const messages =
        body.result?.data?.messages ??
        body.result?.messages ??
        body.data?.messages ??
        body.messages;
      if (!Array.isArray(messages)) {
        throw new Error(`thread list response has no messages: ${JSON.stringify(body)}`);
      }
      return messages;
    },
    { url: rpcUrl, token: authorization, id: threadId },
  );
  if (
    earlyRows.some(
      (row) =>
        row.content === QUEUED_PROMPT || row.content === UI_QUEUED_PROMPT,
    )
  ) {
    throw new Error("queued image row was persisted before the active assistant turn finished");
  }
  const earlyUiRows =
    (await page.evaluate(
      (id) =>
        window.__OPENHUMAN_STORE__?.getState?.().thread?.messagesByThreadId?.[
          id
        ] ?? [],
      threadId,
    )) ?? [];
  if (
    earlyUiRows.some(
      (row) =>
        row.content === QUEUED_PROMPT || row.content === UI_QUEUED_PROMPT,
    )
  ) {
    throw new Error("queued image appeared as a persisted transcript row before flush");
  }
  const stillActive = await page.evaluate(
    (id) => window.__OPENHUMAN_STORE__?.getState?.().thread?.activeThreadIds?.[id] === true,
    threadId,
  );
  if (!stillActive) throw new Error("queued ACK arrived after the first turn stopped being active");
  await page.getByTestId("stop-generation-button").waitFor({
    state: "visible",
    timeout: 5_000,
  });
  if (await page.getByText(FIRST_REPLY, { exact: false }).isVisible().catch(() => false)) {
    throw new Error("held active turn completed before the queued-row timing assertion");
  }
  log("composer ACK reached the matching pending UI entry; queued RPC image ACK retained an opaque ref; neither had a transcript row before the active turn completed");

  // The mock holds the first turn for sixty one-second thinking deltas. The
  // queue/no-row checks above prove it remained active; this bounded wait lets
  // the scripted response naturally drain without a timing assertion race.
  await page.getByText(FIRST_REPLY, { exact: false }).waitFor({ timeout: 90_000 });
  await waitUntil(
    async () =>
      (await page.getByText(QUEUED_REPLY, { exact: false }).count()) >= 2,
    "both queued assistant replies",
    90_000,
  );

  const rows = await page.evaluate(
    async ({ url, token, id }) => {
      const response = await fetch(url, {
        method: "POST",
        headers: { authorization: token, "content-type": "application/json" },
        body: JSON.stringify({
          jsonrpc: "2.0",
          id: "queued-image-after-flush",
          method: "openhuman.threads_messages_list",
          params: { thread_id: id },
        }),
      });
      const body = await response.json();
      if (!response.ok) throw new Error(`thread list failed with HTTP ${response.status}`);
      if (body.error) throw new Error(`thread list RPC failed: ${JSON.stringify(body.error)}`);
      const messages =
        body.result?.data?.messages ??
        body.result?.messages ??
        body.data?.messages ??
        body.messages;
      if (!Array.isArray(messages)) {
        throw new Error(`thread list response has no messages: ${JSON.stringify(body)}`);
      }
      return messages;
    },
    { url: rpcUrl, token: authorization, id: threadId },
  );
  const userRows = rows.filter(
    (row) => row.sender === "user" && row.content === QUEUED_PROMPT,
  );
  if (userRows.length !== 1) {
    throw new Error(`queued image produced ${userRows.length} persisted user rows, expected one`);
  }
  const uiUserRows = rows.filter(
    (row) => row.sender === "user" && row.content === UI_QUEUED_PROMPT,
  );
  if (uiUserRows.length !== 1) {
    throw new Error(
      `composer follow-up produced ${uiUserRows.length} persisted user rows, expected one`,
    );
  }
  const persisted = userRows[0];
  if (persisted.id !== `user:${ack.request_id}`) {
    throw new Error(
      `queued row ID ${persisted.id} does not match request-key ownership`,
    );
  }
  if (/\[(?:ATTACHMENT|IMAGE|FILE|VOICE):/i.test(persisted.content)) {
    throw new Error("queued typed image leaked a legacy marker into persisted content");
  }
  const reference = persisted.attachments?.[0];
  if (
    reference?.kind !== "image" ||
    reference.media?.contentType !== "image/png" ||
    reference.media?.fileName !== FILE_NAME ||
    reference.media?.sizeBytes !== IMAGE_BYTES.length ||
    !reference.media?.opaqueId ||
    reference.media?.path ||
    reference.media?.url
  ) {
    throw new Error(`queued image did not persist as an opaque typed ref: ${JSON.stringify(reference)}`);
  }
  if (!findStoredBytes(actionDir, IMAGE_BYTES)) {
    throw new Error("queued model image bytes differ from the browser upload");
  }

  const imageCandidates = mock
    .requests()
    .filter((request) => /(?:^|\/)chat\/completions(?:\?|$)/.test(request.url))
    .map((request) => ({ request, body: JSON.parse(request.body) }))
    .filter(({ body }) =>
      (body.messages ?? []).some((message) => {
        if (message.role !== "user") return false;
        if (typeof message.content === "string") return message.content === QUEUED_PROMPT;
        return Array.isArray(message.content) && message.content.some(
          (part) => part.type === "text" && part.text.includes(QUEUED_PROMPT),
        );
      }),
    );
  if (!imageCandidates.length) {
    const observed = mock
      .requests()
      .filter((request) => /(?:^|\/)chat\/completions(?:\?|$)/.test(request.url))
      .map((request) => {
        const messages = JSON.parse(request.body).messages ?? [];
        return messages.filter((message) => message.role === "user").map((message) => ({
          content: Array.isArray(message.content)
            ? message.content.map((part) => part.type === "text"
                ? { type: "text", text: part.text }
                : { type: part.type, image_url: part.image_url
                    ? { urlPrefix: String(part.image_url.url ?? "").slice(0, 48) }
                    : undefined })
            : message.content,
        }));
      });
    throw new Error(`queued prompt was not found in a model user message: ${JSON.stringify(observed)}`);
  }
  const candidateShapes = imageCandidates.map(({ body }) =>
    body.messages.filter((message) => message.role === "user").map((message) => ({
      role: message.role,
      content: Array.isArray(message.content)
        ? message.content.map((part) => ({
            type: part.type,
            text: part.text,
            image_url: part.image_url
              ? { urlPrefix: String(part.image_url.url ?? "").slice(0, 48), detail: part.image_url.detail }
              : undefined,
          }))
        : message.content,
    })),
  );
  const imageRequest = imageCandidates[imageCandidates.length - 1].request;
  const input = JSON.stringify(imageCandidates[imageCandidates.length - 1].body.messages ?? []);
  if (/\[(?:ATTACHMENT|IMAGE|FILE|VOICE):/i.test(input)) {
    throw new Error("queued model input contains a legacy marker");
  }
  if (!input.includes(IMAGE_BYTES.toString("base64"))) {
    throw new Error(
      `queued model input did not contain the uploaded PNG bytes; exact-prompt candidates: ${JSON.stringify(candidateShapes)}`,
    );
  }

  await page.reload();
  await composer.waitFor({ state: "visible", timeout: 30_000 });
  await page.evaluate(
    (id) =>
      window.__OPENHUMAN_STORE__?.dispatch({
        type: "thread/setSelectedThread",
        payload: id,
      }),
    threadId,
  );
  await waitUntil(
    async () =>
      (await page.getByText(QUEUED_REPLY, { exact: false }).count()) >= 2,
    "both queued assistant replies after reload",
    30_000,
  );
  const reloaded = await page.evaluate(
    ({ id, rowId }) => {
      const messages =
        window.__OPENHUMAN_STORE__?.getState?.().thread?.messagesByThreadId?.[
          id
        ] ?? [];
      return messages.find((message) => message.sender === "user" && message.id === rowId);
    },
    { id: threadId, rowId: persisted.id },
  );
  if (
    reloaded?.content !== QUEUED_PROMPT ||
    JSON.stringify(reloaded?.attachments) !== JSON.stringify(persisted.attachments)
  ) {
    throw new Error("queued opaque image ref or clean text did not survive thread reload");
  }
  log(`queued image persisted once as ${persisted.id}; model received ${IMAGE_BYTES.length} typed PNG bytes`);
}
