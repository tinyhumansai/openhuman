#!/usr/bin/env node

import { readFile, readdir, writeFile } from "node:fs/promises";
import { dirname, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { isClaudeCodeBridgeMessage } from "../lib/runtime-boundary-types.mjs";
import { sanctionedSdkReexportLines } from "../lib/agent-sdk-contracts.mjs";

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), "../..");
const baselinePath = resolve(
  repoRoot,
  "scripts/ci/agent-runtime-boundary-baseline.json",
);
const openhumanCratesRoot = resolve(repoRoot, "crates");
const tinyagentsRoot = resolve(repoRoot, "vendor/tinyagents/crates");
const writeBaseline = process.argv.includes("--write-baseline");
const noBaseline = process.argv.includes("--no-baseline");

// Embed's injection API exposes the original provider/policy contracts (#7306)
// so callers need only the facade dependency. These are exact declarations,
// not alternative implementations or compatibility layers. Keep the allowance
// bounded by both file and the complete statement, including grouped members.
const embedContractExports = new Map(
  [
    [
      "crates/openhuman-core/src/agent/host_overrides.rs",
      ["pub use tinyagents_harness::cancel::CancellationToken;"],
    ],
    [
      "crates/openhuman-embed/src/config.rs",
      [
        "pub use tinytools::{DefaultEffect, Patterns, RuleEffect, Surface, ToolMatcher, ToolRule, ToolRules};",
      ],
    ],
    [
      "crates/openhuman-embed/src/lib.rs",
      [
        "pub use tinytools::ToolPolicy;",
        "pub use tinyinference_llm::message::MessageDelta;",
        "pub use tinyinference_llm::model::{ChatModel, DeferredHandle, DeferredStatus, ModelProfile, ModelRequest, ModelResponse, ModelStream, ModelStreamItem, ModelStreamMetadata};",
        "pub use tinyinference_llm::{Error, Result};",
      ],
    ],
  ].map(([path, statements]) => [
    path,
    new Set(statements.map(normalizeExport)),
  ]),
);

function normalizeExport(statement) {
  return statement.replace(/\s+/g, "").replace(/,}/g, "}");
}

function isEmbedContractExport(path, lines, index) {
  const allowed = embedContractExports.get(path);
  if (!allowed) return false;
  const remaining = lines
    .slice(index)
    .map(({ text }) => text)
    .join("\n");
  const end = remaining.indexOf(";");
  return end >= 0 && allowed.has(normalizeExport(remaining.slice(0, end + 1)));
}

// Product types need not start with `OpenHuman`, so this is an explicit,
// reviewed inventory rather than a naming convention.
const openhumanDomainTypes = new Set([
  "AgentDefinitionDisplay",
  "AgentDefinitionModel",
  "AgentProgress",
  "ApprovalDecision",
  "ChatMessage",
  "ConversationMessage",
  "ConversationMessagePatch",
  "ConversationMessageRecord",
  "DomainEvent",
  "IntegrationAccount",
  "OpenHumanConfig",
  "OpenHumanRunContext",
  "ToolExecutionResult",
  "ToolResultMessage",
  "TurnOrigin",
  "UserProfileHook",
  "WorkspacePolicy",
]);

// Names moved out of compatibility layers. Values identify their sole owner;
// declarations in any other TinyAgents crate are facades even if private.
const movedSymbolOwners = new Map([
  ["Tool", "tinytools"],
  ["ToolResult", "tinytools"],
  ["ToolPolicy", "tinytools"],
  ["ToolSchema", "tinytools"],
  ["ToolCall", "tinytools"],
  ["ToolCallOptions", "tinytools"],
  ["ToolRunContext", "tinytools"],
  ["WorkspaceDescriptor", "tinytools"],
  ["ToolContent", "tinytools"],
  ["ToolRegistry", "tinytools"],
  ["ChatModel", "tinyinference-llm"],
  ["ModelRequest", "tinyinference-llm"],
  ["ModelResponse", "tinyinference-llm"],
  ["Message", "tinyinference-llm"],
  ["MessageDelta", "tinyinference-llm"],
  ["Usage", "tinyinference-llm"],
  ["UsageTotals", "tinyinference-llm"],
  ["AgentHarness", "tinyagents-harness"],
  ["AgentTurnRequest", "tinyagents-harness"],
  ["HostCapabilities", "tinyagents-harness"],
  ["RunConfig", "tinyagents-harness"],
  ["RunContext", "tinyagents-harness"],
  ["CancellationToken", "tinyagents-harness"],
  ["RetryPolicy", "tinyagents-harness"],
  ["FallbackPolicy", "tinyagents-harness"],
]);

const bridgeSymbols = [
  "SharedToolAdapter",
  "ToolAdapter",
  "spec_to_schema",
  "execute_openhuman_tool",
  "assemble_turn_harness",
  "run_turn_via_tinyagents_shared",
];
const deletedTaskLocalModules = [
  "fork_context",
  "sandbox_context",
  "spawn_depth_context",
  "task_recency_context",
  "turn_attachments_context",
  "turn_dispatch_guard",
  "turn_subagent_usage",
  "resolved_route",
  "run_cancellation_context",
  "thread_context",
];
const taskLocalAccessors = [
  "current_parent",
  "with_parent_context",
  "current_sandbox_mode",
  "with_current_sandbox_mode",
  "current_spawn_depth",
  "with_spawn_depth",
  "current_task_recency_window",
  "with_task_recency_window",
  "current_turn_image_placeholders",
  "with_current_turn_image_placeholders",
  "with_dispatch_guard",
  "record_subagent_usage",
  "with_turn_collector",
  "current_progress_sink",
  "with_progress_sink",
  "with_resolved_provider_route_scope",
  "record_resolved_provider_route",
  "current_route_slot",
  "with_route_slot",
  "current_resolved_provider_route",
  "current_run_cancellation",
  "with_run_cancellation",
  "current_thread_id",
  "with_thread_id",
  "current_is_user_authored",
  "with_origin",
  "with_inherited_origin",
  "current_request_id",
];
const taskLocalStatics = [
  "PARENT_CONTEXT",
  "CURRENT_AGENT_SANDBOX_MODE",
  "TASK_RECENCY_WINDOW",
  "CURRENT_TURN_IMAGE_PLACEHOLDERS",
  "AGENT_PROGRESS_SINK",
  "RESOLVED_PROVIDER_ROUTE",
  "CURRENT_RUN_CANCELLATION",
  "THREAD_ID",
  "AGENT_TURN_ORIGIN",
  "AGENT_TURN_WORKSPACE",
];

async function filesUnder(root, predicate) {
  const found = [];
  async function visit(directory) {
    for (const entry of await readdir(directory, { withFileTypes: true })) {
      const path = resolve(directory, entry.name);
      if (entry.isDirectory()) await visit(path);
      else if (predicate(path)) found.push(path);
    }
  }
  await visit(root);
  return found.sort();
}

function isTestPath(path) {
  return /(?:^|\/)(?:tests?|examples)(?:\/|$)|(?:_tests?|test)\.rs$/.test(path);
}

function escapeRegex(value) {
  return value.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

// Mask comments and literals while retaining line locations. Rust permits
// nested block comments and raw strings, so both are handled explicitly.
function rustCode(source) {
  let result = "";
  let index = 0;
  let blockDepth = 0;
  let state = "code";
  let rawHashes = "";
  const blank = (char) => (char === "\n" || char === "\r" ? char : " ");
  while (index < source.length) {
    const char = source[index];
    const next = source[index + 1];
    if (state === "line") {
      result += blank(char);
      if (char === "\n") state = "code";
      index += 1;
      continue;
    }
    if (state === "block") {
      if (char === "/" && next === "*") {
        blockDepth += 1;
        result += "  ";
        index += 2;
      } else if (char === "*" && next === "/") {
        blockDepth -= 1;
        result += "  ";
        index += 2;
        if (blockDepth === 0) state = "code";
      } else {
        result += blank(char);
        index += 1;
      }
      continue;
    }
    if (state === "quoted" || state === "char") {
      result += blank(char);
      if (char === "\\") {
        if (next !== undefined) {
          result += blank(next);
          index += 2;
        } else index += 1;
      } else {
        if (
          (state === "quoted" && char === '"') ||
          (state === "char" && char === "'")
        )
          state = "code";
        index += 1;
      }
      continue;
    }
    if (state === "raw") {
      if (
        char === '"' &&
        source.slice(index + 1, index + 1 + rawHashes.length) === rawHashes
      ) {
        result += " ".repeat(rawHashes.length + 1);
        index += rawHashes.length + 1;
        state = "code";
      } else {
        result += blank(char);
        index += 1;
      }
      continue;
    }
    if (char === "/" && next === "/") {
      result += "  ";
      index += 2;
      state = "line";
    } else if (char === "/" && next === "*") {
      result += "  ";
      index += 2;
      blockDepth = 1;
      state = "block";
    } else if (char === '"') {
      result += " ";
      index += 1;
      state = "quoted";
    } else if (char === "'" && isRustCharLiteral(source, index)) {
      result += " ";
      index += 1;
      state = "char";
    } else if (char === "r") {
      const raw = source.slice(index).match(/^r(#{0,255})"/);
      if (raw) {
        result += " ".repeat(raw[0].length);
        index += raw[0].length;
        rawHashes = raw[1];
        state = "raw";
      } else {
        result += char;
        index += 1;
      }
    } else {
      result += char;
      index += 1;
    }
  }
  return result;
}

function isRustCharLiteral(source, start) {
  const first = source.codePointAt(start + 1);
  if (first === undefined) return false;
  if (first === 0x5c) {
    const escapeStart = start + 2;
    const escape = source[escapeStart];
    if (escape === undefined) return false;
    if ("\\'\"nrt0".includes(escape)) return source[escapeStart + 1] === "'";
    if (escape === "x")
      return /^[0-9a-fA-F]{2}'/.test(source.slice(escapeStart + 1));
    if (escape === "u") {
      const close = source.indexOf("}", escapeStart + 1);
      return (
        close >= 0 &&
        /^\{[0-9a-fA-F_]+\}$/.test(source.slice(escapeStart + 1, close + 1)) &&
        source[close + 1] === "'"
      );
    }
    return false;
  }
  const width = String.fromCodePoint(first).length;
  return source[start + 1 + width] === "'";
}

function codeLines(source) {
  const code = rustCode(source).split(/\r?\n/);
  const raw = source.split(/\r?\n/);
  return code.map((text, index) => ({
    line: index + 1,
    text,
    raw: raw[index] ?? "",
  }));
}

function compactRustPath(text) {
  return text.replace(/\s*::\s*/g, "::");
}

function isUpstreamPath(text) {
  return /\b(?:tinyagents(?:_[a-z0-9_]+)?|tinytools(?:_[a-z0-9_]+)?|tinyinference(?:_[a-z0-9_]+)?)\s*::/i.test(
    text,
  );
}

function isDependencyTable(header) {
  return /(?:^|\.)(?:dev-|build-)?dependencies\s*$/i.test(header.trim());
}

function nestedDependencyName(header) {
  const match = header.match(
    /(?:^|\.)(?:dev-|build-)?dependencies\s*\.\s*([A-Za-z0-9_-]+)\s*$/i,
  );
  return match?.[1];
}

function tomlWithoutComment(line) {
  let quoted = false;
  let escaped = false;
  for (let index = 0; index < line.length; index += 1) {
    const char = line[index];
    if (char === '"' && !escaped) quoted = !quoted;
    if (char === "#" && !quoted) return line.slice(0, index);
    escaped = char === "\\" && !escaped;
    if (char !== "\\") escaped = false;
  }
  return line;
}

function cargoOpenhumanDependencyLines(source) {
  const findings = [];
  let table = "";
  let nestedDependency = "";
  for (const [offset, raw] of source.split(/\r?\n/).entries()) {
    const line = tomlWithoutComment(raw);
    const tableMatch = line.match(/^\s*\[([^\]]+)]\s*$/);
    if (tableMatch) {
      table = tableMatch[1].replace(/\s+/g, "");
      nestedDependency = nestedDependencyName(table) ?? "";
      if (/^openhuman(?:[-_][A-Za-z0-9_-]+)?$/i.test(nestedDependency))
        findings.push({ line: offset + 1, text: raw });
      continue;
    }
    if (!isDependencyTable(table) && !nestedDependency) continue;
    const keyMatch = line.match(/^\s*([A-Za-z0-9_-]+)\s*=/);
    const key = keyMatch?.[1] ?? "";
    const namesOpenhuman = /^openhuman(?:[-_][A-Za-z0-9_-]+)?$/i.test(key);
    const aliasesOpenhuman =
      /\bpackage\s*=\s*["']openhuman(?:[-_][A-Za-z0-9_-]+)?["']/i.test(line);
    if (namesOpenhuman || aliasesOpenhuman)
      findings.push({ line: offset + 1, text: raw });
  }
  return findings;
}

function tinyagentsCrateName(path) {
  return relative(tinyagentsRoot, path).match(/^([^/\\]+)/)?.[1] ?? "";
}

function isBehaviorFreeForwarder(source) {
  let remaining = rustCode(source);
  remaining = remaining.replace(/^\s*#\s*\[[^\n]*]\s*$/gm, "");
  remaining = remaining.replace(
    /(?:pub(?:\s*\([^)]*\))?\s+)?use\s+[\s\S]*?;/g,
    "",
  );
  return !/[A-Za-z0-9_]/.test(remaining);
}

function assertSelfTests() {
  const embedPath = "crates/openhuman-embed/src/lib.rs";
  const policyExport = [{ text: "pub use tinytools::ToolPolicy;" }];
  if (
    !isEmbedContractExport(embedPath, policyExport, 0) ||
    isEmbedContractExport("crates/openhuman-core/src/lib.rs", policyExport, 0) ||
    isEmbedContractExport(
      embedPath,
      [{ text: "pub use tinytools::{ToolPolicy, ToolRegistry};" }],
      0,
    )
  )
    throw new Error(
      "agent-runtime boundary self-test: host tool policy export must stay exact",
    );
  const sdkPath = "crates/openhuman-core/src/agent/tinyagents/budget.rs";
  const sdkStatement =
    "pub use tinyinference_llm::model::budget::{Budget, BudgetExceeded, BudgetSnapshot, CallBudget, Spend, SpendLimits};";
  if (
    sanctionedSdkReexportLines(sdkPath, sdkStatement).size !== 1 ||
    sanctionedSdkReexportLines(
      sdkPath,
      sdkStatement.replace("SpendLimits", "SpendLimits, BudgetedModel"),
    ).size !== 0 ||
    sanctionedSdkReexportLines(
      sdkPath,
      "pub use tinyagents_harness::runtime::AgentHarness;",
    ).size !== 0
  )
    throw new Error(
      "agent-runtime boundary self-test: host SDK inventory leaked runtime contracts",
    );
  const fixture = [
    "[dependencies.openhuman]",
    'version = "1"',
    "[target.'cfg(unix)'.dev-dependencies]",
    'host = { package = "openhuman-core", version = "1" }',
    "[workspace.dependencies]",
    'openhuman_tools = "1"',
    "[build-dependencies]",
    'helper = { package = "openhuman" }',
    "[dev-dependencies.host]",
    "package = 'openhuman'",
  ].join("\n");
  if (cargoOpenhumanDependencyLines(fixture).length !== 5)
    throw new Error(
      "agent-runtime boundary self-test: Cargo dependency form escaped detection",
    );
  if (
    !["ChatMessage", "ConversationMessage", "ToolResultMessage"].every((name) =>
      openhumanDomainTypes.has(name),
    )
  )
    throw new Error(
      "agent-runtime boundary self-test: durable transcript inventory regressed",
    );
  const masked = rustCode(
    "/* openhuman :: AgentProgress */ let x = openhuman :: AgentProgress; // tinytools::Tool",
  );
  if (
    !compactRustPath(masked).includes("openhuman::AgentProgress") ||
    masked.includes("tinytools::Tool")
  )
    throw new Error(
      "agent-runtime boundary self-test: Rust masking/path normalization regressed",
    );
  const lifetimeMasked = rustCode(
    "fn borrow<'a>(message: &'a ChatMessage) { 'retry: loop { let _ = openhuman :: ChatMessage; break 'retry; } }",
  );
  if (
    !compactRustPath(lifetimeMasked).includes("openhuman::ChatMessage") ||
    !lifetimeMasked.includes("&'a ChatMessage")
  )
    throw new Error(
      "agent-runtime boundary self-test: Rust lifetimes or labels hid code",
    );
  const literalMasked = rustCode(
    "let marker = 'x'; let text = \"openhuman :: ChatMessage\"; // openhuman :: ChatMessage",
  );
  if (
    literalMasked.includes("openhuman") ||
    literalMasked.includes("marker = 'x'")
  )
    throw new Error(
      "agent-runtime boundary self-test: Rust literals or comments leaked into scan",
    );
  if (!isBehaviorFreeForwarder("pub(crate) use tinytools::Tool;\n"))
    throw new Error(
      "agent-runtime boundary self-test: private forwarding module escaped detection",
    );
  const beforeMove = stable([
    {
      rule: "fixture",
      path: "fixture.rs",
      line: 7,
      text: "use openhuman::ChatMessage;",
    },
  ]);
  const afterMove = stable([
    {
      rule: "fixture",
      path: "fixture.rs",
      line: 8,
      text: "use openhuman::ChatMessage;",
    },
  ]);
  if (JSON.stringify(beforeMove) === JSON.stringify(afterMove))
    throw new Error(
      "agent-runtime boundary self-test: physical moves escaped the exact baseline",
    );
}

assertSelfTests();

const findings = [];
const claudeCodeBridge = rustCode(
  await readFile(
    resolve(
      tinyagentsRoot,
      "tinyagents-harness/src/providers/claude_code/bridge.rs",
    ),
    "utf8",
  ),
);
function add(rule, path, line, text) {
  findings.push({
    rule,
    path: relative(repoRoot, path),
    line,
    text: text.trim(),
  });
}

// Scan every OpenHuman crate: facades are not restricted to agent/.
for (const path of await filesUnder(
  openhumanCratesRoot,
  (path) => path.endsWith(".rs") && /\/src\//.test(path),
)) {
  const rel = relative(repoRoot, path);
  const source = await readFile(path, "utf8");
  const sdkContractLines = sanctionedSdkReexportLines(rel, rustCode(source));
  const lines = codeLines(source);
  for (const [index, { line, text, raw }] of lines.entries()) {
    if (!text.trim()) continue;
    const compact = compactRustPath(text);
    if (/\btinyagents_harness::tool_calling\b/.test(compact))
      add("openhuman-tool-calling-facade", path, line, raw);
    if (
      /\bcrate::agent::(?:dispatcher|pformat|harness::(?:parse|instructions|tool_filter))\b/.test(
        compact,
      )
    )
      add("openhuman-agent-facade", path, line, raw);
    for (const symbol of bridgeSymbols)
      if (new RegExp(`\\b${escapeRegex(symbol)}\\b`).test(text))
        add("openhuman-runtime-bridge", path, line, raw);
    if (!isTestPath(rel)) {
      const isListedTaskLocalFile =
        /\/(?:fork_context|sandbox_context|spawn_depth_context|task_recency_context|turn_attachments_context|turn_dispatch_guard|turn_subagent_usage|progress_sink|resolved_route|run_cancellation_context|thread_context|turn_origin|turn_workspace)\.rs$/.test(
          path,
        );
      const namesTaskLocalAccessor = taskLocalAccessors.some(
        (name) =>
          new RegExp(`\\b${escapeRegex(name)}\\s*\\(`).test(text) ||
          (/\b(?:pub(?:\([^)]*\))?\s+)?use\b/.test(text) &&
            new RegExp(`\\b${escapeRegex(name)}\\b`).test(text)),
      );
      const namesTaskLocalStatic = taskLocalStatics.some((name) =>
        new RegExp(`\\b${escapeRegex(name)}\\b`).test(text),
      );
      const usesGenericAccessor =
        /\b(?:turn_origin|turn_workspace|turn_dispatch_guard)::(?:current|capture|propagate|spawn|spawn_unlabelled|check|record_subagent_elapsed|with_workspace)\b/.test(
          compact,
        ) ||
        (/\/turn_workspace\.rs$/.test(path) &&
          /\bwith_workspace\s*\(/.test(text));
      const declaresDeletedModule = deletedTaskLocalModules.some((name) =>
        new RegExp(`\\bmod\\s+${escapeRegex(name)}\\s*;`).test(text),
      );
      if (
        (isListedTaskLocalFile && /\btokio\s*::\s*task_local!/.test(text)) ||
        namesTaskLocalAccessor ||
        namesTaskLocalStatic ||
        usesGenericAccessor ||
        declaresDeletedModule
      )
        add("openhuman-task-local", path, line, raw);
    }
    if (
      /\bpub(?:\s*\([^)]*\))?\s+use\s+(?:tinyagents(?:_[a-z_]+)?|tinytools(?:_[a-z_]+)?|tinyinference(?:_[a-z_]+)?)\s*::/i.test(
        text,
      ) &&
      !sdkContractLines.has(line) &&
      !isEmbedContractExport(rel, lines, index)
    )
      add("openhuman-upstream-reexport", path, line, raw);
  }
}

for (const path of await filesUnder(
  tinyagentsRoot,
  (path) => path.endsWith(".rs") || path.endsWith("Cargo.toml"),
)) {
  const source = await readFile(path, "utf8");
  if (path.endsWith("Cargo.toml")) {
    for (const item of cargoOpenhumanDependencyLines(source))
      add("tinyagents-openhuman-dependency", path, item.line, item.text);
    continue;
  }
  const crateName = tinyagentsCrateName(path);
  const lines = codeLines(source);
  let firstForwardingLine;
  for (const { line, text, raw } of lines) {
    if (!text.trim()) continue;
    const compact = compactRustPath(text);
    if (/\b(?:pub(?:\s*\([^)]*\))?\s+)?mod\s+tool_calling\s*;/.test(text))
      add("tinyagents-tool-calling-facade", path, line, raw);
    if (
      /\bpub(?:\s*\([^)]*\))?\s+use\s+(?:tinyagents(?:_[a-z_]+)?|tinytools(?:_[a-z_]+)?|tinyinference(?:_[a-z_]+)?)\s*::/i.test(
        text,
      )
    )
      add("tinyagents-upstream-reexport", path, line, raw);
    if (
      /\b(?:pub(?:\s*\([^)]*\))?\s+)?use\s+tinytools_agent(?:\s*::|\s*;)/.test(
        text,
      )
    )
      add("tinyagents-tool-calling-facade", path, line, raw);
    if (isUpstreamPath(text) && firstForwardingLine === undefined)
      firstForwardingLine = { line, raw };
    const declaration = text.match(
      /\b(?:pub(?:\s*\([^)]*\))?\s+)?(?:type|struct|enum|trait|fn)\s+([A-Za-z_][A-Za-z0-9_]*)\b/,
    );
    if (declaration) {
      const owner = movedSymbolOwners.get(declaration[1]);
      if (owner && !crateName.startsWith(owner))
        add("tinyagents-moved-symbol-alias", path, line, raw);
    }
    if (
      /\b(?:pub(?:\s*\([^)]*\))?\s+)?type\s+[A-Za-z_][A-Za-z0-9_]*[^=]*=\s*(?:tinyagents(?:_[a-z_]+)?|tinytools(?:_[a-z_]+)?|tinyinference(?:_[a-z_]+)?)\s*::/i.test(
        text,
      ) ||
      /\b(?:pub(?:\s*\([^)]*\))?\s+)?struct\s+[A-Za-z_][A-Za-z0-9_]*\s*\(\s*(?:tinyagents(?:_[a-z_]+)?|tinytools(?:_[a-z_]+)?|tinyinference(?:_[a-z_]+)?)\s*::/i.test(
        text,
      )
    )
      add("tinyagents-moved-symbol-alias", path, line, raw);
    if (/\bopenhuman(?:\s*::|_[A-Za-z0-9_]*\s*::)/i.test(compact))
      add("tinyagents-openhuman-domain-name", path, line, raw);
    for (const name of openhumanDomainTypes) {
      if (
        isClaudeCodeBridgeMessage(
          relative(repoRoot, path),
          name,
          claudeCodeBridge,
          text,
        )
      )
        continue;
      if (new RegExp(`\\b${escapeRegex(name)}\\b`).test(text)) {
        add("tinyagents-openhuman-domain-name", path, line, raw);
        break;
      }
    }
  }
  if (firstForwardingLine && isBehaviorFreeForwarder(source))
    add(
      "tinyagents-forwarding-module",
      path,
      firstForwardingLine.line,
      firstForwardingLine.raw,
    );
}

findings.sort(
  (a, b) =>
    a.rule.localeCompare(b.rule) ||
    a.path.localeCompare(b.path) ||
    a.line - b.line ||
    a.text.localeCompare(b.text),
);

function stable(records) {
  const occurrences = new Map();
  return records.map(({ rule, path, line, text }) => {
    // Deliberately excludes `line`. The identity of a violation is the rule it
    // breaks, the file it is in, and the source text — not where in the file it
    // sits. Keying on the line answers "did anything ABOVE this change?", which
    // is a proximity alarm rather than debt tracking, and it made the gate red
    // on every unrelated edit to a file containing a violation (#6525).
    //
    // `occurrence` carries the load the line was credited with: a file that
    // gains a SECOND identical violation goes 1 -> 2 and is correctly reported
    // as unbaselined. `line` is retained on the record as advisory metadata for
    // the error output, and is refreshed whenever the baseline is regenerated.
    const base = `${rule}\0${path}\0${text}`;
    const occurrence = (occurrences.get(base) ?? 0) + 1;
    occurrences.set(base, occurrence);
    return { rule, path, line, text, occurrence };
  });
}

if (writeBaseline) {
  await writeFile(
    baselinePath,
    `${JSON.stringify(stable(findings), null, 2)}\n`,
  );
  console.log(
    `Wrote ${findings.length} exact agent-runtime boundary violations to ${relative(repoRoot, baselinePath)}.`,
  );
  process.exit(0);
}

let expected = [];
if (!noBaseline) {
  try {
    expected = JSON.parse(await readFile(baselinePath, "utf8"));
  } catch (error) {
    console.error(
      `Unable to read ${relative(repoRoot, baselinePath)}: ${error.message}`,
    );
    process.exit(1);
  }
}
// Compared on identity rather than on the whole record: `JSON.stringify` would
// reintroduce `line`, and because the two sets are compared in both directions
// a moved violation would land in `added` AND `stale` at once — which is why a
// set of five that merely shifted used to report as "5 unbaselined + 5 stale",
// inviting a triager to believe ten things had changed.
const identity = ({ rule, path, text, occurrence }) =>
  `${rule}\0${path}\0${text}\0${occurrence}`;
const actualStable = stable(findings);
const expectedKeys = new Set(expected.map(identity));
const actualKeys = new Set(actualStable.map(identity));
const added = findings.filter(
  (_, index) => !expectedKeys.has(identity(actualStable[index])),
);
const stale = expected.filter((record) => !actualKeys.has(identity(record)));
if (added.length || stale.length) {
  console.error("Agent-runtime ownership boundary changed.");
  if (added.length) {
    console.error(`\nUnbaselined violations (${added.length}):`);
    for (const item of added)
      console.error(`  ${item.rule}: ${item.path}:${item.line}: ${item.text}`);
  }
  if (stale.length) {
    console.error(`\nStale baseline entries (${stale.length}); remove them:`);
    for (const item of stale)
      console.error(
        `  ${item.rule}: ${item.path}: ${item.text} [${item.occurrence}]`,
      );
  }
  process.exit(1);
}
console.log(
  `Agent-runtime boundary holds (${findings.length} temporary exact violations baselined).`,
);
