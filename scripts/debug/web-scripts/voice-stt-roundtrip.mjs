// Prove the hosted STT RPC sends valid audio to the local mock and returns its
// transcript through the canonical wire method.
const TRANSCRIPT = "mocked voice transcript from valid PCM audio";

function silentWavBase64() {
  const sampleRate = 16_000;
  const samples = 160;
  const payload = Buffer.alloc(44 + samples * 2);
  payload.write("RIFF", 0);
  payload.writeUInt32LE(36 + samples * 2, 4);
  payload.write("WAVEfmt ", 8);
  payload.writeUInt32LE(16, 16);
  payload.writeUInt16LE(1, 20);
  payload.writeUInt16LE(1, 22);
  payload.writeUInt32LE(sampleRate, 24);
  payload.writeUInt32LE(sampleRate * 2, 28);
  payload.writeUInt16LE(2, 32);
  payload.writeUInt16LE(16, 34);
  payload.write("data", 36);
  payload.writeUInt32LE(samples * 2, 40);
  return payload.toString("base64");
}

export default async function voiceSttRoundtrip({ mock, rpc, log }) {
  mock.set("audioTranscriptionText", TRANSCRIPT);
  const result = await rpc("openhuman.voice_stt_dispatch", {
    audio_base64: silentWavBase64(),
    mime_type: "audio/wav",
    file_name: "retained-stt.wav",
    language: "en",
    provider: "cloud",
  });
  if (result.text !== TRANSCRIPT) {
    throw new Error(
      `STT returned an unexpected transcript: ${JSON.stringify(result)}`,
    );
  }
  const requests = mock.requests();
  const request = requests.find(
    (entry) =>
      entry.method === "POST" &&
      /^\/openai\/v1\/audio\/transcriptions\/?$/.test(entry.url),
  );
  if (!request) {
    throw new Error(
      "STT returned text without sending a transcription request to the mock",
    );
  }
  if (!request.body.includes("retained-stt.wav")) {
    throw new Error(
      "the mock transcription request did not contain the submitted audio file",
    );
  }
  log(
    `voice STT receipt: ${JSON.stringify({
      provider: "cloud",
      transcript: result.text,
      request_method: request.method,
      request_path: new URL(request.url, mock.url).pathname,
      filename: "retained-stt.wav",
    })}`,
  );
}
