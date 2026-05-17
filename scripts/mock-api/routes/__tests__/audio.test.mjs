import assert from "node:assert/strict";
import test from "node:test";

import { resetMockBehavior, setMockBehavior } from "../../state.mjs";
import { handleAudio } from "../audio.mjs";

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

test.beforeEach(() => {
  resetMockBehavior();
});

test("mock audio speech route returns audio + visemes", async () => {
  const res = createRes();
  const handled = await handleAudio({
    method: "POST",
    url: "/openai/v1/audio/speech",
    parsedBody: {
      text: "hello world",
      with_visemes: true,
    },
    res,
  });

  assert.equal(handled, true);
  assert.equal(res.statusCode, 200);
  const payload = JSON.parse(res.body);
  assert.equal(payload.audio_mime, "audio/mpeg");
  assert.ok(
    typeof payload.audio_base64 === "string" && payload.audio_base64.length > 0,
  );
  assert.ok(Array.isArray(payload.visemes) && payload.visemes.length > 0);
  assert.ok(Array.isArray(payload.alignment) && payload.alignment.length > 0);
});

test("mock audio speech route honors behavior overrides", async () => {
  setMockBehavior("audioSpeechMime", "audio/wav");
  setMockBehavior(
    "audioSpeechBase64",
    Buffer.from("WAVMOCK", "utf8").toString("base64"),
  );

  const res = createRes();
  await handleAudio({
    method: "POST",
    url: "/openai/v1/audio/speech",
    parsedBody: { text: "override" },
    res,
  });

  const payload = JSON.parse(res.body);
  assert.equal(payload.audio_mime, "audio/wav");
  assert.equal(
    Buffer.from(payload.audio_base64, "base64").toString("utf8"),
    "WAVMOCK",
  );
});

test("mock audio transcription route returns deterministic text", async () => {
  setMockBehavior("audioTranscriptionText", "Podcast transcription.");
  const res = createRes();
  const handled = await handleAudio({
    method: "POST",
    url: "/openai/v1/audio/transcriptions",
    parsedBody: {},
    res,
  });

  assert.equal(handled, true);
  assert.equal(JSON.parse(res.body).text, "Podcast transcription.");
});
