/**
 * OpenClaw Configuration Loader
 *
 * Loads all 7 ZeroClaw-compliant workspace files as raw markdown strings
 * and builds a unified context block for injection into user messages.
 *
 * Matches the Rust backend's `build_system_prompt()` approach exactly:
 * each file is injected as a `### FILENAME` section within `## Project Context`.
 */
import agentsMd from '../../../src-tauri/ai/AGENTS.md?raw';
import bootstrapMd from '../../../src-tauri/ai/BOOTSTRAP.md?raw';
import identityMd from '../../../src-tauri/ai/IDENTITY.md?raw';
import memoryMd from '../../../src-tauri/ai/MEMORY.md?raw';
import soulMd from '../../../src-tauri/ai/SOUL.md?raw';
import toolsMd from '../../../src-tauri/ai/TOOLS.md?raw';
import userMd from '../../../src-tauri/ai/USER.md?raw';

const MAX_CHARS = 20_000;

const OPENCLAW_FILES = [
  { name: 'SOUL.md', content: soulMd },
  { name: 'IDENTITY.md', content: identityMd },
  { name: 'AGENTS.md', content: agentsMd },
  { name: 'USER.md', content: userMd },
  { name: 'BOOTSTRAP.md', content: bootstrapMd },
  { name: 'MEMORY.md', content: memoryMd },
  { name: 'TOOLS.md', content: toolsMd },
] as const;

/**
 * Returns true if a file has meaningful content (not just a TODO template).
 */
function hasContent(content: string): boolean {
  const trimmed = content.trim();
  if (!trimmed) return false;

  // Skip files that are just TODO templates
  const lines = trimmed.split('\n').filter(l => l.trim().length > 0);
  if (lines.length <= 3) return false;

  // If the first non-heading line is "TODO:", it's a placeholder
  const firstContentLine = lines.find(l => !l.startsWith('#'));
  if (firstContentLine && firstContentLine.trim().startsWith('TODO:')) return false;

  return true;
}

/**
 * Build the full OpenClaw context string from all workspace files.
 * Matches ZeroClaw's format: each file as a ### section under ## Project Context.
 * Empty/TODO-only files are skipped. Total output is truncated at MAX_CHARS.
 */
export function buildOpenClawContext(): string {
  const sections: string[] = [];

  for (const file of OPENCLAW_FILES) {
    if (!hasContent(file.content)) continue;
    sections.push(`### ${file.name}\n\n${file.content.trim()}`);
  }

  if (sections.length === 0) return '';

  let context = `## Project Context\n\n${sections.join('\n\n---\n\n')}`;

  if (context.length > MAX_CHARS) {
    context = context.slice(0, MAX_CHARS) + '\n\n[...truncated]';
  }

  return context;
}
