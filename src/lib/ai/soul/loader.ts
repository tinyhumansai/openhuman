import soulMd from '../../../../rust-core/ai/SOUL.md?raw';
import type {
  BehaviorPattern,
  EmergencyResponse,
  Interaction,
  MemorySettings,
  PersonalityTrait,
  SafetyRule,
  SoulConfig,
  SoulIdentity,
  VoiceToneGuideline,
} from './types';

const SOUL_GITHUB_URL =
  'https://raw.githubusercontent.com/openhumanxyz/openhuman/refs/heads/main/rust-core/ai/SOUL.md';
const SOUL_CACHE_KEY = 'openhuman.soul.cache';
const SOUL_CACHE_TTL = 1000 * 60 * 30; // 30 minutes

let cachedSoulConfig: SoulConfig | null = null;

/**
 * Load SOUL.md with caching and fallback strategy:
 * 1. Try in-memory cache
 * 2. Try localStorage cache (with TTL)
 * 3. Try GitHub remote
 * 4. Fallback to bundled SOUL.md
 */
export async function loadSoul(): Promise<SoulConfig> {
  // 1. Memory cache
  if (cachedSoulConfig) {
    return cachedSoulConfig;
  }

  // 2. Local storage cache
  try {
    const cached = localStorage.getItem(SOUL_CACHE_KEY);
    if (cached) {
      const parsed = JSON.parse(cached) as { config: SoulConfig; timestamp: number };
      if (Date.now() - parsed.timestamp < SOUL_CACHE_TTL) {
        cachedSoulConfig = parsed.config;
        return parsed.config;
      }
    }
  } catch {
    // Ignore cache errors
  }

  let raw: string;
  let isDefault = false;

  try {
    // 3. GitHub remote
    const response = await fetch(SOUL_GITHUB_URL);
    if (!response.ok) throw new Error(`HTTP ${response.status}`);
    raw = await response.text();
  } catch {
    // 4. Fallback to bundled
    raw = soulMd;
    isDefault = true;
  }

  const config = parseSoul(raw, isDefault);

  // Cache the result
  cachedSoulConfig = config;
  try {
    localStorage.setItem(SOUL_CACHE_KEY, JSON.stringify({ config, timestamp: Date.now() }));
  } catch {
    // Ignore storage errors
  }

  return config;
}

/**
 * Parse SOUL markdown into structured config
 */
export function parseSoul(raw: string, isDefault: boolean): SoulConfig {
  const identity = parseIdentity(raw);
  const personality = parsePersonality(raw);
  const voiceTone = parseVoiceTone(raw);
  const behaviors = parseBehaviors(raw);
  const safetyRules = parseSafetyRules(raw);
  const interactions = parseInteractions(raw);
  const memorySettings = parseMemorySettings(raw);
  const emergencyResponses = parseEmergencyResponses(raw);

  return {
    raw,
    identity,
    personality,
    voiceTone,
    behaviors,
    safetyRules,
    interactions,
    memorySettings,
    emergencyResponses,
    isDefault,
    loadedAt: Date.now(),
  };
}

function extractSection(raw: string, heading: string): string {
  const regex = new RegExp(`## ${heading}\\s*\\n([\\s\\S]*?)(?=\\n## |$)`, 'i');
  const match = raw.match(regex);
  return match?.[1]?.trim() ?? '';
}

function parseIdentity(raw: string): SoulIdentity {
  // Look for the title (first # heading)
  const titleMatch = raw.match(/^#\s+(.+)/m);
  const name = titleMatch?.[1]?.trim() ?? 'Unknown';

  // Look for description in the first few lines after title
  const lines = raw.split('\n');
  let description = '';
  for (let i = 1; i < Math.min(lines.length, 10); i++) {
    const line = lines[i].trim();
    if (line && !line.startsWith('#') && !line.startsWith('##')) {
      description = line;
      break;
    }
  }

  return { name, description: description || 'AI Assistant' };
}

function parsePersonality(raw: string): PersonalityTrait[] {
  const section = extractSection(raw, 'Personality');
  if (!section) return [];

  const traits: PersonalityTrait[] = [];
  const lines = section.split('\n').filter(l => l.trim().startsWith('- **'));

  for (const line of lines) {
    const match = line.match(/- \*\*(.+?)\*\*:\s*(.+)/);
    if (match) {
      traits.push({ trait: match[1].trim(), description: match[2].trim() });
    }
  }

  return traits;
}

function parseVoiceTone(raw: string): VoiceToneGuideline[] {
  const section = extractSection(raw, 'Voice & Tone');
  if (!section) return [];

  const guidelines: VoiceToneGuideline[] = [];
  const lines = section.split('\n').filter(l => l.trim().startsWith('- '));

  for (const line of lines) {
    const guideline = line.replace(/^-\s*/, '').trim();
    if (guideline) {
      guidelines.push({ guideline });
    }
  }

  return guidelines;
}

function parseBehaviors(raw: string): BehaviorPattern[] {
  const section = extractSection(raw, 'Behaviors');
  if (!section) return [];

  const patterns: BehaviorPattern[] = [];
  const subsections = section.split(/(?=### )/);

  for (const subsection of subsections) {
    const titleMatch = subsection.match(/### (.+)/);
    if (!titleMatch) continue;

    const context = titleMatch[1].trim();
    const behaviors = subsection
      .split('\n')
      .filter(l => l.trim().startsWith('- '))
      .map(l => l.replace(/^-\s*/, '').trim())
      .filter(Boolean);

    if (behaviors.length > 0) {
      patterns.push({ context, behaviors });
    }
  }

  return patterns;
}

function parseSafetyRules(raw: string): SafetyRule[] {
  const section = extractSection(raw, 'Safety Rules');
  if (!section) return [];

  const rules: SafetyRule[] = [];
  const lines = section.split('\n').filter(l => /^\d+\./.test(l.trim()));

  for (let i = 0; i < lines.length; i++) {
    const line = lines[i];
    const rule = line.replace(/^\d+\.\s*/, '').trim();
    if (rule) {
      rules.push({
        id: `safety-${i + 1}`,
        rule,
        priority: 10 - i, // Earlier rules have higher priority
      });
    }
  }

  return rules;
}

function parseInteractions(raw: string): Interaction[] {
  const section = extractSection(raw, 'Games You Know');
  if (!section) return [];

  const interactions: Interaction[] = [];
  const lines = section.split('\n').filter(l => /^\d+\./.test(l.trim()));

  for (const line of lines) {
    const match = line.match(/^\d+\.\s*\*\*(.+?)\*\*:\s*(.+)/);
    if (match) {
      interactions.push({ name: match[1].trim(), description: match[2].trim() });
    }
  }

  return interactions;
}

function parseMemorySettings(raw: string): MemorySettings {
  const section = extractSection(raw, 'Memory');
  if (!section) return { remember: [] };

  const remember: string[] = [];
  const lines = section.split('\n').filter(l => l.trim().startsWith('- '));

  for (const line of lines) {
    const item = line.replace(/^-\s*/, '').trim();
    if (item) {
      remember.push(item);
    }
  }

  return { remember };
}

function parseEmergencyResponses(raw: string): EmergencyResponse[] {
  const section = extractSection(raw, 'Emergency Responses');
  if (!section) return [];

  const responses: EmergencyResponse[] = [];
  const lines = section.split('\n').filter(l => l.trim().startsWith('- **'));

  for (const line of lines) {
    const match = line.match(/- \*\*(.+?)\*\*:\s*(.+)/);
    if (match) {
      responses.push({ trigger: match[1].trim(), response: match[2].trim() });
    }
  }

  return responses;
}

/**
 * Clear SOUL cache (useful for testing or manual refresh)
 */
export function clearSoulCache(): void {
  cachedSoulConfig = null;
  try {
    localStorage.removeItem(SOUL_CACHE_KEY);
  } catch {
    // Ignore storage errors
  }
}
