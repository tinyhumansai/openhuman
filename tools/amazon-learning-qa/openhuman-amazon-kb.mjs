#!/usr/bin/env node

import { mkdir, readFile, writeFile } from "node:fs/promises";
import { createReadStream } from "node:fs";
import { createInterface } from "node:readline/promises";
import path from "node:path";

import { resolveAmazonQaPaths } from "./amazon-qa-paths.mjs";

const QA_PATHS = resolveAmazonQaPaths(import.meta.dirname);
const ROOT = QA_PATHS.root;
const KB_ROOT = QA_PATHS.workspace;
const PROCESSED_DIR = path.join(KB_ROOT, "processed");
const ARTICLES_DIR = path.join(PROCESSED_DIR, "articles");
const MANIFEST_PATH = path.join(PROCESSED_DIR, "manifest.jsonl");
const DEFAULT_RPC = "http://127.0.0.1:7789/rpc";
const DEFAULT_NAMESPACE = "amazon-learning";

const SOURCES = [
  { author: "张子卿", dir: path.join(ROOT, "张子卿html") },
  { author: "飞翔的波波", dir: path.join(ROOT, "飞翔的波波html") },
  { author: "跨境电商长期主义", dir: path.join(ROOT, "跨境电商长期主义html") },
];

function usage() {
  console.log(`Usage:
  node tools/openhuman-amazon-kb.mjs prepare
  node tools/openhuman-amazon-kb.mjs import [--rpc ${DEFAULT_RPC}] [--namespace ${DEFAULT_NAMESPACE}]
  node tools/openhuman-amazon-kb.mjs verify --query <text> [--rpc ${DEFAULT_RPC}] [--namespace ${DEFAULT_NAMESPACE}]

Environment:
  OPENHUMAN_CORE_TOKEN must match the token used to start openhuman-core.`);
}

function argValue(args, name, fallback = undefined) {
  const index = args.indexOf(name);
  if (index === -1) return fallback;
  const value = args[index + 1];
  if (!value || value.startsWith("--")) {
    throw new Error(`Missing value for ${name}`);
  }
  return value;
}

function decodeHtml(value) {
  const named = {
    amp: "&",
    lt: "<",
    gt: ">",
    quot: '"',
    apos: "'",
    nbsp: " ",
    mdash: "-",
    ndash: "-",
  };
  return value.replace(/&(#x?[0-9a-fA-F]+|[a-zA-Z]+);/g, (match, entity) => {
    if (entity[0] === "#") {
      const isHex = entity[1]?.toLowerCase() === "x";
      const raw = entity.slice(isHex ? 2 : 1);
      const codePoint = Number.parseInt(raw, isHex ? 16 : 10);
      return Number.isFinite(codePoint) ? String.fromCodePoint(codePoint) : match;
    }
    return named[entity] ?? match;
  });
}

function stripHtml(html) {
  let body = html.match(/<content\b[^>]*>([\s\S]*?)<\/content>/i)?.[1];
  if (!body) body = html.match(/<body\b[^>]*>([\s\S]*?)<\/body>/i)?.[1] ?? html;

  body = body
    .replace(/<script\b[\s\S]*?<\/script>/gi, "")
    .replace(/<style\b[\s\S]*?<\/style>/gi, "")
    .replace(/<img\b[^>]*(?:alt=["']([^"']*)["'][^>]*)?>/gi, (_m, alt) =>
      alt ? `\n[图片：${decodeHtml(alt)}]\n` : "\n",
    )
    .replace(/<br\s*\/?>/gi, "\n")
    .replace(/<\/(p|section|div|h[1-6]|li|blockquote|tr)>/gi, "\n")
    .replace(/<li\b[^>]*>/gi, "\n- ")
    .replace(/<[^>]+>/g, "");

  return decodeHtml(body)
    .replace(/\r/g, "")
    .replace(/\u00a0/g, " ")
    .replace(/[ \t]+\n/g, "\n")
    .replace(/\n{3,}/g, "\n\n")
    .split("\n")
    .map((line) => line.trim())
    .filter(Boolean)
    .join("\n\n")
    .trim();
}

function safeName(value) {
  return value
    .replace(/[\\/:*?"<>|]/g, "-")
    .replace(/\s+/g, " ")
    .replace(/^\.+|\.+$/g, "")
    .slice(0, 160);
}

function parseFilename(fileName, fallbackAuthor) {
  const base = fileName.replace(/\.html?$/i, "");
  const match = base.match(/^(.+?)_(\d{4}-\d{2}-\d{2})_(.+)$/);
  if (!match) {
    return { author: fallbackAuthor, date: "", title: base };
  }
  return { author: match[1], date: match[2], title: match[3] };
}

function parseDataScript(html) {
  const raw = html.match(/<script>\s*var\s+data\s*=\s*(\{[\s\S]*?\})\s*;\s*<\/script>/i)?.[1];
  if (!raw) return {};
  try {
    return JSON.parse(raw);
  } catch {
    return {};
  }
}

async function listHtmlFiles(dir) {
  const { readdir } = await import("node:fs/promises");
  const names = await readdir(dir);
  return names
    .filter((name) => /\.html?$/i.test(name))
    .sort((a, b) => a.localeCompare(b, "zh-Hans-CN"))
    .map((name) => path.join(dir, name));
}

async function prepare() {
  await mkdir(ARTICLES_DIR, { recursive: true });
  const rows = [];
  const stats = [];

  for (const source of SOURCES) {
    const files = await listHtmlFiles(source.dir);
    let kept = 0;
    let skipped = 0;
    const authorDir = path.join(ARTICLES_DIR, safeName(source.author));
    await mkdir(authorDir, { recursive: true });

    for (const file of files) {
      const html = await readFile(file, "utf8");
      const data = parseDataScript(html);
      const fromName = parseFilename(path.basename(file), source.author);
      const title = (data.title || fromName.title).trim();
      const author = (data.mp || fromName.author || source.author).trim();
      const date = (data.time || fromName.date || "").trim();
      const sourceUrl = html.match(/<h1\b[^>]*>[\s\S]*?<a\b[^>]*href=["']([^"']+)["']/i)?.[1] ?? "";
      const text = stripHtml(html);

      if (text.length < 80) {
        skipped += 1;
        continue;
      }

      const markdown = [
        `# ${title}`,
        "",
        `作者：${author}`,
        date ? `发布时间：${date}` : "",
        sourceUrl ? `原文链接：${sourceUrl}` : "",
        `来源文件：${path.relative(ROOT, file)}`,
        "",
        text,
        "",
      ]
        .filter((part) => part !== "")
        .join("\n");

      const outputName = `${safeName(fromName.date || "unknown")}_${safeName(title)}.md`;
      const outputPath = path.join(authorDir, outputName);
      await writeFile(outputPath, markdown, "utf8");
      rows.push({
        author,
        date,
        title,
        source_path: path.relative(ROOT, file),
        source_url: sourceUrl,
        markdown_path: path.relative(ROOT, outputPath),
        chars: markdown.length,
      });
      kept += 1;
    }

    stats.push({ author: source.author, total: files.length, kept, skipped });
  }

  await writeFile(MANIFEST_PATH, rows.map((row) => JSON.stringify(row)).join("\n") + "\n", "utf8");
  console.log(JSON.stringify({ manifest: path.relative(ROOT, MANIFEST_PATH), total: rows.length, stats }, null, 2));
}

async function rpcCall(rpcUrl, method, params) {
  const token = process.env.OPENHUMAN_CORE_TOKEN;
  if (!token) throw new Error("OPENHUMAN_CORE_TOKEN is required");
  const response = await fetch(rpcUrl, {
    method: "POST",
    headers: {
      "content-type": "application/json",
      authorization: `Bearer ${token}`,
    },
    body: JSON.stringify({ jsonrpc: "2.0", id: Date.now(), method, params }),
  });
  const payload = await response.json();
  if (!response.ok || payload.error) {
    throw new Error(`${method} failed: ${JSON.stringify(payload.error ?? payload)}`);
  }
  return payload.result;
}

async function readManifestRows() {
  const rows = [];
  const rl = createInterface({ input: createReadStream(MANIFEST_PATH, { encoding: "utf8" }) });
  for await (const line of rl) {
    if (line.trim()) rows.push(JSON.parse(line));
  }
  return rows;
}

async function importKb(args) {
  const rpcUrl = argValue(args, "--rpc", DEFAULT_RPC);
  const namespace = argValue(args, "--namespace", DEFAULT_NAMESPACE);
  const rows = await readManifestRows();
  let imported = 0;

  for (const row of rows) {
    const fullPath = path.join(ROOT, row.markdown_path);
    const content = await readFile(fullPath, "utf8");
    await rpcCall(rpcUrl, "openhuman.memory_doc_put", {
      namespace,
      key: row.markdown_path,
      title: `${row.author} ${row.date ? row.date.slice(0, 10) : ""} ${row.title}`.trim(),
      content,
      source_type: "amazon-author-article",
      priority: "high",
      tags: ["amazon", "亚马逊学习", row.author],
      metadata: {
        author: row.author,
        date: row.date,
        source_path: row.source_path,
        source_url: row.source_url,
        chars: row.chars,
      },
      category: "core",
    });
    imported += 1;
    if (imported % 50 === 0 || imported === rows.length) {
      console.log(`imported ${imported}/${rows.length}`);
    }
  }

  const listed = await rpcCall(rpcUrl, "openhuman.memory_doc_list", { namespace });
  const count = Array.isArray(listed?.documents) ? listed.documents.length : undefined;
  console.log(JSON.stringify({ namespace, imported, stored_documents: count }, null, 2));
}

async function verify(args) {
  const rpcUrl = argValue(args, "--rpc", DEFAULT_RPC);
  const namespace = argValue(args, "--namespace", DEFAULT_NAMESPACE);
  const query = argValue(args, "--query");
  const result = await rpcCall(rpcUrl, "openhuman.memory_context_query", {
    namespace,
    query,
    limit: 5,
  });
  console.log(typeof result === "string" ? result : JSON.stringify(result, null, 2));
}

async function main() {
  const [command, ...args] = process.argv.slice(2);
  if (!command || ["-h", "--help", "help"].includes(command)) {
    usage();
    return;
  }
  if (command === "prepare") return prepare();
  if (command === "import") return importKb(args);
  if (command === "verify") return verify(args);
  throw new Error(`Unknown command: ${command}`);
}

main().catch((error) => {
  console.error(error.message);
  process.exit(1);
});
