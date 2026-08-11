/**
 * cursor-bridge: serves Cursor SDK models over an OpenAI-compatible HTTP API
 * so OpenHuman can use Cursor as a cloud provider (slug "cursor").
 *
 *   GET  /v1/models            -> Cursor.models.list(), expanded into
 *                                 reasoning-effort / preset variants
 *   POST /v1/chat/completions  -> one-shot Agent.prompt(), streamed back as SSE
 *                                 when the client asks for it
 *
 * Auth: the Authorization bearer is used as the Cursor API key; falls back to
 * CURSOR_API_KEY from the environment.
 */
import http from "node:http";
import path from "node:path";
import { mkdirSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { Agent, Cursor } from "@cursor/sdk";
import type { ModelListItem, ModelParameterValue } from "@cursor/sdk";

const HOST = "127.0.0.1";
const PORT = Number(process.env.CURSOR_BRIDGE_PORT || 8790);
const BRIDGE_DIR = path.dirname(fileURLToPath(import.meta.url));
const WORKSPACE = path.join(BRIDGE_DIR, "workspace");
mkdirSync(WORKSPACE, { recursive: true });

const MODELS_TTL_MS = 5 * 60 * 1000;

interface ResolvedModel {
  id: string;
  params?: ModelParameterValue[];
}

interface ModelCache {
  at: number;
  /** OpenAI-style model id -> SDK model selection. */
  byId: Map<string, ResolvedModel>;
}

const caches = new Map<string, ModelCache>();

const PARAM_MARKER = "~p=";

function encodedModelId(id: string, params: ModelParameterValue[]): string {
  return `${id}${params
    .map(param => `${PARAM_MARKER}${encodeURIComponent(param.id)}:${encodeURIComponent(param.value)}`)
    .join("")}`;
}

/**
 * Advertise the base model and each supported parameter value. The settings UI
 * groups these records into separate controls; selected controls are encoded
 * together, so users can combine e.g. reasoning + context + speed.
 */
function expandModel(model: ModelListItem, into: Map<string, ResolvedModel>): void {
  if (!into.has(model.id)) into.set(model.id, { id: model.id });

  for (const param of model.parameters ?? []) {
    for (const option of param.values) {
      const params: ModelParameterValue[] = [{ id: param.id, value: option.value }];
      const encoded = encodedModelId(model.id, params);
      if (!into.has(encoded)) into.set(encoded, { id: model.id, params });
    }
  }
}

function parseModelId(raw: string): ResolvedModel {
  const firstParam = raw.indexOf(PARAM_MARKER);
  if (firstParam < 0) return { id: raw };

  const id = raw.slice(0, firstParam);
  const params: ModelParameterValue[] = [];
  for (const part of raw.slice(firstParam + PARAM_MARKER.length).split(PARAM_MARKER)) {
    const separator = part.indexOf(":");
    if (separator <= 0) continue;
    try {
      params.push({
        id: decodeURIComponent(part.slice(0, separator)),
        value: decodeURIComponent(part.slice(separator + 1)),
      });
    } catch {
      // A malformed custom model entry is forwarded as its literal id below.
      return { id: raw };
    }
  }
  return params.length > 0 ? { id, params } : { id };
}

async function modelCacheFor(apiKey: string): Promise<ModelCache> {
  const cached = caches.get(apiKey);
  if (cached && Date.now() - cached.at < MODELS_TTL_MS) return cached;

  const models = await Cursor.models.list({ apiKey });
  const byId = new Map<string, ResolvedModel>();
  for (const model of models) expandModel(model, byId);
  const fresh: ModelCache = { at: Date.now(), byId };
  caches.set(apiKey, fresh);
  return fresh;
}

type ChatContent =
  | string
  | Array<
      | { type: "text"; text: string }
      | { type: "image_url"; image_url?: { url?: string } }
      | { type: string; text?: string }
    >;

interface ChatMessage {
  role: string;
  content?: ChatContent | null;
  name?: string;
}

function contentToText(content: ChatContent | null | undefined): string {
  if (!content) return "";
  if (typeof content === "string") return content;
  return content
    .map((part) => {
      if (part.type === "text") return part.text ?? "";
      if (part.type === "image_url") return "[image omitted: not supported by cursor-bridge]";
      return part.text ?? "";
    })
    .filter(Boolean)
    .join("\n");
}

const PREAMBLE = [
  "You are the backend model behind an OpenAI-compatible chat API.",
  "Reply to the final user message directly, in plain text, without preamble or meta-commentary.",
  "The conversation transcript follows.",
].join(" ");

function messagesToPrompt(messages: ChatMessage[]): string {
  const system: string[] = [];
  const turns: string[] = [];
  for (const message of messages) {
    const text = contentToText(message.content);
    if (!text) continue;
    if (message.role === "system") system.push(text);
    else if (message.role === "user") turns.push(`User: ${text}`);
    else if (message.role === "assistant") turns.push(`Assistant: ${text}`);
    else if (message.role === "tool") turns.push(`Tool output: ${text}`);
    else turns.push(`${message.role}: ${text}`);
  }
  const sections = [PREAMBLE];
  if (system.length) sections.push(`System instructions:\n${system.join("\n")}`);
  sections.push(turns.join("\n\n"));
  return sections.join("\n\n");
}

function apiKeyFrom(req: http.IncomingMessage): string | undefined {
  const header = req.headers.authorization;
  const bearer = header?.startsWith("Bearer ") ? header.slice("Bearer ".length).trim() : undefined;
  return bearer || process.env.CURSOR_API_KEY;
}

function sendJson(res: http.ServerResponse, status: number, body: unknown): void {
  const payload = JSON.stringify(body);
  res.writeHead(status, { "Content-Type": "application/json" });
  res.end(payload);
}

function sendError(res: http.ServerResponse, status: number, message: string): void {
  sendJson(res, status, { error: { message, type: "cursor_bridge_error", code: status } });
}

function sseChunk(res: http.ServerResponse, payload: unknown): void {
  res.write(`data: ${JSON.stringify(payload)}\n\n`);
}

function streamText(res: http.ServerResponse, id: string, model: string, text: string): void {
  res.writeHead(200, {
    "Content-Type": "text/event-stream",
    "Cache-Control": "no-cache",
    Connection: "keep-alive",
  });
  const created = Math.floor(Date.now() / 1000);
  const base = { id, object: "chat.completion.chunk", created, model };
  sseChunk(res, { ...base, choices: [{ index: 0, delta: { role: "assistant" }, finish_reason: null }] });
  const PIECE = 48;
  for (let i = 0; i < text.length; i += PIECE) {
    sseChunk(res, {
      ...base,
      choices: [{ index: 0, delta: { content: text.slice(i, i + PIECE) }, finish_reason: null }],
    });
  }
  sseChunk(res, { ...base, choices: [{ index: 0, delta: {}, finish_reason: "stop" }] });
  res.write("data: [DONE]\n\n");
  res.end();
}

async function readBody(req: http.IncomingMessage): Promise<string> {
  const chunks: Buffer[] = [];
  for await (const chunk of req) chunks.push(chunk as Buffer);
  return Buffer.concat(chunks).toString("utf8").replace(/^﻿/, "");
}

async function handleModels(res: http.ServerResponse, apiKey: string): Promise<void> {
  const cache = await modelCacheFor(apiKey);
  sendJson(res, 200, {
    object: "list",
    data: [...cache.byId.keys()].map((id) => ({
      id,
      object: "model",
      created: 0,
      owned_by: "cursor",
    })),
  });
}

async function handleCompletion(
  res: http.ServerResponse,
  apiKey: string,
  body: string,
): Promise<void> {
  const request = JSON.parse(body) as {
    model?: string;
    messages?: ChatMessage[];
    stream?: boolean;
  };
  if (!request.model) return sendError(res, 400, "missing model");
  if (!Array.isArray(request.messages) || request.messages.length === 0) {
    return sendError(res, 400, "missing messages");
  }

  const cache = await modelCacheFor(apiKey);
  const resolved = cache.byId.get(request.model) ?? parseModelId(request.model);
  const prompt = messagesToPrompt(request.messages);

  const result = await Agent.prompt(prompt, {
    apiKey,
    model: resolved.params ? { id: resolved.id, params: resolved.params } : { id: resolved.id },
    tools: [],
    local: { cwd: WORKSPACE },
  });

  if (result.status === "error") {
    return sendError(res, 502, result.error?.message ?? `cursor run ${result.id} failed`);
  }
  const text = result.result ?? "";
  const created = Math.floor(Date.now() / 1000);

  if (request.stream) {
    streamText(res, `chatcmpl-cursor-${result.id}`, request.model, text);
    return;
  }
  sendJson(res, 200, {
    id: `chatcmpl-cursor-${result.id}`,
    object: "chat.completion",
    created,
    model: request.model,
    choices: [
      {
        index: 0,
        message: { role: "assistant", content: text },
        finish_reason: "stop",
      },
    ],
  });
}

const server = http.createServer(async (req, res) => {
  try {
    const url = new URL(req.url ?? "/", `http://${HOST}`);
    if (req.method === "GET" && (url.pathname === "/" || url.pathname === "/health")) {
      return sendJson(res, 200, { ok: true, service: "cursor-bridge" });
    }

    const apiKey = apiKeyFrom(req);
    if (!apiKey) {
      return sendError(
        res,
        401,
        "no Cursor API key: send Authorization: Bearer <key> or set CURSOR_API_KEY",
      );
    }

    if (req.method === "GET" && url.pathname === "/v1/models") {
      return await handleModels(res, apiKey);
    }
    if (req.method === "POST" && url.pathname === "/v1/chat/completions") {
      return await handleCompletion(res, apiKey, await readBody(req));
    }
    sendError(res, 404, `unknown route: ${req.method} ${url.pathname}`);
  } catch (err) {
    const message = err instanceof Error ? err.message : String(err);
    sendError(res, 502, message);
  }
});

// Agent runs can take minutes; disable Node's default request timeouts.
server.requestTimeout = 0;
server.headersTimeout = 0;

server.listen(PORT, HOST, () => {
  console.log(`[cursor-bridge] listening on http://${HOST}:${PORT}/v1`);
});
