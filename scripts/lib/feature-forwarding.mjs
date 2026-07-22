// Detects drift between the core crate's default-ON Cargo gates and the gates
// the Tauri shell forwards to its embedded copy of that crate.
//
// Why this exists (#4919): the shell declares `openhuman_core` with
// `default-features = false`, so it does NOT inherit the core's `default` list.
// Every default-ON gate must be forwarded by hand, and nothing enforced that.
// When the two drift, the domain is compiled out of the shipped desktop app and
// the failure is invisible — no build error, no failing test:
//
//   - `voice`  shipped missing from v0.58.19 to v0.61.x. Every `openhuman.voice_*`
//              RPC answered "unknown method"; 56 users, ~93k Sentry events (#4901).
//   - `tokenjuice-treesitter` was never forwarded once since #4123 and failed
//              *soft* — AST compression silently degraded to a heuristic (#4918).
//
// Two of three gates were dropped, by two different authors, one of whom knew
// about the trap and documented it in a comment. A comment is documentation,
// not enforcement.
//
// This is a deliberately narrow TOML reader rather than a general parser: it
// only needs two well-known shapes, and the repo has no TOML dependency for
// Node. It is regex/scanner-based in the same spirit as `checklist-parser.mjs`.

/**
 * Gates the desktop shell intentionally does NOT forward, mapped to why.
 *
 * Adding an entry is a deliberate product decision, not a way to silence the
 * forwarding guard — the reason string is what a future reader (and reviewer)
 * relies on to tell "excluded on purpose" from "forgotten". That ambiguity is
 * exactly what let #4918 sit unnoticed since #4123.
 *
 * Lives here (not in the checker) so both the CI checker and the self-test read
 * the same source of truth — otherwise the self-test can demand a forward the
 * checker legitimately exempts, which is exactly the drift #5084's `tui` gate hit.
 */
export const INTENTIONALLY_NOT_FORWARDED = {
  // 'some-gate': 'Reason it must not ship in the desktop build.',
  tui: 'Terminal UI subcommand (openhuman tui/chat); the desktop app ships its own Tauri UI and never runs the ratatui terminal front-end.',
};

/**
 * Strip TOML `#` comments while respecting quoted strings, so a `#` inside a
 * value (or an issue number in a comment) can't truncate a real line.
 */
export function stripComments(text) {
  const out = [];
  for (const line of text.split(/\r?\n/)) {
    let quote = null;
    let cut = -1;
    for (let i = 0; i < line.length; i++) {
      const ch = line[i];
      if (quote) {
        if (ch === quote && line[i - 1] !== '\\') quote = null;
      } else if (ch === '"' || ch === "'") {
        quote = ch;
      } else if (ch === '#') {
        cut = i;
        break;
      }
    }
    out.push(cut === -1 ? line : line.slice(0, cut));
  }
  return out.join('\n');
}

/**
 * Read a bracketed array starting at `open` (the index of `[`), returning its
 * raw inner text. Scans for the balanced close so multi-line arrays work.
 */
function readArray(text, open) {
  let depth = 0;
  for (let i = open; i < text.length; i++) {
    if (text[i] === '[') depth++;
    else if (text[i] === ']') {
      depth--;
      if (depth === 0) return text.slice(open + 1, i);
    }
  }
  return null;
}

/** Pull the quoted string items out of a raw TOML array body. */
function arrayItems(raw) {
  if (raw === null) return [];
  return [...raw.matchAll(/"([^"]+)"/g)].map(m => m[1]);
}

/**
 * The core crate's default-ON gates: `[features] default = [...]`.
 *
 * Scoped to the `[features]` table so an unrelated `default = [...]` in another
 * table cannot be picked up by mistake.
 */
export function parseCoreDefaultFeatures(coreToml) {
  const text = stripComments(coreToml);
  // NOTE: `[ \t]` not `\s` — `\s` matches newlines, so `^\s*\[features\]` would
  // happily anchor several lines early and slice the section to nothing.
  const header = text.match(/^[ \t]*\[features\][ \t]*$/m);
  if (!header) return [];
  // Bound the search at the next table header so we stay inside [features].
  const rest = text.slice(header.index + header[0].length);
  const nextTable = rest.search(/^[ \t]*\[[^[\]]+\][ \t]*$/m);
  const section = nextTable === -1 ? rest : rest.slice(0, nextTable);
  const defaultAt = section.search(/^[ \t]*default[ \t]*=[ \t]*\[/m);
  if (defaultAt === -1) return [];
  return arrayItems(readArray(section, section.indexOf('[', defaultAt)));
}

/**
 * What the shell forwards on its `openhuman_core` dependency.
 *
 * Returns `{ defaultFeatures, features }`. `defaultFeatures: true` means the
 * shell inherits the core's defaults and forwarding is moot — there is nothing
 * to drift.
 */
export function parseShellForwardedFeatures(shellToml, depName = 'openhuman_core') {
  const text = stripComments(shellToml);
  // `[ \t]` not `\s`, for the same newline-matching reason as above.
  const declAt = text.search(new RegExp(`^[ \\t]*${depName}[ \\t]*=[ \\t]*\\{`, 'm'));
  if (declAt === -1) return null;
  const braceOpen = text.indexOf('{', declAt);
  // Scan to the matching close brace; the inline table spans lines.
  let depth = 0;
  let braceClose = -1;
  for (let i = braceOpen; i < text.length; i++) {
    if (text[i] === '{') depth++;
    else if (text[i] === '}') {
      depth--;
      if (depth === 0) {
        braceClose = i;
        break;
      }
    }
  }
  if (braceClose === -1) return null;
  const decl = text.slice(braceOpen, braceClose + 1);
  const defaultFeatures = !/default-features\s*=\s*false/.test(decl);
  const featuresAt = decl.search(/features\s*=\s*\[/);
  const features =
    featuresAt === -1 ? [] : arrayItems(readArray(decl, decl.indexOf('[', featuresAt)));
  return { defaultFeatures, features };
}

/**
 * Compare the two lists.
 *
 * `allowlist` maps a gate name to the reason it is intentionally NOT forwarded.
 * An intentional exclusion must be explicit and carry a reason, so that
 * "deliberately excluded" and "forgotten" stop looking identical — which is the
 * ambiguity that let #4918 sit unnoticed.
 */
export function diffForwarding({ coreDefaults, shell, allowlist = {} }) {
  if (shell === null) {
    return { ok: false, reason: 'dependency-not-found', missing: [], stale: [], allowed: [] };
  }
  // Inheriting defaults means there is no forwarding list to drift.
  if (shell.defaultFeatures) {
    return { ok: true, reason: 'inherits-defaults', missing: [], stale: [], allowed: [] };
  }
  const forwarded = new Set(shell.features);
  const missing = [];
  const allowed = [];
  for (const gate of coreDefaults) {
    if (forwarded.has(gate)) continue;
    if (Object.prototype.hasOwnProperty.call(allowlist, gate)) allowed.push(gate);
    else missing.push(gate);
  }
  // A gate that is allow-listed AND forwarded is a contradiction: the allow-list
  // entry is stale and would mask a real drop if the gate were later removed.
  const stale = Object.keys(allowlist).filter(gate => forwarded.has(gate));
  return { ok: missing.length === 0 && stale.length === 0, reason: null, missing, stale, allowed };
}

export function formatReport(result, { coreDefaults, shell, allowlist = {} }) {
  if (result.reason === 'dependency-not-found') {
    return 'FAIL: could not find the `openhuman_core` dependency in the shell manifest.\nThe guard cannot verify forwarding — fix the parser or the manifest.';
  }
  if (result.reason === 'inherits-defaults') {
    return 'OK: the shell inherits the core default features (no `default-features = false`), so no forwarding is required.';
  }
  const lines = [
    `Core default gates (${coreDefaults.length}): ${coreDefaults.join(', ') || '(none)'}`,
    `Shell forwards (${shell.features.length}): ${shell.features.join(', ') || '(none)'}`,
  ];
  for (const gate of result.allowed) {
    lines.push(`  allowed: ${gate} — ${allowlist[gate]}`);
  }
  if (result.stale.length > 0) {
    lines.push('', 'Stale allow-list entries (gate is forwarded, so the entry is wrong):');
    for (const gate of result.stale) lines.push(`  - ${gate}`);
  }
  if (result.missing.length > 0) {
    lines.push('', 'Default-ON core gates NOT forwarded by the desktop shell:');
    for (const gate of result.missing) lines.push(`  - ${gate}`);
    lines.push(
      '',
      'Each of these is compiled OUT of the shipped desktop app, silently.',
      'Fix by adding the gate to the `openhuman_core` features list in',
      'app/src-tauri/Cargo.toml — or, if the exclusion is deliberate, add it to',
      'INTENTIONALLY_NOT_FORWARDED in scripts/ci/check-feature-forwarding.mjs',
      'with a reason. See #4901 (voice) and #4918 (tokenjuice-treesitter).'
    );
  }
  if (result.ok)
    lines.push('', 'OK: every default-ON core gate is forwarded to the desktop shell.');
  return lines.join('\n');
}
