// A provider-local neutral message is not OpenHuman's durable chat record.
export function isClaudeCodeBridgeMessage(
  path,
  name,
  bridgeCode,
  reference = "",
) {
  if (
    name !== "ChatMessage" ||
    !path
      .replaceAll("\\", "/")
      .startsWith(
        "vendor/tinyagents/crates/tinyagents-harness/src/providers/claude_code/",
      )
  )
    return false;
  if (/\bopenhuman(?:_[A-Za-z0-9_]+)?\s*::/.test(reference)) return false;
  const body = bridgeCode.match(/\bstruct\s+ChatMessage\s*\{([^}]*)\}/)?.[1];
  // Fail closed if this reviewed wire DTO grows product identity/state fields.
  return (
    body?.replace(/\s+/g, "") ===
    "pub(crate)role:String,pub(crate)content:String,"
  );
}
