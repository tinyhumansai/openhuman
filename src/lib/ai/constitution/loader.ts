import defaultConstitutionMd from './default-constitution.md?raw';
import type {
  ConstitutionConfig,
  ConstitutionPrinciple,
  DecisionCriterion,
  MemoryPrinciple,
  ProhibitedAction,
} from './types';

const CONSTITUTION_URL =
  'https://raw.githubusercontent.com/alphahumanxyz/constitution/refs/heads/main/CONSTITUTION.md';

/**
 * Load the constitution from the public GitHub repository.
 * Falls back to the bundled default if the fetch fails (e.g. offline).
 */
export async function loadConstitution(): Promise<ConstitutionConfig> {
  let raw: string;
  let isDefault = false;

  try {
    const response = await fetch(CONSTITUTION_URL);
    if (!response.ok) throw new Error(`HTTP ${response.status}`);
    raw = await response.text();
  } catch {
    // Fetch failed (offline, network error, etc.) — use bundled default
    raw = defaultConstitutionMd;
    isDefault = true;
  }

  return parseConstitution(raw, isDefault);
}

/**
 * Parse a constitution markdown string into structured config.
 */
export function parseConstitution(raw: string, isDefault: boolean): ConstitutionConfig {
  const corePrinciples = parsePrinciples(raw);
  const memoryPrinciples = parseMemoryPrinciples(raw);
  const decisionFramework = parseDecisionFramework(raw);
  const prohibitedActions = parseProhibitedActions(raw);
  const interactionGuidelines = parseInteractionGuidelines(raw);

  return {
    raw,
    corePrinciples,
    memoryPrinciples,
    decisionFramework,
    prohibitedActions,
    interactionGuidelines,
    isDefault,
  };
}

function extractSection(raw: string, heading: string): string {
  const regex = new RegExp(`## ${heading}\\s*\\n([\\s\\S]*?)(?=\\n## |$)`, 'i');
  const match = raw.match(regex);
  return match?.[1]?.trim() ?? '';
}

/** Parse ### N. Title subsections into title/description pairs */
function parseSubsections(section: string): { title: string; description: string }[] {
  const parts = section.split(/(?=###\s+\d+\.)/).filter(s => /^###\s+\d+\./.test(s));
  return parts.map(part => {
    const titleMatch = part.match(/###\s+\d+\.\s*(.+)/);
    const title = titleMatch?.[1]?.trim() ?? '';
    const lines = part.split('\n').slice(1);
    const descLine = lines.find(
      l => l.trim() && !l.trim().startsWith('-') && !l.trim().startsWith('#')
    );
    const description = descLine?.trim() ?? '';
    return { title, description };
  });
}

/** Collect all bullet points from a section */
function collectBullets(section: string): string[] {
  return section
    .split('\n')
    .filter(l => l.startsWith('- '))
    .map(l => l.replace(/^-\s*/, ''));
}

function parsePrinciples(raw: string): ConstitutionPrinciple[] {
  // Old format: ## Core Principles with "1. **Title** — Description"
  const oldSection = extractSection(raw, 'Core Principles');
  if (oldSection) {
    const lines = oldSection.split('\n').filter(l => l.match(/^\d+\./));
    return lines.map((line, i) => {
      const match = line.match(/\*\*(.+?)\*\*\s*[—-]\s*(.+)/);
      return {
        id: `principle-${i + 1}`,
        title: match?.[1] ?? `Principle ${i + 1}`,
        description: match?.[2] ?? line.replace(/^\d+\.\s*/, ''),
      };
    });
  }

  // New format: ## I. Core Values with ### N. Title subsections
  const newSection = extractSection(raw, 'I\\.\\s*Core Values');
  if (newSection) {
    return parseSubsections(newSection).map((sub, i) => ({
      id: `principle-${i + 1}`,
      title: sub.title,
      description: sub.description,
    }));
  }

  return [];
}

function parseMemoryPrinciples(raw: string): MemoryPrinciple[] {
  // Old format: ## Memory Principles with bullet items
  const oldSection = extractSection(raw, 'Memory Principles');
  if (oldSection) {
    return collectBullets(oldSection).map(rule => ({ rule: rule.replace(/\*\*/g, '') }));
  }

  // New format: ## IV. Privacy and Data Responsibility
  const newSection = extractSection(raw, 'IV\\.\\s*Privacy and Data Responsibility');
  if (newSection) {
    return collectBullets(newSection).map(rule => ({ rule }));
  }

  return [];
}

function parseDecisionFramework(raw: string): DecisionCriterion[] {
  // Old format: ## Decision Framework with "1. **Title** — Question"
  const oldSection = extractSection(raw, 'Decision Framework');
  if (oldSection) {
    const lines = oldSection.split('\n').filter(l => l.match(/^\d+\./));
    return lines.map((line, i) => {
      const match = line.match(/\*\*(.+?)\*\*\s*[—-]\s*(.+)/);
      return {
        id: match?.[1]?.toLowerCase() ?? `criterion-${i + 1}`,
        question: match?.[2] ?? line.replace(/^\d+\.\s*/, ''),
      };
    });
  }

  // New format: ## II. Alignment and Decision-Making Principles
  const newSection = extractSection(raw, 'II\\.\\s*Alignment and Decision-Making Principles');
  if (newSection) {
    return parseSubsections(newSection).map(sub => ({
      id: sub.title.toLowerCase(),
      question: sub.description,
    }));
  }

  return [];
}

function parseProhibitedActions(raw: string): ProhibitedAction[] {
  // Old format: ## Prohibited Actions with bullet items
  const oldSection = extractSection(raw, 'Prohibited Actions');
  if (oldSection) {
    return collectBullets(oldSection).map(desc => ({ description: desc }));
  }

  // New format: ## III. Boundaries and Refusals
  const newSection = extractSection(raw, 'III\\.\\s*Boundaries and Refusals');
  if (newSection) {
    return collectBullets(newSection).map(desc => ({ description: desc }));
  }

  return [];
}

function parseInteractionGuidelines(raw: string): string[] {
  // Old format: ## Interaction Guidelines with bullet items
  const oldSection = extractSection(raw, 'Interaction Guidelines');
  if (oldSection) {
    return collectBullets(oldSection);
  }

  // New format: ## V. Agency and Power Use
  const newSection = extractSection(raw, 'V\\.\\s*Agency and Power Use');
  if (newSection) {
    return collectBullets(newSection);
  }

  return [];
}
