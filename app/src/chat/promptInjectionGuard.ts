export type PromptInjectionVerdict = 'allow' | 'block' | 'review';

export interface PromptInjectionReason {
  code: string;
  message: string;
}

export interface PromptInjectionCheck {
  verdict: PromptInjectionVerdict;
  score: number;
  reasons: PromptInjectionReason[];
}

interface Rule {
  code: string;
  message: string;
  score: number;
  regex: RegExp;
}

const SPACE_RE = /\s+/g;
const BASE64_RE = /[A-Za-z0-9+/]{24,}={0,2}/;

const RULES: Rule[] = [
  {
    code: 'override.ignore_previous',
    message: 'Looks like an attempt to override existing instructions.',
    score: 0.44,
    regex:
      /(ignore|disregard|forget|bypass)\s+(all\s+)?(previous|prior|above|system)\s+(instructions|rules|constraints|prompts?)/i,
  },
  {
    code: 'override.role_hijack',
    message: 'Looks like a role or policy hijack attempt.',
    score: 0.3,
    regex: /(you\s+are\s+now|act\s+as|developer\s+mode|jailbreak|unrestricted\s+mode|dan)/i,
  },
  {
    code: 'exfiltrate.system_prompt',
    message: 'Looks like a request to reveal hidden prompts/instructions.',
    score: 0.42,
    regex:
      /(reveal|show|print|dump|leak|display)\s+((the|your)\s+)?(system|developer|hidden)\s+(prompt|instructions|rules|message)/i,
  },
  {
    code: 'exfiltrate.secrets',
    message: 'Looks like a request for sensitive credentials.',
    score: 0.42,
    regex:
      /(api\s*key|secret|token|password|private\s+key|credentials?|session\s+cookie|jwt|bearer)/i,
  },
];

function normalize(input: string): {
  lowered: string;
  collapsed: string;
  compact: string;
  hasInstructionOverride: boolean;
  hasExfiltrationIntent: boolean;
} {
  const lowered = input.toLowerCase();
  const mapped = Array.from(lowered)
    .map(ch => {
      switch (ch) {
        case '0':
          return 'o';
        case '1':
          return 'i';
        case '3':
          return 'e';
        case '4':
          return 'a';
        case '5':
          return 's';
        case '7':
          return 't';
        case '\u200b':
        case '\u200c':
        case '\u200d':
        case '\u2060':
        case '\ufeff':
          return ' ';
        default:
          return /[a-z0-9\s]/i.test(ch) ? ch : ' ';
      }
    })
    .join('');

  const collapsed = mapped.trim().replace(SPACE_RE, ' ');
  const compact = collapsed.replace(/\s/g, '');
  const hasInstructionOverride =
    collapsed.includes('ignore previous instructions') ||
    collapsed.includes('ignore all previous instructions') ||
    compact.includes('ignoreallpreviousinstructions') ||
    compact.includes('ignorepreviousinstructions');
  const hasExfiltrationIntent =
    collapsed.includes('system prompt') ||
    collapsed.includes('developer instructions') ||
    collapsed.includes('hidden prompt') ||
    collapsed.includes('reveal');

  return { lowered, collapsed, compact, hasInstructionOverride, hasExfiltrationIntent };
}

export function checkPromptInjection(input: string): PromptInjectionCheck {
  const normalized = normalize(input);
  const reasons: PromptInjectionReason[] = [];
  let score = 0;

  if (normalized.hasInstructionOverride) {
    score += 0.46;
    reasons.push({
      code: 'override.obfuscated_instruction',
      message: 'Detected obfuscated instruction-override phrase.',
    });
  }
  if (normalized.hasExfiltrationIntent) {
    score += 0.24;
    reasons.push({
      code: 'exfiltration.intent',
      message: 'Detected exfiltration-focused prompt intent.',
    });
  }
  if (BASE64_RE.test(normalized.lowered)) {
    score += 0.08;
    reasons.push({
      code: 'obfuscation.base64_like',
      message: 'Contains base64-like obfuscated content.',
    });
  }

  for (const rule of RULES) {
    if (
      rule.regex.test(normalized.lowered) ||
      rule.regex.test(normalized.collapsed) ||
      rule.regex.test(normalized.compact)
    ) {
      score += rule.score;
      reasons.push({ code: rule.code, message: rule.message });
    }
  }

  score = Math.min(1, score);
  const verdict: PromptInjectionVerdict =
    score >= 0.7 ? 'block' : score >= 0.45 ? 'review' : 'allow';
  return { verdict, score, reasons };
}

export function promptGuardMessage(check: PromptInjectionCheck): string {
  if (check.verdict === 'block') {
    return 'This message looks like a prompt-injection attempt and will likely be blocked by server-side security checks.';
  }
  if (check.verdict === 'review') {
    return 'This message may be unsafe and could be rejected by server-side security checks. Please rephrase.';
  }
  return '';
}
