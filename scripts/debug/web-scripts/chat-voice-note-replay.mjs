const TRANSCRIPT = "Please remind me about the dentist";
const REPLY = "I will remind you about the dentist.";

function silentWavBase64() {
  const sampleRate = 8000;
  const samples = new Uint8Array(sampleRate * 5);
  const bytes = new Uint8Array(44 + samples.length);
  const view = new DataView(bytes.buffer);
  const tag = (offset, value) =>
    [...value].forEach(
      (char, index) => (bytes[offset + index] = char.charCodeAt(0)),
    );
  tag(0, "RIFF");
  view.setUint32(4, 36 + samples.length, true);
  tag(8, "WAVE");
  tag(12, "fmt ");
  view.setUint32(16, 16, true);
  view.setUint16(20, 1, true);
  view.setUint16(22, 1, true);
  view.setUint32(24, sampleRate, true);
  view.setUint32(28, sampleRate, true);
  view.setUint16(32, 1, true);
  view.setUint16(34, 8, true);
  tag(36, "data");
  view.setUint32(40, samples.length, true);
  bytes.set(samples, 44);
  return Buffer.from(bytes).toString("base64");
}

const AUDIO_B64 = silentWavBase64();

export default async function voiceNoteReplay({ page, mock, log, screenshot }) {
  let typedVoiceNoteSent = false;
  let typedVoiceNoteId;
  let threadId = null;
  let rpcUrl;
  let rpcAuthorizationPromise;
  page.on("request", (request) => {
    if (!request.url().endsWith("/rpc")) return;
    rpcUrl = request.url();
    rpcAuthorizationPromise = request
      .allHeaders()
      .then((headers) => headers.authorization);
    try {
      const body = JSON.parse(request.postData() ?? "{}");
      if (body.method !== "openhuman.channel_web_chat") return;
      const params = body.params ?? {};
      const typedVoicePart = params.attachments?.find(
        (part) => part.kind === "voice_note",
      );
      typedVoiceNoteId = typedVoicePart?.voiceNote?.media?.opaqueId;
      typedVoiceNoteSent =
        params.source === "voice_note" &&
        !!typedVoiceNoteId &&
        params.message === TRANSCRIPT &&
        !/\[(?:FILE|VOICE):/i.test(params.message) &&
        !typedVoicePart.voiceNote?.media?.url &&
        typedVoicePart.voiceNote?.durationMs > 0;
      log(`voice-note chat RPC carried typed media: ${typedVoiceNoteSent}`);
    } catch {}
  });
  page.on("response", async (response) => {
    const body = response.request().postData() ?? "";
    if (body.includes("channel_web_voice_note_read")) {
      let result = "<unreadable>";
      try {
        result = JSON.stringify(await response.json(), (_key, value) =>
          typeof value === "string" && value.length > 256
            ? `[string:${value.length}]`
            : value,
        );
      } catch {}
      log(`voice note read RPC ${body}: ${result}`);
    }
  });
  mock.set("audioTranscriptionText", TRANSCRIPT);
  mock.set("audioSpeechBase64", AUDIO_B64);
  mock.set("audioSpeechMime", "audio/wav");
  mock.set("audioSpeechVisemes", [
    { viseme: "aa", start_ms: 0, end_ms: 180 },
    { viseme: "PP", start_ms: 180, end_ms: 420 },
  ]);
  mock.set("llmStreamScript", [{ text: REPLY }, { finish: "stop" }]);

  await page.goto(new URL("/#/chat", page.url()).toString());
  const composer = page.getByRole("textbox", { name: "Message input" });
  await composer.waitFor({ state: "visible", timeout: 30_000 });
  threadId = await page.evaluate(
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
  await page.waitForFunction(
    (id) =>
      window.__OPENHUMAN_STORE__?.getState?.().thread?.selectedThreadId === id,
    threadId,
    { timeout: 10_000 },
  );
  if (!threadId) throw new Error("new chat thread was not selected");

  await page.evaluate((wav) => {
    const raw = Uint8Array.from(atob(wav), (char) => char.charCodeAt(0));
    Object.defineProperty(navigator, "mediaDevices", {
      configurable: true,
      value: {
        getUserMedia: async () => ({ getTracks: () => [{ stop() {} }] }),
      },
    });
    window.MediaRecorder = class {
      static isTypeSupported() {
        return true;
      }
      constructor() {
        this.state = "inactive";
        this.mimeType = "audio/wav";
        this.listeners = {};
      }
      addEventListener(type, fn) {
        this.listeners[type] = fn;
      }
      start() {
        this.state = "recording";
      }
      stop() {
        this.state = "inactive";
        this.listeners.dataavailable?.({
          data: new Blob([raw], { type: "audio/wav" }),
        });
        this.listeners.stop?.();
      }
    };
  }, AUDIO_B64);

  await page.getByTestId("voice-note-record").click();
  await page.getByTestId("voice-note-recording").waitFor({ state: "visible" });
  await page.getByTestId("voice-note-stop").click();
  await page.getByTestId("voice-note-preview").waitFor({ state: "visible" });
  await page.getByTestId("voice-note-send").click();
  await page.waitForTimeout(3_000);
  await page.evaluate(
    (id) =>
      window.__OPENHUMAN_STORE__?.dispatch({
        type: "thread/setSelectedThread",
        payload: id,
      }),
    threadId,
  );
  log(
    `selected thread ${threadId}; user player count=${await page.getByTestId("voice-note-player").count()}`,
  );
  const transcriptToggle = page
    .getByTestId("voice-note-transcript-toggle")
    .first();
  if (await transcriptToggle.count()) await transcriptToggle.click();
  const state = await page.evaluate((id) => {
    const state = window.__OPENHUMAN_STORE__?.getState?.();
    const messages = state?.thread?.messagesByThreadId?.[id] ?? [];
    return messages.map((message) => ({
      id: message.id,
      sender: message.sender,
      content: message.content,
      extraMetadata: message.extraMetadata,
    }));
  }, threadId);
  log(`selected thread messages: ${JSON.stringify(state)}`);
  await page
    .getByText(TRANSCRIPT, { exact: false })
    .waitFor({ timeout: 30_000 });
  await page.getByText(REPLY, { exact: false }).waitFor({ timeout: 90_000 });

  await page
    .getByTestId("voice-note-player")
    .nth(1)
    .waitFor({ state: "visible", timeout: 90_000 });
  const requests = mock.requests().map((request) => request.url);
  for (const expected of [
    "/openai/v1/audio/transcriptions",
    "/openai/v1/audio/speech",
  ]) {
    if (!requests.includes(expected))
      throw new Error(
        `mock request missing ${expected}: ${requests.join(", ")}`,
      );
  }
  if (!typedVoiceNoteSent)
    throw new Error("voice-note turn did not send typed voice_note media");
  await page.reload();
  await page
    .getByRole("textbox", { name: "Message input" })
    .waitFor({ state: "visible", timeout: 30_000 });
  await page.evaluate(
    (id) =>
      window.__OPENHUMAN_STORE__?.dispatch({
        type: "thread/setSelectedThread",
        payload: id,
      }),
    threadId,
  );
  await page
    .getByTestId("voice-note-player")
    .nth(1)
    .waitFor({ state: "visible", timeout: 30_000 });
  const rpcAuthorization = await rpcAuthorizationPromise;
  if (!rpcUrl || !rpcAuthorization)
    throw new Error("could not capture the signed local core RPC endpoint");
  const transcriptResponse = await page.request.post(rpcUrl, {
    headers: {
      authorization: rpcAuthorization,
      "content-type": "application/json",
    },
    data: {
      jsonrpc: "2.0",
      id: "voice-note-transcript-replay",
      method: "openhuman.threads_transcript_get",
      params: { thread_id: threadId, limit: 500 },
    },
  });
  if (!transcriptResponse.ok())
    throw new Error(
      `threads_transcript_get returned ${transcriptResponse.status()}`,
    );
  let transcriptValue = await transcriptResponse.json();
  for (
    let depth = 0;
    depth < 6 && transcriptValue && !Array.isArray(transcriptValue.items);
    depth += 1
  ) {
    transcriptValue =
      transcriptValue.result ?? transcriptValue.data ?? transcriptValue.value;
  }
  const projectedVoiceNotes = (
    Array.isArray(transcriptValue?.items) ? transcriptValue.items : []
  )
    .filter((item) => item.extraMetadata?.voiceNote?.noteId)
    .map((item) => ({
      kind: item.kind,
      noteId: item.extraMetadata.voiceNote.noteId,
    }));
  log(
    `threads_transcript_get message correlation: ${JSON.stringify(
      transcriptValue?.items
        ?.filter(
          (item) =>
            item.kind === "userMessage" || item.kind === "assistantMessage",
        )
        .map((item) => ({
          kind: item.kind,
          content: item.content,
          requestId: item.requestId,
          extraMetadata: item.extraMetadata,
        })),
    )}`,
  );
  log(
    `threads_transcript_get voice rows: ${JSON.stringify(projectedVoiceNotes)}`,
  );
  if (
    !projectedVoiceNotes.some(
      (item) => item.kind === "userMessage" && item.noteId === typedVoiceNoteId,
    ) ||
    !projectedVoiceNotes.some((item) => item.kind === "assistantMessage")
  )
    throw new Error(
      `threads_transcript_get did not project both voice-note rows: ${JSON.stringify(projectedVoiceNotes)}`,
    );
  await page.getByText(REPLY, { exact: false }).waitFor({ timeout: 10_000 });
  await screenshot("voice-note-reloaded");

  await page.evaluate(() => {
    const NativeAudio = window.Audio;
    window.__E2E_AUDIO_ELEMENTS = [];
    window.Audio = function (source) {
      const audio = new NativeAudio(source);
      window.__E2E_AUDIO_ELEMENTS.push(audio);
      return audio;
    };
    window.Audio.prototype = NativeAudio.prototype;
  });
  const toggles = page.getByTestId("voice-note-toggle");
  if ((await toggles.count()) !== 2)
    throw new Error(
      `expected two voice-note players, found ${await toggles.count()}`,
    );
  for (let index = 0; index < 2; index += 1) {
    await toggles.nth(index).click();
    try {
      await page.waitForFunction(
        (i) => {
          const audio = window.__E2E_AUDIO_ELEMENTS?.[i];
          return (
            audio &&
            audio.readyState >= 2 &&
            Number.isFinite(audio.duration) &&
            audio.duration > 0
          );
        },
        index,
        { timeout: 10_000 },
      );
    } catch (error) {
      log(
        `audio ${index} state: ${JSON.stringify(
          await page.evaluate((i) => {
            const a = window.__E2E_AUDIO_ELEMENTS?.[i];
            return a
              ? {
                  src: a.src,
                  readyState: a.readyState,
                  networkState: a.networkState,
                  duration: a.duration,
                  error: a.error?.code,
                }
              : null;
          }, index),
        )}`,
      );
      throw error;
    }
    await page.waitForFunction(
      (i) => window.__E2E_AUDIO_ELEMENTS?.[i]?.paused === false,
      index,
      { timeout: 5_000 },
    );
    await toggles.nth(index).click();
    log(`reloaded voice note ${index + 1} decoded and played`);
  }
  log(
    `recorded and sent voice note; transcription and speech mock routes called; thread ${threadId} reloaded`,
  );
}
