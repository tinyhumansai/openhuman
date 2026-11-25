import { readdirSync, readFileSync, statSync } from "node:fs";
import { join } from "node:path";

const USER_TEXT = "Describe the attached image briefly.";
const REPLY = "The image is attached.";
const EDITED_TEXT = "Describe the same attached image with more detail.";
const EDITED_REPLY = "The same image remains attached after editing.";
const FILE_NAME = "typed-roundtrip.png";
const IMAGE_BYTES = Buffer.from(
  "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=",
  "base64",
);

function unwrap(value) {
  for (let depth = 0; depth < 6 && value && typeof value === "object"; depth += 1) {
    if (value.error) throw new Error(`core RPC failed: ${JSON.stringify(value.error)}`);
    const wrapper = ["result", "data", "value"].find((key) => key in value);
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
      const bytes = readFileSync(path);
      if (bytes.equals(expected)) return bytes;
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

export default async function typedImageAttachmentRoundtrip({
  page,
  mock,
  log,
  logDir,
}) {
  const actionDir = process.env.OPENHUMAN_ACTION_DIR;
  if (!actionDir) throw new Error("OPENHUMAN_ACTION_DIR must isolate staged uploads");
  const rpcCalls = [];
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
  mock.set("llmStreamScript", [{ text: REPLY }, { finish: "stop" }]);

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

  await page.locator('input[type="file"]').setInputFiles({
    name: FILE_NAME,
    mimeType: "image/png",
    buffer: IMAGE_BYTES,
  });
  await page
    .getByText(FILE_NAME, { exact: true })
    .waitFor({ state: "visible" });
  await composer.fill(USER_TEXT);
  await page.getByTestId("send-message-button").click();
  await page.getByText(REPLY, { exact: false }).waitFor({ timeout: 90_000 });

  const append = rpcCalls.find(
    (call) => call.method === "openhuman.threads_message_append",
  );
  if (!append)
    throw new Error(
      "composer did not persist the user message through the core",
    );
  const part = append.params?.attachments?.[0];
  const dataUri = part?.media?.url;
  if (
    part?.kind !== "image" ||
    part?.media?.contentType !== "image/png" ||
    part?.media?.fileName !== FILE_NAME ||
    !dataUri?.startsWith("data:image/png;name=")
  ) {
    throw new Error(
      `core append lacked the typed PNG payload: ${JSON.stringify(part)}`,
    );
  }

  const stagedMessage = unwrap(
    await page.evaluate(
      async ({ rpcUrl: url, token, threadId: id }) => {
        const response = await fetch(url, {
          method: "POST",
          headers: { authorization: token, "content-type": "application/json" },
          body: JSON.stringify({
            jsonrpc: "2.0",
            id: "typed-image-attachment-roundtrip-read",
            method: "openhuman.threads_messages_list",
            params: { thread_id: id },
          }),
        });
        return response.json();
      },
      { rpcUrl, token: authorization, threadId },
    ),
  );
  const persisted = stagedMessage?.messages?.find(
    (message) =>
      message.sender === "user" && message.id === append.params?.message?.id,
  );
  if (!persisted)
    throw new Error("typed image was not persisted in the thread message list");
  if (/\[(?:ATTACHMENT|IMAGE|FILE):/i.test(persisted.content))
    throw new Error(
      "modern typed upload leaked a staging marker into new persisted content",
    );
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
    throw new Error(
      `persisted image reference is not opaque and typed: ${JSON.stringify(reference)}`,
    );
  }
  if (persisted.content !== USER_TEXT)
    throw new Error("persisted user text differs from clean composer text");
  const stagedBytes = findStoredBytes(actionDir, IMAGE_BYTES);
  if (!stagedBytes)
    throw new Error("staged image bytes differ from the browser upload");
  log(
    `core staged ${reference.media.contentType} ${reference.media.fileName} (${stagedBytes.length} bytes) behind opaque ref ${reference.media.opaqueId}`,
  );

  const chatRequest = rpcCalls.find(
    (call) => call.method === "openhuman.channel_web_chat",
  );
  if (
    !chatRequest ||
    chatRequest.params?.message !== persisted.content ||
    chatRequest.params?.persisted_message_id !== persisted.id
  )
    throw new Error(
      "chat turn did not reference the marker-free persisted user row",
    );
  const clientId = chatRequest.params?.client_id;
  if (typeof clientId !== "string" || clientId.length === 0)
    throw new Error("initial chat request did not expose its live client ID");
  if (/\[(?:ATTACHMENT|IMAGE|FILE):/i.test(chatRequest.params.message))
    throw new Error(
      "chat turn sent a staging marker in its modern user text",
    );
  if (
    JSON.stringify(chatRequest.params?.attachments) !==
    JSON.stringify(persisted.attachments)
  )
    throw new Error("chat turn did not carry the persisted opaque media ref");
  if (
    /data:[^\s\"]+|base64,/i.test(JSON.stringify(chatRequest.params.attachments)) ||
    hasSourceAuthority(chatRequest.params.attachments)
  )
    throw new Error(
      `chat turn sent source bytes or a filesystem/network path: ${JSON.stringify(chatRequest.params.attachments)}`,
    );

  const inferenceRequest = mock
    .requests()
    .find((request) => /(?:^|\/)chat\/completions(?:\?|$)/.test(request.url));
  if (!inferenceRequest)
    throw new Error(
      "mock backend did not receive the model request for the image turn",
    );
  const inferenceBody = JSON.parse(inferenceRequest.body);
  const userMessages =
    inferenceBody.messages?.filter((message) => message.role === "user") ?? [];
  const userText = JSON.stringify(userMessages);
  if (/\[(?:ATTACHMENT|IMAGE|FILE):/i.test(userText))
    throw new Error(
      "legacy or durable staging markers reached the model's user text",
    );
  if (!userText.includes(IMAGE_BYTES.toString("base64")))
    throw new Error(
      "model request did not receive the uploaded image bytes as typed media",
    );
  log("mock model request contained typed image bytes and clean user text");

  // Exercise the real edit RPC with the persisted row's opaque refs. The core
  // must resolve them against that exact user row and restage trusted bytes;
  // it must not turn the refs into user-controlled paths or markers.
  mock.set("llmStreamScript", [
    { text: EDITED_REPLY },
    { finish: "stop" },
  ]);
  const editResponse = unwrap(
    await page.evaluate(
      async ({ rpcUrl: url, token, threadId: id, messageId, attachments, clientId: socketId }) => {
        const response = await fetch(url, {
          method: "POST",
          headers: { authorization: token, "content-type": "application/json" },
          body: JSON.stringify({
            jsonrpc: "2.0",
            id: "typed-image-edit-roundtrip",
            method: "openhuman.threads_edit_message",
            params: {
              thread_id: id,
              message_id: messageId,
              content: "Describe the same attached image with more detail.",
              attachments,
              client_id: socketId,
            },
          }),
        });
        return response.json();
      },
      {
        rpcUrl,
        token: authorization,
        threadId,
        messageId: persisted.id,
        attachments: persisted.attachments,
        clientId,
      },
    ),
  );
  if (typeof editResponse?.request_id !== "string")
    throw new Error(
      `edit RPC did not admit the replacement turn: ${JSON.stringify(editResponse)}`,
    );
  await page.getByText(EDITED_REPLY, { exact: false }).waitFor({ timeout: 90_000 });

  const editedMessages = unwrap(
    await page.evaluate(
      async ({ rpcUrl: url, token, threadId: id }) => {
        const response = await fetch(url, {
          method: "POST",
          headers: { authorization: token, "content-type": "application/json" },
          body: JSON.stringify({
            jsonrpc: "2.0",
            id: "typed-image-edit-roundtrip-read",
            method: "openhuman.threads_messages_list",
            params: { thread_id: id },
          }),
        });
        return response.json();
      },
      { rpcUrl, token: authorization, threadId },
    ),
  );
  const editedUserRows =
    editedMessages?.messages?.filter(
      (message) => message.sender === "user" && message.content === EDITED_TEXT,
    ) ?? [];
  const editedUserRow = editedUserRows[0];
  const editedRef = editedUserRow?.attachments?.[0];
  if (
    editedUserRows.length !== 1 ||
    editedMessages.messages.some(
      (message) =>
        (message.sender === "user" && message.content === USER_TEXT) ||
        (message.sender !== "user" && message.content === REPLY),
    ) ||
    editedUserRow.content.includes("[ATTACHMENT:") ||
    editedRef?.kind !== "image" ||
    editedRef.media?.opaqueId === reference.media.opaqueId ||
    editedRef.media?.path ||
    editedRef.media?.url
  )
    throw new Error(
      `edited row did not own fresh opaque image refs: ${JSON.stringify(editedUserRow)}`,
    );
  const modelRequests = mock
    .requests()
    .filter((request) => /(?:^|\/)chat\/completions(?:\?|$)/.test(request.url));
  const editedModelRequest = [...modelRequests].reverse().find((request) => {
    const body = JSON.parse(request.body);
    return body.messages?.some(
      (message) =>
        message.role === "user" &&
        JSON.stringify(message.content).includes(EDITED_TEXT) &&
        JSON.stringify(message.content).includes(IMAGE_BYTES.toString("base64")),
    );
  });
  const editedModelBody = editedModelRequest && JSON.parse(editedModelRequest.body);
  const editedUserMessages =
    editedModelBody?.messages?.filter((message) => message.role === "user") ?? [];
  const editedPrompt = JSON.stringify(editedUserMessages);
  if (
    !editedModelRequest ||
    !editedPrompt.includes(IMAGE_BYTES.toString("base64")) ||
    /\[(?:ATTACHMENT|IMAGE|FILE):/i.test(editedPrompt)
  )
    throw new Error("edited model request did not receive clean text and original image bytes");
  log("editing preserved image bytes and assigned fresh opaque ownership");

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
  await page.getByText(EDITED_REPLY, { exact: false }).waitFor({ timeout: 30_000 });
  const restoredEdit = await page.evaluate(
    ({ id, content }) => {
      const messages =
        window.__OPENHUMAN_STORE__?.getState?.().thread?.messagesByThreadId?.[
          id
        ] ?? [];
      return {
        row: messages.find(
          (message) => message.sender === "user" && message.content === content,
        ),
        staleOriginal: messages.some(
          (message) =>
            (message.sender === "user" && message.content === "Describe the attached image briefly.") ||
            (message.sender !== "user" && message.content === "The image is attached."),
        ),
      };
    },
    { id: threadId, content: EDITED_TEXT },
  );
  if (
    !restoredEdit.row ||
    restoredEdit.staleOriginal ||
    restoredEdit.row.content !== EDITED_TEXT ||
    /\[(?:ATTACHMENT|IMAGE|FILE):/i.test(restoredEdit.row.content ?? "") ||
    restoredEdit.row.attachments?.[0]?.media?.opaqueId !== editedRef.media.opaqueId
  )
    throw new Error("edited image row did not reload with clean text and fresh opaque reference");
  log(
    "typed image attachment survived core staging, chat submission and reload",
  );
}
