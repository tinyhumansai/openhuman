/** Minimal skill entry for prompt rendering */
interface SkillPromptEntry {
  name: string;
  description: string;
  location?: string;
}

/**
 * Build the available skills section of the system prompt.
 * Matches OpenClaw's <available_skills> XML format.
 */
export function buildSkillsSection(skills: SkillPromptEntry[]): string {
  if (skills.length === 0) return '';

  const parts: string[] = [];

  parts.push('## Skills (mandatory)\n');
  parts.push('Before replying: scan <available_skills> <description> entries.');
  parts.push(
    '- If exactly one skill clearly applies: read its SKILL.md at <location>, then follow it.'
  );
  parts.push('- If multiple could apply: choose the most specific one, then read/follow it.');
  parts.push('- If none clearly apply: do not read any SKILL.md.');
  parts.push('Constraints: never read more than one skill up front; only read after selecting.');
  parts.push('');

  parts.push('<available_skills>');
  for (const skill of skills) {
    parts.push('  <skill>');
    parts.push(`    <name>${skill.name}</name>`);
    parts.push(`    <description>${skill.description}</description>`);
    if (skill.location) {
      parts.push(`    <location>${skill.location}</location>`);
    }
    parts.push('  </skill>');
  }
  parts.push('</available_skills>');
  parts.push('');

  return parts.join('\n');
}
