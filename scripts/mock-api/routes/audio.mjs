import { json } from "../http.mjs";
import { behavior, parseBehaviorJson } from "../state.mjs";

const DEFAULT_AUDIO_BYTES = Buffer.from("ID3MOCKAUDIO", "utf8");

function defaultVisemes() {
  return [
    { viseme: "sil", start_ms: 0, end_ms: 40 },
    { viseme: "aa", start_ms: 40, end_ms: 240 },
  ];
}

function defaultAlignment(text) {
  const chars = String(text || "ok")
    .slice(0, 8)
    .split("");
  return chars.map((char, index) => ({
    char,
    start_ms: index * 80,
    end_ms: index * 80 + 80,
  }));
}

export async function handleAudio(ctx) {
  const { method, url, parsedBody, res } = ctx;

  if (method === "POST" && /^\/openai\/v1\/audio\/speech\/?$/.test(url)) {
    const text = String(parsedBody?.text || "");
    const mockBehavior = behavior();
    const visemes = parseBehaviorJson("audioSpeechVisemes", defaultVisemes());
    const alignment = parseBehaviorJson(
      "audioSpeechAlignment",
      defaultAlignment(text),
    );
    const audioBytes = mockBehavior.audioSpeechBase64
      ? Buffer.from(String(mockBehavior.audioSpeechBase64), "base64")
      : DEFAULT_AUDIO_BYTES;
    json(res, 200, {
      audio_base64: audioBytes.toString("base64"),
      audio_mime: mockBehavior.audioSpeechMime || "audio/mpeg",
      visemes:
        parsedBody?.with_visemes === true || parsedBody?.with_alignment === true
          ? visemes
          : [],
      alignment:
        parsedBody?.with_alignment === true || parsedBody?.with_visemes === true
          ? alignment
          : undefined,
      voice_id:
        parsedBody?.voice_id || mockBehavior.audioSpeechVoiceId || "mock-voice",
      model_id:
        parsedBody?.model_id ||
        mockBehavior.audioSpeechModelId ||
        "mock-tts-v1",
    });
    return true;
  }

  if (
    method === "POST" &&
    /^\/openai\/v1\/audio\/transcriptions\/?$/.test(url)
  ) {
    json(res, 200, {
      text:
        behavior().audioTranscriptionText ||
        "Mock transcription from the E2E server.",
    });
    return true;
  }

  return false;
}
