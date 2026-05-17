import { createMockId } from "../../state.mjs";
import {
  collectMessagesByRole,
  detectModelFamily,
  latestRoleMessage,
} from "./shared.mjs";

function compactWhitespace(text) {
  return String(text || "")
    .replace(/\s+/g, " ")
    .trim();
}

function sentenceParts(text) {
  return compactWhitespace(text)
    .split(/(?<=[.!?])\s+/)
    .map((part) => part.trim())
    .filter(Boolean);
}

function truncate(text, limit = 160) {
  const compact = compactWhitespace(text);
  if (compact.length <= limit) return compact;
  return `${compact.slice(0, Math.max(0, limit - 1)).trimEnd()}…`;
}

function inferLanguage(prompt, thread) {
  const lower = String(prompt || "").toLowerCase();
  if (/typescript|tsx|ts\b/.test(lower)) return "ts";
  if (/javascript|node|react|js\b/.test(lower)) return "js";
  if (/python|pytest/.test(lower)) return "python";
  if (/rust|cargo/.test(lower)) return "rust";
  if (/bash|shell|zsh|sh\b/.test(lower)) return "bash";
  if (/json/.test(lower)) return "json";
  return thread?.lastCodeLanguage || "ts";
}

function makeCodeSnippet(language, prompt, thread) {
  const lower = String(prompt || "").toLowerCase();
  if (language === "python") {
    if (/async/.test(lower)) {
      return [
        "import asyncio",
        "",
        "async def run_task(name: str) -> str:",
        "    await asyncio.sleep(0)",
        '    return f\"done:{name}\"',
      ].join("\n");
    }
    if (/test/.test(lower)) {
      return [
        "def test_run_task_returns_done():",
        '    assert run_task_sync(\"build\") == \"done:build\"',
      ].join("\n");
    }
    return [
      "def run_task_sync(name: str) -> str:",
      '    return f\"done:{name}\"',
    ].join("\n");
  }

  if (language === "rust") {
    if (/async/.test(lower)) {
      return [
        "pub async fn run_task(name: &str) -> String {",
        '    format!("done:{name}")',
        "}",
      ].join("\n");
    }
    return [
      "pub fn run_task(name: &str) -> String {",
      '    format!("done:{name}")',
      "}",
    ].join("\n");
  }

  if (language === "bash") {
    return [
      "#!/usr/bin/env bash",
      "set -euo pipefail",
      'name=\"${1:-build}\"',
      'printf \"done:%s\\n\" \"$name\"',
    ].join("\n");
  }

  if (language === "json") {
    return JSON.stringify(
      {
        task: "mock",
        prompt: truncate(prompt, 48),
        mode: /async/.test(lower) ? "async" : "sync",
      },
      null,
      2,
    );
  }

  const asyncKeyword = language === "ts" ? "async " : "";
  const typeSuffix = language === "ts" ? ": Promise<string>" : "";
  const awaitLine = /async/.test(lower) ? "  await Promise.resolve();\n" : "";
  const testBlock = /test/.test(lower)
    ? `\nexport function runTaskTest() {\n  return runTask("build");\n}\n`
    : "\n";

  return [
    `${asyncKeyword}function runTask(name${language === "ts" ? ": string" : ""})${typeSuffix} {`,
    awaitLine ? awaitLine.trimEnd() : "  return `done:${name}`;",
    awaitLine ? "  return `done:${name}`;" : "",
    "}",
    testBlock.trimEnd(),
  ]
    .filter(Boolean)
    .join("\n");
}

function summarizeText(source, style = "balanced") {
  const sentences = sentenceParts(source);
  if (sentences.length === 0) return "Nothing substantial to summarize yet.";
  if (style === "brief") return truncate(sentences.slice(0, 1).join(" "), 120);
  if (style === "bullets") {
    return sentences
      .slice(0, 3)
      .map((sentence) => `- ${truncate(sentence, 90)}`)
      .join("\n");
  }
  return truncate(sentences.slice(0, 3).join(" "), 240);
}

function buildReasoningSummary(prompt, toolText, thread) {
  const basis = toolText || prompt || thread?.lastUserMessage || "the request";
  return [
    `Recommendation: focus on ${truncate(basis, 80)}.`,
    thread?.turnCount > 0
      ? "This follows the previous turn and keeps the same working context."
      : "This is the first pass, so the answer stays broad and reversible.",
    "If you want, I can tighten this into an implementation plan or a direct patch.",
  ].join(" ");
}

function inferToolCalls(prompt, toolDefs = []) {
  const lower = String(prompt || "").toLowerCase();
  const declared = Array.isArray(toolDefs) ? toolDefs : [];
  const picked = (name, args) => ({
    id: createMockId("tool"),
    name,
    arguments: args,
  });

  if (/search|look up|find|research/.test(lower)) {
    return [
      picked(declared[0]?.function?.name || declared[0]?.name || "web_search", {
        q: truncate(prompt, 120),
      }),
    ];
  }
  if (/read file|open file|inspect file/.test(lower)) {
    return [
      picked(declared[0]?.function?.name || declared[0]?.name || "fs_read", {
        path: "src/example.ts",
      }),
    ];
  }
  if (/run|shell|command|git/.test(lower)) {
    return [
      picked(declared[0]?.function?.name || declared[0]?.name || "shell_exec", {
        cmd: "echo mock-agentic-run",
      }),
    ];
  }
  if (/fetch|download|http|url/.test(lower)) {
    return [
      picked(declared[0]?.function?.name || declared[0]?.name || "http_fetch", {
        url: "https://example.com/mock",
      }),
    ];
  }

  if (declared.length > 0) {
    return [
      picked(declared[0]?.function?.name || declared[0]?.name || "mock_tool", {
        input: truncate(prompt, 120),
      }),
    ];
  }

  return [];
}

function latestToolResult(parsedBody, thread) {
  const toolMessage = latestRoleMessage(parsedBody, "tool");
  return toolMessage?.normalizedContent || thread?.lastToolResult || "";
}

function followUpMode(prompt) {
  const lower = String(prompt || "").toLowerCase();
  return {
    shorter: /shorter|brief|condense|tldr|tl;dr/.test(lower),
    continue: /continue|go on|next/.test(lower),
    expand: /expand|more detail|elaborate/.test(lower),
    async: /async/.test(lower),
    tests: /test|spec/.test(lower),
  };
}

export function buildDynamicCompletion({ model, parsedBody, thread }) {
  const family = detectModelFamily({ model, parsedBody });
  const latestUser = latestRoleMessage(parsedBody, "user");
  const prompt = latestUser?.normalizedContent || "";
  const toolText = latestToolResult(parsedBody, thread);
  const mode = followUpMode(prompt);

  if (family === "summarization") {
    const sourceCandidates = [
      toolText,
      thread?.lastAssistantContent,
      ...collectMessagesByRole(parsedBody, "assistant").map(
        (item) => item.normalizedContent,
      ),
      ...collectMessagesByRole(parsedBody, "user")
        .slice(0, -1)
        .map((item) => item.normalizedContent),
    ].filter(Boolean);
    const source = sourceCandidates.join(" ");
    const content = summarizeText(
      source ||
        prompt ||
        thread?.lastUserMessage ||
        "No prior material was provided.",
      mode.shorter
        ? "brief"
        : /bullet|list/.test(prompt.toLowerCase())
          ? "bullets"
          : "balanced",
    );
    return {
      family,
      content,
      streamScript: [{ text: content }, { finish: "stop" }],
      codeLanguage: null,
    };
  }

  if (family === "coding") {
    const language = inferLanguage(prompt, thread);
    const code = makeCodeSnippet(language, prompt, thread);
    const intro =
      thread?.turnCount > 0
        ? `Updated ${language.toUpperCase()} version based on the previous turn:`
        : `Generated ${language.toUpperCase()} starter:`;
    const content = `${intro}\n\n\`\`\`${language}\n${code}\n\`\`\``;
    return {
      family,
      content,
      streamScript: [
        { text: intro },
        { text: `\n\n\`\`\`${language}\n${code}\n\`\`\`` },
        { finish: "stop" },
      ],
      codeLanguage: language,
    };
  }

  if (family === "agentic") {
    const toolCalls = toolText ? [] : inferToolCalls(prompt, parsedBody?.tools);
    const content = toolText
      ? `Tool result received. The relevant outcome is: ${truncate(
          toolText,
          mode.shorter ? 100 : 220,
        )}${mode.expand ? " I can turn this into a fuller answer if needed." : ""}`
      : "Planning tool work now.";
    return {
      family,
      content,
      toolCalls,
      streamScript:
        toolCalls.length > 0
          ? [
              { text: "Planning tool work now." },
              ...toolCalls.map((toolCall) => ({ toolCall })),
              { finish: "tool_calls" },
            ]
          : [{ text: content }, { finish: "stop" }],
      codeLanguage: null,
    };
  }

  if (family === "reasoning") {
    const content = buildReasoningSummary(prompt, toolText, thread);
    return {
      family,
      content,
      streamScript: [
        {
          thinking: `Assessing the request about ${truncate(prompt || toolText || "the current task", 72)}.`,
        },
        {
          thinking:
            thread?.turnCount > 0
              ? "Incorporating prior turn context."
              : "Choosing a safe first-pass approach.",
        },
        { text: content },
        {
          usage: { prompt_tokens: 18, completion_tokens: 24, total_tokens: 42 },
        },
        { finish: "stop" },
      ],
      codeLanguage: null,
    };
  }

  const content =
    mode.shorter && thread?.lastAssistantContent
      ? truncate(thread.lastAssistantContent, 120)
      : thread?.turnCount > 0 && (mode.continue || mode.expand)
        ? `Continuing from the previous turn: ${truncate(
            thread.lastAssistantContent || toolText || prompt || "ready",
            mode.expand ? 220 : 140,
          )}`
        : "Hello from e2e mock agent";
  return {
    family,
    content,
    streamScript: [{ text: content }, { finish: "stop" }],
    codeLanguage: null,
  };
}
