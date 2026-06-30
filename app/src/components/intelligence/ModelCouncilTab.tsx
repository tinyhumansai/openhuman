/**
 * Model Council tab — configure a small council of agent-flavored model seats,
 * ask one question, then let a judge model synthesize the responses.
 *
 * The Rust core still owns orchestration through `openhuman.model_council_run`.
 * This surface gives each seat an agent profile, Rive presence, and council
 * settings, then resolves the roster to model ids for the existing RPC.
 */
import { useCallback, useEffect, useMemo, useState } from 'react';

import {
  getMascotPalette,
  hexToArgbInt,
  type MascotFace,
  RiveMascot,
} from '../../features/human/Mascot';
import { useT } from '../../lib/i18n/I18nContext';
import { BubbleMarkdown } from '../../pages/conversations/components/AgentMessageBubble';
import {
  listProviderModels,
  loadAISettings,
  loadLocalProviderSnapshot,
  type ModelInfo,
} from '../../services/api/aiSettingsApi';
import { type CouncilDefinition, councilRegistryApi } from '../../services/api/councilRegistryApi';
import {
  type CouncilMemberResult,
  modelCouncilApi,
  type ModelCouncilResult,
} from '../../services/api/modelCouncilApi';
import {
  type AgentProfilesStatus,
  loadAgentProfiles,
  selectAgentProfiles,
} from '../../store/agentProfileSlice';
import { useAppDispatch, useAppSelector } from '../../store/hooks';
import type { AgentProfile } from '../../types/agentProfile';
import Button from '../ui/Button';

/** Matches the server-side MAX_COUNCIL_MEMBERS cap. */
const MAX_MEMBERS = 5;
const MIN_MEMBERS = 1;
const MAX_DEBATE_ROUNDS = 4;
const MIN_DEBATE_ROUNDS = 2;
const DEFAULT_REASONING_MODEL = 'reasoning-v1';

type SeatMode = 'default' | 'profile' | 'custom';

interface CouncilSeat {
  id: number;
  mode: SeatMode;
  profileId: string;
  name: string;
  model: string;
  brief: string;
}

interface ResolvedSeat {
  label: string;
  model: string;
  brief: string;
}

interface LiveMemberThought {
  status: 'pending' | 'answered' | 'failed';
  member: CouncilMemberResult | null;
  turns: CouncilDebateTurn[];
}

interface CouncilDebateTurn {
  round: number;
  response: string | null;
  error: string | null;
}

interface DebateUsageEstimate {
  inputTokens: number;
  outputTokens: number;
  totalTokens: number;
}

interface ModelPickerState {
  title: string;
  value: string;
  onSelect: (model: string) => void;
}

// `value` is the stable hint identifier; `labelKey` is resolved via `useT()`
// at render (this is a module-level const with no hook scope of its own).
const MODEL_HINTS = [
  { value: 'default', labelKey: 'modelCouncil.hint.default' },
  { value: 'hint:chat', labelKey: 'modelCouncil.hint.chat' },
  { value: 'hint:reasoning', labelKey: 'modelCouncil.hint.reasoning' },
  { value: 'hint:code', labelKey: 'modelCouncil.hint.code' },
  { value: 'hint:summarize', labelKey: 'modelCouncil.hint.summarize' },
] as const;

interface ConnectedModelProvider {
  slug: string;
  label: string;
  models?: ModelInfo[];
}

function parseProviderModel(value: string): { provider: string; model: string } {
  const trimmed = value.trim();
  const colon = trimmed.indexOf(':');
  if (colon <= 0) {
    return { provider: '', model: trimmed };
  }
  return { provider: trimmed.slice(0, colon), model: trimmed.slice(colon + 1) };
}

async function loadConnectedModelProviders(): Promise<ConnectedModelProvider[]> {
  const [settings, localSnapshot] = await Promise.all([
    loadAISettings(),
    loadLocalProviderSnapshot().catch(() => null),
  ]);
  const providers: ConnectedModelProvider[] = [{ slug: 'openhuman', label: 'Managed' }];
  const seen = new Set(providers.map(provider => provider.slug));

  for (const provider of settings.cloudProviders) {
    const slug = provider.slug.trim();
    if (!slug || seen.has(slug)) continue;
    if (!provider.has_api_key && provider.auth_style !== 'none') continue;
    providers.push({ slug, label: provider.label || slug });
    seen.add(slug);
  }

  const localModels =
    localSnapshot?.installedModels
      .filter(model => model.chat_capable !== false)
      .map(model => ({ id: model.name, context_window: model.context_length ?? null })) ?? [];
  if (localModels.length > 0 && !seen.has('ollama')) {
    providers.push({ slug: 'ollama', label: 'Ollama', models: localModels });
  }

  return providers;
}

const Icon = ({
  name,
  size = 16,
}: {
  name: 'arrow-left' | 'plus' | 'settings' | 'trash';
  size?: number;
}) => {
  const common = {
    fill: 'none',
    stroke: 'currentColor',
    strokeLinecap: 'round' as const,
    strokeLinejoin: 'round' as const,
    strokeWidth: 2,
  };
  return (
    <svg width={size} height={size} viewBox="0 0 24 24" aria-hidden="true" {...common}>
      {name === 'arrow-left' && (
        <>
          <path d="M19 12H5" />
          <path d="m12 19-7-7 7-7" />
        </>
      )}
      {name === 'plus' && (
        <>
          <path d="M12 5v14" />
          <path d="M5 12h14" />
        </>
      )}
      {name === 'settings' && (
        <>
          <path d="M12 15.5A3.5 3.5 0 1 0 12 8a3.5 3.5 0 0 0 0 7.5Z" />
          <path d="M19.4 15a1.7 1.7 0 0 0 .34 1.88l.06.06a2 2 0 1 1-2.83 2.83l-.06-.06A1.7 1.7 0 0 0 15 19.4a1.7 1.7 0 0 0-1 1.55V21a2 2 0 1 1-4 0v-.09A1.7 1.7 0 0 0 9 19.4a1.7 1.7 0 0 0-1.88.34l-.06.06a2 2 0 1 1-2.83-2.83l.06-.06A1.7 1.7 0 0 0 4.6 15a1.7 1.7 0 0 0-1.55-1H3a2 2 0 1 1 0-4h.09A1.7 1.7 0 0 0 4.6 9a1.7 1.7 0 0 0-.34-1.88l-.06-.06a2 2 0 1 1 2.83-2.83l.06.06A1.7 1.7 0 0 0 9 4.6a1.7 1.7 0 0 0 1-1.55V3a2 2 0 1 1 4 0v.09A1.7 1.7 0 0 0 15 4.6a1.7 1.7 0 0 0 1.88-.34l.06-.06a2 2 0 1 1 2.83 2.83l-.06.06A1.7 1.7 0 0 0 19.4 9a1.7 1.7 0 0 0 1.55 1H21a2 2 0 1 1 0 4h-.09A1.7 1.7 0 0 0 19.4 15Z" />
        </>
      )}
      {name === 'trash' && (
        <>
          <path d="M3 6h18" />
          <path d="M8 6V4h8v2" />
          <path d="m6 6 1 16h10l1-16" />
          <path d="M10 11v6" />
          <path d="M14 11v6" />
        </>
      )}
    </svg>
  );
};

const ModelPickerDialog = ({
  picker,
  onClose,
}: {
  picker: ModelPickerState;
  onClose: () => void;
}) => {
  const { t } = useT();
  const initial = parseProviderModel(picker.value);
  const initialHint = MODEL_HINTS.some(hint => hint.value === picker.value);
  const [selectionMode, setSelectionMode] = useState<'hint' | 'custom'>(
    initial.provider && !initialHint ? 'custom' : 'hint'
  );
  const [providers, setProviders] = useState<ConnectedModelProvider[]>([]);
  const [providersLoading, setProvidersLoading] = useState(false);
  const [providersError, setProvidersError] = useState<string | null>(null);
  const [provider, setProvider] = useState(initial.provider);
  const [models, setModels] = useState<ModelInfo[]>([]);
  const [modelsLoading, setModelsLoading] = useState(false);
  const [modelsError, setModelsError] = useState<string | null>(null);
  const [model, setModel] = useState(initial.model);

  useEffect(() => {
    let active = true;
    setProvidersLoading(true);
    setProvidersError(null);
    loadConnectedModelProviders()
      .then(loaded => {
        if (!active) return;
        setProviders(loaded);
        setProvidersLoading(false);
        setProvider(current => {
          if (current && loaded.some(item => item.slug === current)) return current;
          return loaded[0]?.slug ?? '';
        });
      })
      .catch(err => {
        if (!active) return;
        setProvidersError(err instanceof Error ? err.message : String(err));
        setProvidersLoading(false);
      });
    return () => {
      active = false;
    };
  }, []);

  useEffect(() => {
    if (selectionMode !== 'custom' || !provider) {
      setModels([]);
      setModelsError(null);
      return;
    }

    const connectedProvider = providers.find(item => item.slug === provider);
    if (!connectedProvider) {
      setModels([]);
      setModelsError(null);
      return;
    }

    if (connectedProvider.models) {
      setModels(connectedProvider.models);
      setModelsError(null);
      setModelsLoading(false);
      setModel(current => current || connectedProvider.models?.[0]?.id || '');
      return;
    }

    let active = true;
    setModelsLoading(true);
    setModels([]);
    setModelsError(null);
    listProviderModels(provider)
      .then(loaded => {
        if (!active) return;
        setModels(loaded);
        setModelsLoading(false);
        setModel(current => {
          if (current && loaded.some(item => item.id === current)) return current;
          return loaded[0]?.id ?? '';
        });
      })
      .catch(err => {
        if (!active) return;
        setModelsError(err instanceof Error ? err.message : String(err));
        setModelsLoading(false);
      });
    return () => {
      active = false;
    };
  }, [provider, providers, selectionMode]);

  const saveProviderModel = () => {
    const trimmedModel = model.trim();
    if (!trimmedModel) return;
    const trimmedProvider = provider.trim();
    picker.onSelect(trimmedProvider ? `${trimmedProvider}:${trimmedModel}` : trimmedModel);
    onClose();
  };

  return (
    <div
      role="dialog"
      aria-modal="true"
      aria-labelledby="model-council-model-picker-title"
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/30 p-4">
      <div className="w-full max-w-md rounded-lg border border-line bg-surface p-4 shadow-xl dark:bg-surface-canvas">
        <div className="flex items-start justify-between gap-3">
          <div>
            <h3
              id="model-council-model-picker-title"
              className="text-sm font-semibold text-content">
              {picker.title}
            </h3>
            <p className="mt-1 text-xs text-content-muted">{t('modelCouncil.modelPickerHelp')}</p>
          </div>
          <Button variant="tertiary" size="xs" onClick={onClose}>
            {t('modelCouncil.closeModelPicker')}
          </Button>
        </div>

        <div className="mt-4 space-y-2">
          <p className="text-[11px] font-semibold uppercase text-content-muted">
            {t('modelCouncil.modelPickerHints')}
          </p>
          <div className="grid grid-cols-2 gap-2">
            {MODEL_HINTS.map(hint => (
              <button
                key={hint.value}
                type="button"
                onClick={() => {
                  setSelectionMode('hint');
                  picker.onSelect(hint.value);
                  onClose();
                }}
                className={`rounded-lg border px-3 py-2 text-left text-sm ${
                  picker.value === hint.value
                    ? 'border-primary-500 bg-primary-50 text-primary-700 dark:bg-primary-500/15 dark:text-primary-200'
                    : 'border-line text-content-secondary hover:bg-surface-muted dark:hover:bg-surface'
                }`}>
                {t(hint.labelKey)}
                <span className="block font-mono text-[11px] text-content-muted">{hint.value}</span>
              </button>
            ))}
          </div>
        </div>

        <div className="mt-4 space-y-3 rounded-lg border border-line bg-surface-muted p-3">
          <button
            type="button"
            onClick={() => setSelectionMode('custom')}
            aria-pressed={selectionMode === 'custom'}
            className={`w-full rounded-lg border px-3 py-2 text-left text-sm font-semibold ${
              selectionMode === 'custom'
                ? 'border-primary-500 bg-surface text-primary-700 dark:bg-surface-canvas dark:text-primary-200'
                : 'border-line bg-surface text-content-secondary hover:bg-surface-hover dark:bg-surface-canvas'
            }`}>
            {t('modelCouncil.modelPickerProviderModel')}
            <span className="block text-[11px] font-normal text-content-muted">
              {t('modelCouncil.mode.custom')}
            </span>
          </button>

          <div className="grid gap-2 sm:grid-cols-[minmax(0,1fr)_minmax(0,1fr)]">
            <select
              value={provider}
              onChange={e => setProvider(e.target.value)}
              aria-label={t('modelCouncil.modelProviderLabel')}
              disabled={selectionMode !== 'custom' || providersLoading || providers.length === 0}
              className="rounded-lg border border-line bg-surface px-3 py-2 text-sm text-content focus:outline-none focus:ring-2 focus:ring-primary-400 disabled:cursor-not-allowed disabled:opacity-50 dark:bg-surface-canvas">
              {providers.map(item => (
                <option key={item.slug} value={item.slug}>
                  {`${item.slug === 'openhuman' ? t('settings.ai.routing.managed') : item.label} (${item.slug})`}
                </option>
              ))}
            </select>
            <select
              value={model}
              onChange={e => setModel(e.target.value)}
              aria-label={t('modelCouncil.modelIdLabel')}
              disabled={selectionMode !== 'custom' || modelsLoading || models.length === 0}
              className="rounded-lg border border-line bg-surface px-3 py-2 font-mono text-sm text-content focus:outline-none focus:ring-2 focus:ring-primary-400 disabled:cursor-not-allowed disabled:opacity-50 dark:bg-surface-canvas">
              {models.map(item => (
                <option key={item.id} value={item.id}>
                  {item.id}
                </option>
              ))}
            </select>
          </div>
          {(providersLoading || modelsLoading) && (
            <p className="text-[11px] text-content-muted">{t('skills.resource.preview.loading')}</p>
          )}
          {(providersError || modelsError) && (
            <p role="alert" className="text-[11px] text-coral-700 dark:text-coral-300">
              {providersError || modelsError}
            </p>
          )}
          <Button
            variant="primary"
            onClick={saveProviderModel}
            disabled={selectionMode !== 'custom' || !provider.trim() || !model.trim()}
            className="w-full">
            {t('modelCouncil.useProviderModel')}
          </Button>
        </div>
      </div>
    </div>
  );
};

const DEFAULT_MODEL = DEFAULT_REASONING_MODEL;
const DEFAULT_JUDGE_MODEL = DEFAULT_REASONING_MODEL;
const SHARED_REASONING_FILE = 'shared_reasoning.md';
const DEFAULT_SHARED_REASONING = [
  '# Shared reasoning',
  '- Claims the council agrees on:',
  '- Open disagreements:',
  '- Evidence or constraints to preserve:',
  '- Judge synthesis notes:',
].join('\n');
const DEFAULT_SEATS: CouncilSeat[] = [
  {
    id: 0,
    mode: 'default',
    profileId: '',
    name: 'Analyst',
    model: DEFAULT_MODEL,
    brief: 'Evidence, assumptions, and risk.',
  },
  {
    id: 1,
    mode: 'default',
    profileId: '',
    name: 'Builder',
    model: DEFAULT_MODEL,
    brief: 'Practical implementation path.',
  },
  {
    id: 2,
    mode: 'default',
    profileId: '',
    name: 'Skeptic',
    model: DEFAULT_MODEL,
    brief: 'Failure modes and missing context.',
  },
];

const SEAT_COLORS = ['yellow', 'burgundy', 'navy', 'black', 'yellow'] as const;
const SEAT_FACES: MascotFace[] = ['thinking', 'writing', 'reading', 'curious', 'proud'];
const ACTIVE_SEAT_FACES: MascotFace[] = ['thinking', 'writing', 'thinking', 'reading', 'curious'];

const nextSeatId = (seats: CouncilSeat[]): number =>
  seats.reduce((max, seat) => Math.max(max, seat.id), -1) + 1;

function profileLabel(profile: AgentProfile): string {
  return profile.modelOverride ? `${profile.name} · ${profile.modelOverride}` : profile.name;
}

function profileModel(profile: AgentProfile | undefined): string {
  return profile?.modelOverride?.trim() || profile?.agentId?.trim() || profile?.id?.trim() || '';
}

function resolveSeat(seat: CouncilSeat, profiles: AgentProfile[], index: number): ResolvedSeat {
  const profile = profiles.find(p => p.id === seat.profileId);
  const fallbackName =
    seat.mode === 'profile' && profile ? profile.name : seat.name.trim() || `Juror ${index + 1}`;
  const fallbackModel = seat.mode === 'profile' ? profileModel(profile) : DEFAULT_MODEL;

  return {
    label: fallbackName,
    model: seat.model.trim() || fallbackModel,
    brief: seat.brief.trim(),
  };
}

function mascotColors(index: number) {
  const palette = getMascotPalette(SEAT_COLORS[index % SEAT_COLORS.length]);
  return {
    primaryColor: hexToArgbInt(palette.bodyFill),
    secondaryColor: hexToArgbInt(palette.neckShadowColor),
  };
}

function deliberationThought(
  seat: ResolvedSeat,
  index: number,
  t: (key: string) => string
): string {
  const brief = seat.brief.trim();
  if (brief) {
    return t('modelCouncil.thinkingWithBrief').replace('{brief}', brief);
  }

  const keys = [
    'modelCouncil.thought.evidence',
    'modelCouncil.thought.plan',
    'modelCouncil.thought.risk',
    'modelCouncil.thought.tradeoffs',
    'modelCouncil.thought.synthesis',
  ];
  return t(keys[index % keys.length]);
}

function buildCouncilQuestion(
  question: string,
  sharedReasoning: string,
  seats: ResolvedSeat[],
  judgeName: string
): string {
  const trimmedQuestion = question.trim();
  const trimmedSharedReasoning = sharedReasoning.trim();
  const roster = seats
    .map((seat, index) => {
      const brief = seat.brief ? ` — ${seat.brief}` : '';
      return `${index + 1}. ${seat.label} (${seat.model})${brief}`;
    })
    .join('\n');
  const commonPrefix = [
    `Council workspace: ${SHARED_REASONING_FILE}`,
    'Use this shared reasoning file as the common deliberation scratchpad.',
    '',
    'Council roster:',
    roster,
    '',
    `Judge agent: ${judgeName}`,
  ];

  if (!trimmedSharedReasoning) {
    return [...commonPrefix, '', 'User question:', trimmedQuestion].join('\n');
  }

  return [
    ...commonPrefix,
    '',
    `${SHARED_REASONING_FILE}:`,
    trimmedSharedReasoning,
    '',
    'User question:',
    trimmedQuestion,
  ].join('\n');
}

function buildDebateTurnQuestion(
  baseQuestion: string,
  seat: ResolvedSeat,
  round: number,
  totalRounds: number,
  transcript: CouncilDebateTurn[][],
  t: (key: string) => string
): string {
  const previousTurns = transcript
    .map((turns, seatIndex) => {
      if (turns.length === 0) return '';
      const body = turns
        .map(turn => {
          const text = turn.response || `[${turn.error || 'no response'}]`;
          return `Round ${turn.round}: ${text}`;
        })
        .join('\n');
      return `Juror ${seatIndex + 1} previous turns:\n${body}`;
    })
    .filter(Boolean)
    .join('\n\n');

  const phase =
    round === totalRounds
      ? t('modelCouncil.debateFinalInstruction')
      : t('modelCouncil.debateRoundInstruction');

  return [
    baseQuestion,
    '',
    `Debate round ${round} of ${totalRounds}.`,
    `You are ${seat.label}. Perspective: ${seat.brief || 'independent council juror'}.`,
    phase,
    previousTurns ? ['', 'Debate so far:', previousTurns].join('\n') : '',
    '',
    'Write this turn as a concise council thought plus your current conclusion.',
  ]
    .filter(Boolean)
    .join('\n');
}

function appendScratchpadRound(
  scratchpad: string,
  round: number,
  seats: ResolvedSeat[],
  roundResults: Array<{ index: number; turn: CouncilDebateTurn }>,
  t: (key: string) => string
): string {
  const existing = scratchpad.trim() || '# Shared reasoning';
  const lines = [
    '',
    '',
    `## ${t('modelCouncil.scratchpadRoundHeading').replace('{round}', String(round))}`,
  ];
  for (const { index, turn } of [...roundResults].sort((a, b) => a.index - b.index)) {
    const seat = seats[index];
    lines.push('', `### ${seat?.label || `Juror ${index + 1}`}`);
    if (turn.response) {
      lines.push(turn.response.trim());
    } else {
      lines.push(`_${t('modelCouncil.scratchpadNoResponse')}: ${turn.error || 'unknown'}_`);
    }
  }
  return `${existing}${lines.join('\n')}`;
}

function estimateTokens(text: string): number {
  return Math.max(1, Math.ceil(text.length / 4));
}

function formatTokenCount(value: number): string {
  return new Intl.NumberFormat(undefined, { maximumFractionDigits: 0 }).format(value);
}

function buildMemberSynthesisInput(
  seat: ResolvedSeat,
  model: string,
  turns: CouncilDebateTurn[]
): CouncilMemberResult {
  const answeredTurns = turns.filter(turn => turn.response);
  if (answeredTurns.length === 0) {
    return {
      model,
      response: null,
      error: turns.find(turn => turn.error)?.error || 'no debate turns completed',
    };
  }

  return {
    model,
    response: [
      `${seat.label} debate record:`,
      ...turns.map(turn => {
        const text = turn.response || `[failed: ${turn.error || 'unknown'}]`;
        return `Round ${turn.round}: ${text}`;
      }),
    ].join('\n\n'),
    error: null,
  };
}

function councilSeatsFromDefinition(council: CouncilDefinition): CouncilSeat[] {
  return council.seats.map(seat => ({
    id: seat.id,
    mode: seat.mode,
    profileId: seat.profile_id,
    name: seat.name,
    model: seat.model,
    brief: seat.brief,
  }));
}

function createDraftCouncil(): CouncilDefinition {
  const now = Date.now();
  return {
    id: '',
    name: 'New council',
    description: '',
    jury_count: 3,
    debate_rounds: 3,
    seats: DEFAULT_SEATS.map(seat => ({
      id: seat.id,
      mode: seat.mode,
      profile_id: seat.profileId,
      name: seat.name,
      model: seat.model,
      brief: seat.brief,
    })),
    judge: { mode: 'default', profile_id: '', name: 'Chief Judge', model: DEFAULT_JUDGE_MODEL },
    shared_reasoning: DEFAULT_SHARED_REASONING,
    created_at_ms: now,
    updated_at_ms: now,
  };
}

const ModelCouncilTab = () => {
  const { t } = useT();
  const dispatch = useAppDispatch();
  const profiles = useAppSelector(selectAgentProfiles);
  const profileStatus = useAppSelector(state => state.agentProfiles.status as AgentProfilesStatus);

  const [question, setQuestion] = useState('');
  const [view, setView] = useState<'list' | 'run' | 'edit'>('list');
  const [councils, setCouncils] = useState<CouncilDefinition[]>([]);
  const [selectedCouncil, setSelectedCouncil] = useState<CouncilDefinition | null>(null);
  const [councilName, setCouncilName] = useState('Default council');
  const [councilDescription, setCouncilDescription] = useState('');
  const [registryLoading, setRegistryLoading] = useState(true);
  const [registrySaving, setRegistrySaving] = useState(false);
  const [registryError, setRegistryError] = useState<string | null>(null);
  const [sharedReasoning, setSharedReasoning] = useState(DEFAULT_SHARED_REASONING);
  const [liveScratchpad, setLiveScratchpad] = useState<string | null>(null);
  const [juryCount, setJuryCount] = useState(3);
  const [debateRounds, setDebateRounds] = useState(3);
  const [seats, setSeats] = useState<CouncilSeat[]>(DEFAULT_SEATS);
  const [judgeMode, setJudgeMode] = useState<SeatMode>('default');
  const [judgeProfileId, setJudgeProfileId] = useState('');
  const [judgeName, setJudgeName] = useState('Chief Judge');
  const [judgeModel, setJudgeModel] = useState(DEFAULT_JUDGE_MODEL);
  const [running, setRunning] = useState(false);
  const [liveMembers, setLiveMembers] = useState<LiveMemberThought[]>([]);
  const [judgeSynthesizing, setJudgeSynthesizing] = useState(false);
  const [usageEstimate, setUsageEstimate] = useState<DebateUsageEstimate | null>(null);
  const [modelPicker, setModelPicker] = useState<ModelPickerState | null>(null);
  const [result, setResult] = useState<ModelCouncilResult | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (profileStatus === 'idle' && profiles.length === 0) {
      void dispatch(loadAgentProfiles());
    }
  }, [dispatch, profileStatus, profiles.length]);

  const applyCouncilDefinition = useCallback((council: CouncilDefinition) => {
    setSelectedCouncil(council);
    setCouncilName(council.name || 'Untitled council');
    setCouncilDescription(council.description || '');
    setJuryCount(Math.min(MAX_MEMBERS, Math.max(MIN_MEMBERS, council.jury_count || 3)));
    setDebateRounds(
      Math.min(MAX_DEBATE_ROUNDS, Math.max(MIN_DEBATE_ROUNDS, council.debate_rounds || 3))
    );
    setSeats(councilSeatsFromDefinition(council).slice(0, council.jury_count || 3));
    setJudgeMode(council.judge.mode);
    setJudgeProfileId(council.judge.profile_id || '');
    setJudgeName(council.judge.name || 'Chief Judge');
    setJudgeModel(council.judge.model ?? DEFAULT_JUDGE_MODEL);
    setSharedReasoning(council.shared_reasoning || DEFAULT_SHARED_REASONING);
    setQuestion('');
    setLiveMembers([]);
    setLiveScratchpad(null);
    setJudgeSynthesizing(false);
    setUsageEstimate(null);
    setResult(null);
    setError(null);
  }, []);

  const loadCouncils = useCallback(async () => {
    setRegistryLoading(true);
    setRegistryError(null);
    try {
      const loaded = await councilRegistryApi.list();
      setCouncils(loaded);
    } catch (err) {
      setRegistryError(err instanceof Error ? err.message : String(err));
    } finally {
      setRegistryLoading(false);
    }
  }, []);

  useEffect(() => {
    void loadCouncils();
  }, [loadCouncils]);

  useEffect(() => {
    setSeats(prev => {
      if (prev.length === juryCount) return prev;
      if (prev.length > juryCount) return prev.slice(0, juryCount);

      const next = [...prev];
      while (next.length < juryCount) {
        const index = next.length;
        next.push({
          id: nextSeatId(next),
          mode: 'default',
          profileId: '',
          name: `${t('modelCouncil.jurorFallback')} ${index + 1}`,
          model: DEFAULT_MODEL,
          brief: '',
        });
      }
      return next;
    });
  }, [juryCount, t]);

  const judgeProfile = useMemo(
    () => profiles.find(profile => profile.id === judgeProfileId),
    [profiles, judgeProfileId]
  );

  const resolvedSeats = useMemo(
    () => seats.map((seat, index) => resolveSeat(seat, profiles, index)),
    [profiles, seats]
  );

  const resolvedJudgeModel =
    judgeModel.trim() ||
    (judgeMode === 'profile' ? profileModel(judgeProfile) : '') ||
    DEFAULT_JUDGE_MODEL;
  const resolvedJudgeName =
    judgeMode === 'profile' && judgeProfile ? judgeProfile.name : judgeName.trim() || 'Chief Judge';

  const canRun =
    !running &&
    question.trim().length > 0 &&
    resolvedSeats.some(seat => seat.model.trim().length > 0) &&
    resolvedJudgeModel.trim().length > 0;

  const updateSeat = useCallback((id: number, patch: Partial<CouncilSeat>) => {
    setSeats(prev => prev.map(seat => (seat.id === id ? { ...seat, ...patch } : seat)));
  }, []);

  const buildCouncilDefinition = useCallback(
    (base: CouncilDefinition | null): CouncilDefinition => {
      const now = Date.now();
      return {
        id: base?.id || '',
        name: councilName.trim() || 'Untitled council',
        description: councilDescription.trim(),
        jury_count: juryCount,
        debate_rounds: debateRounds,
        seats: seats
          .slice(0, juryCount)
          .map(seat => ({
            id: seat.id,
            mode: seat.mode,
            profile_id: seat.profileId,
            name: seat.name,
            model: seat.model,
            brief: seat.brief,
          })),
        judge: { mode: judgeMode, profile_id: judgeProfileId, name: judgeName, model: judgeModel },
        shared_reasoning: sharedReasoning,
        created_at_ms: base?.created_at_ms || now,
        updated_at_ms: now,
      };
    },
    [
      councilDescription,
      councilName,
      debateRounds,
      judgeMode,
      judgeModel,
      judgeName,
      judgeProfileId,
      juryCount,
      seats,
      sharedReasoning,
    ]
  );

  const saveCouncil = useCallback(async () => {
    setRegistrySaving(true);
    setRegistryError(null);
    try {
      const saved = await councilRegistryApi.upsert(buildCouncilDefinition(selectedCouncil));
      setCouncils(prev => {
        const without = prev.filter(council => council.id !== saved.id);
        return [saved, ...without].sort((a, b) => a.name.localeCompare(b.name));
      });
      applyCouncilDefinition(saved);
      setView('run');
    } catch (err) {
      setRegistryError(err instanceof Error ? err.message : String(err));
    } finally {
      setRegistrySaving(false);
    }
  }, [applyCouncilDefinition, buildCouncilDefinition, selectedCouncil]);

  const handleSelectCouncil = useCallback(
    (council: CouncilDefinition) => {
      applyCouncilDefinition(council);
      setView('run');
    },
    [applyCouncilDefinition]
  );

  const handleCreateCouncil = useCallback(() => {
    applyCouncilDefinition(createDraftCouncil());
    setView('edit');
  }, [applyCouncilDefinition]);

  const selectedCouncilId = selectedCouncil?.id;
  const handleDeleteCouncil = useCallback(
    async (council: CouncilDefinition) => {
      setRegistryError(null);
      try {
        await councilRegistryApi.delete(council.id);
        setCouncils(prev => prev.filter(item => item.id !== council.id));
        if (selectedCouncilId === council.id) {
          setSelectedCouncil(null);
          setView('list');
        }
      } catch (err) {
        setRegistryError(err instanceof Error ? err.message : String(err));
      }
    },
    [selectedCouncilId]
  );

  const setSeatMode = useCallback(
    (seat: CouncilSeat, mode: SeatMode) => {
      updateSeat(seat.id, {
        mode,
        profileId: mode === 'profile' ? seat.profileId || profiles[0]?.id || '' : '',
        name: mode === 'custom' ? seat.name : seat.name || '',
        model: mode === 'profile' ? '' : seat.model || DEFAULT_MODEL,
      });
    },
    [profiles, updateSeat]
  );

  const handleRun = useCallback(async () => {
    if (running) return;
    const memberModels = resolvedSeats.map(seat => seat.model.trim()).filter(Boolean);
    const chairModel = resolvedJudgeModel.trim();
    if (question.trim().length === 0 || memberModels.length === 0 || chairModel.length === 0) {
      return;
    }
    setRunning(true);
    setJudgeSynthesizing(false);
    setLiveMembers(memberModels.map(() => ({ status: 'pending', member: null, turns: [] })));
    setLiveScratchpad(sharedReasoning);
    setUsageEstimate(null);
    setError(null);
    setResult(null);
    try {
      const transcript: CouncilDebateTurn[][] = memberModels.map(() => []);
      let currentScratchpad = sharedReasoning;
      let estimatedInputTokens = 0;
      let estimatedOutputTokens = 0;

      for (let round = 1; round <= debateRounds; round += 1) {
        setLiveMembers(prev => prev.map(entry => ({ ...entry, status: 'pending' })));

        const roundResults = await Promise.all(
          memberModels.map(async (model, index) => {
            const councilQuestion = buildCouncilQuestion(
              question,
              currentScratchpad,
              resolvedSeats,
              resolvedJudgeName
            );
            const turnQuestion = buildDebateTurnQuestion(
              councilQuestion,
              resolvedSeats[index],
              round,
              debateRounds,
              transcript,
              t
            );
            estimatedInputTokens += estimateTokens(turnQuestion);
            try {
              const member = await modelCouncilApi.answerMember({ question: turnQuestion, model });
              const turn: CouncilDebateTurn = {
                round,
                response: member.response,
                error: member.error,
              };
              estimatedOutputTokens += estimateTokens(member.response || member.error || '');
              setLiveMembers(prev =>
                prev.map((entry, entryIndex) =>
                  entryIndex === index
                    ? {
                        status: member.error ? 'failed' : 'answered',
                        member,
                        turns: [...entry.turns, turn],
                      }
                    : entry
                )
              );
              return { index, turn };
            } catch (memberError) {
              const errorText =
                memberError instanceof Error ? memberError.message : String(memberError);
              const failedMember: CouncilMemberResult = { model, response: null, error: errorText };
              const turn: CouncilDebateTurn = { round, response: null, error: errorText };
              estimatedOutputTokens += estimateTokens(errorText);
              setLiveMembers(prev =>
                prev.map((entry, entryIndex) =>
                  entryIndex === index
                    ? { status: 'failed', member: failedMember, turns: [...entry.turns, turn] }
                    : entry
                )
              );
              return { index, turn };
            }
          })
        );

        for (const { index, turn } of roundResults) {
          transcript[index].push(turn);
        }
        currentScratchpad = appendScratchpadRound(
          currentScratchpad,
          round,
          resolvedSeats,
          roundResults,
          t
        );
        setLiveScratchpad(currentScratchpad);
        setSharedReasoning(currentScratchpad);
      }

      const councilQuestion = buildCouncilQuestion(
        question,
        currentScratchpad,
        resolvedSeats,
        resolvedJudgeName
      );
      const memberResults = memberModels.map((model, index) =>
        buildMemberSynthesisInput(resolvedSeats[index], model, transcript[index])
      );
      const synthesisInputTokens = estimateTokens(
        `${councilQuestion}\n${JSON.stringify(memberResults)}`
      );
      estimatedInputTokens += synthesisInputTokens;
      setJudgeSynthesizing(true);
      const res = await modelCouncilApi.synthesizeCouncil({
        question: councilQuestion,
        members: memberResults,
        chair_model: chairModel,
      });
      estimatedOutputTokens += estimateTokens(res.synthesis);
      const totalTokens = estimatedInputTokens + estimatedOutputTokens;
      setUsageEstimate({
        inputTokens: estimatedInputTokens,
        outputTokens: estimatedOutputTokens,
        totalTokens,
      });
      setResult(res);
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setJudgeSynthesizing(false);
      setRunning(false);
      setLiveScratchpad(null);
    }
  }, [
    debateRounds,
    resolvedJudgeModel,
    resolvedJudgeName,
    resolvedSeats,
    question,
    running,
    sharedReasoning,
    t,
  ]);

  if (view === 'list') {
    return (
      <div className="space-y-5">
        <div className="flex flex-wrap items-start justify-between gap-3">
          <div>
            <h2 className="text-lg font-semibold text-content">{t('modelCouncil.listTitle')}</h2>
            <p className="mt-1 max-w-3xl text-sm text-content-secondary">
              {t('modelCouncil.listIntro')}
            </p>
          </div>
          <Button
            variant="primary"
            onClick={handleCreateCouncil}
            leadingIcon={<Icon name="plus" size={16} />}>
            {t('modelCouncil.addCouncil')}
          </Button>
        </div>

        {registryError && (
          <p role="alert" className="text-xs text-coral-700 dark:text-coral-300">
            {t('modelCouncil.registryErrorPrefix')} {registryError}
          </p>
        )}

        {registryLoading ? (
          <div className="rounded-lg border border-line bg-surface p-4 text-sm text-content-muted shadow-sm">
            {t('modelCouncil.loadingCouncils')}
          </div>
        ) : councils.length === 0 ? (
          <div className="rounded-lg border border-line bg-surface p-4 text-sm text-content-muted shadow-sm">
            {t('modelCouncil.noCouncils')}
          </div>
        ) : (
          <div className="grid gap-3 md:grid-cols-2 xl:grid-cols-3">
            {councils.map(council => (
              <article
                key={council.id}
                className="rounded-lg border border-line bg-surface p-4 shadow-sm transition hover:border-primary-300 hover:shadow-md dark:hover:border-primary-500/50">
                <div className="flex items-start justify-between gap-3">
                  <div className="min-w-0">
                    <h3 className="truncate text-sm font-semibold text-content">{council.name}</h3>
                    <p className="mt-1 line-clamp-2 text-xs text-content-muted">
                      {council.description || t('modelCouncil.noCouncilDescription')}
                    </p>
                  </div>
                  <div className="flex shrink-0 items-center gap-1">
                    <Button
                      iconOnly
                      variant="tertiary"
                      size="sm"
                      onClick={() => {
                        applyCouncilDefinition(council);
                        setView('edit');
                      }}
                      aria-label={t('modelCouncil.editCouncilAria').replace(
                        '{name}',
                        council.name
                      )}>
                      <Icon name="settings" size={16} />
                    </Button>
                    <Button
                      iconOnly
                      variant="tertiary"
                      tone="danger"
                      size="sm"
                      onClick={() => void handleDeleteCouncil(council)}
                      aria-label={t('modelCouncil.deleteCouncilAria').replace(
                        '{name}',
                        council.name
                      )}>
                      <Icon name="trash" size={16} />
                    </Button>
                  </div>
                </div>
                <dl className="mt-4 grid grid-cols-2 gap-2 text-xs">
                  <div className="rounded-md bg-surface-muted px-2 py-1.5">
                    <dt className="text-content-muted">{t('modelCouncil.juryCountLabel')}</dt>
                    <dd className="font-mono font-semibold text-content">{council.jury_count}</dd>
                  </div>
                  <div className="rounded-md bg-surface-muted px-2 py-1.5">
                    <dt className="text-content-muted">{t('modelCouncil.debateRoundsLabel')}</dt>
                    <dd className="font-mono font-semibold text-content">
                      {council.debate_rounds}
                    </dd>
                  </div>
                </dl>
                <button
                  type="button"
                  onClick={() => handleSelectCouncil(council)}
                  className="mt-4 w-full rounded-lg border border-primary-200 bg-primary-50 px-3 py-2 text-sm font-semibold text-primary-700 hover:bg-primary-100 dark:border-primary-500/30 dark:bg-primary-500/10 dark:text-primary-200 dark:hover:bg-primary-500/20">
                  {t('modelCouncil.openCouncil')}
                </button>
              </article>
            ))}
          </div>
        )}
      </div>
    );
  }

  return (
    <div className="space-y-5">
      <div className="flex flex-wrap items-center justify-between gap-3">
        <div className="flex min-w-0 items-center gap-2">
          <Button
            iconOnly
            variant="tertiary"
            size="sm"
            onClick={() => setView('list')}
            aria-label={t('modelCouncil.backToCouncils')}>
            <Icon name="arrow-left" size={18} />
          </Button>
          <div className="min-w-0">
            <h2 className="truncate text-lg font-semibold text-content">
              {view === 'edit' ? t('modelCouncil.editCouncil') : councilName}
            </h2>
            {selectedCouncil && view === 'run' && (
              <p className="truncate text-xs text-content-muted">
                {councilDescription || t('modelCouncil.noCouncilDescription')}
              </p>
            )}
          </div>
        </div>
        <div className="flex items-center gap-2">
          {view === 'edit' ? (
            <>
              <Button
                variant="secondary"
                onClick={() => {
                  if (selectedCouncil) applyCouncilDefinition(selectedCouncil);
                  setView(selectedCouncil ? 'run' : 'list');
                }}>
                {t('modelCouncil.cancelEdit')}
              </Button>
              <Button
                variant="primary"
                onClick={() => void saveCouncil()}
                disabled={registrySaving}>
                {registrySaving ? t('modelCouncil.savingCouncil') : t('modelCouncil.saveCouncil')}
              </Button>
            </>
          ) : (
            <Button
              variant="secondary"
              onClick={() => setView('edit')}
              aria-label={t('modelCouncil.editCurrentCouncil')}
              leadingIcon={<Icon name="settings" size={16} />}>
              {t('modelCouncil.editCouncil')}
            </Button>
          )}
        </div>
      </div>

      {registryError && (
        <p role="alert" className="text-xs text-coral-700 dark:text-coral-300">
          {t('modelCouncil.registryErrorPrefix')} {registryError}
        </p>
      )}

      {view === 'edit' && (
        <section className="grid gap-3 rounded-lg border border-line bg-surface p-3 shadow-sm md:grid-cols-2">
          <div className="space-y-1.5">
            <label
              htmlFor="model-council-name"
              className="text-xs font-medium text-content-secondary">
              {t('modelCouncil.councilNameLabel')}
            </label>
            <input
              id="model-council-name"
              value={councilName}
              onChange={e => setCouncilName(e.target.value)}
              className="w-full rounded-lg border border-line bg-surface px-3 py-2 text-sm text-content shadow-sm focus:outline-none focus:ring-2 focus:ring-primary-400 dark:bg-surface-canvas"
            />
          </div>
          <div className="space-y-1.5">
            <label
              htmlFor="model-council-description"
              className="text-xs font-medium text-content-secondary">
              {t('modelCouncil.councilDescriptionLabel')}
            </label>
            <textarea
              id="model-council-description"
              value={councilDescription}
              onChange={e => setCouncilDescription(e.target.value)}
              placeholder={t('modelCouncil.councilDescriptionPlaceholder')}
              rows={3}
              className="w-full rounded-lg border border-line bg-surface px-3 py-2 text-sm text-content shadow-sm resize-y focus:outline-none focus:ring-2 focus:ring-primary-400 dark:bg-surface-canvas"
            />
          </div>
        </section>
      )}

      {view === 'run' && (
        <section className="space-y-3">
          <label
            htmlFor="model-council-question"
            className="text-xs font-medium text-content-secondary">
            {t('modelCouncil.questionLabel')}
          </label>
          <textarea
            id="model-council-question"
            value={question}
            onChange={e => setQuestion(e.target.value)}
            rows={4}
            placeholder={t('modelCouncil.questionPlaceholder')}
            aria-label={t('modelCouncil.questionLabel')}
            className="w-full rounded-lg border border-line bg-surface px-3 py-2 text-sm text-content shadow-sm resize-y focus:outline-none focus:ring-2 focus:ring-primary-400"
          />
        </section>
      )}

      {view === 'edit' && (
        <aside className="space-y-3 rounded-lg border border-line bg-surface p-3 shadow-sm">
          <div>
            <p className="text-xs font-semibold uppercase tracking-wide text-content-muted">
              {t('modelCouncil.settingsTitle')}
            </p>
            <p className="mt-1 text-xs text-content-muted">
              {t('modelCouncil.settingsSummary')
                .replace('{count}', String(juryCount))
                .replace('{judge}', resolvedJudgeName)}
            </p>
          </div>

          <div className="space-y-2">
            <div className="flex items-center justify-between gap-3">
              <label
                htmlFor="model-council-jury-count"
                className="text-xs font-medium text-content-secondary">
                {t('modelCouncil.juryCountLabel')}
              </label>
              <output className="rounded-md bg-surface-subtle px-2 py-0.5 text-xs font-semibold text-content-secondary">
                {juryCount}
              </output>
            </div>
            <input
              id="model-council-jury-count"
              type="range"
              min={MIN_MEMBERS}
              max={MAX_MEMBERS}
              value={juryCount}
              aria-label={t('modelCouncil.juryCountLabel')}
              onChange={e => setJuryCount(Number(e.target.value))}
              className="w-full accent-primary-500"
            />
            <div className="grid grid-cols-5 gap-1">
              {Array.from({ length: MAX_MEMBERS }, (_, index) => index + 1).map(count => (
                <button
                  key={count}
                  type="button"
                  onClick={() => setJuryCount(count)}
                  aria-pressed={juryCount === count}
                  className={`rounded-md border px-2 py-1 text-xs font-medium ${
                    juryCount === count
                      ? 'border-primary-500 bg-primary-50 text-primary-700 dark:bg-primary-500/15 dark:text-primary-200'
                      : 'border-line text-content-muted hover:bg-surface-hover'
                  }`}>
                  {count}
                </button>
              ))}
            </div>
          </div>

          <div className="space-y-2">
            <div className="flex items-center justify-between gap-3">
              <label
                htmlFor="model-council-debate-rounds"
                className="text-xs font-medium text-content-secondary">
                {t('modelCouncil.debateRoundsLabel')}
              </label>
              <output className="rounded-md bg-surface-subtle px-2 py-0.5 text-xs font-semibold text-content-secondary">
                {debateRounds}
              </output>
            </div>
            <input
              id="model-council-debate-rounds"
              type="range"
              min={MIN_DEBATE_ROUNDS}
              max={MAX_DEBATE_ROUNDS}
              value={debateRounds}
              aria-label={t('modelCouncil.debateRoundsLabel')}
              onChange={e => setDebateRounds(Number(e.target.value))}
              className="w-full accent-primary-500"
            />
            <div className="grid grid-cols-3 gap-1">
              {Array.from(
                { length: MAX_DEBATE_ROUNDS - MIN_DEBATE_ROUNDS + 1 },
                (_, index) => index + MIN_DEBATE_ROUNDS
              ).map(rounds => (
                <button
                  key={rounds}
                  type="button"
                  onClick={() => setDebateRounds(rounds)}
                  aria-pressed={debateRounds === rounds}
                  className={`rounded-md border px-2 py-1 text-xs font-medium ${
                    debateRounds === rounds
                      ? 'border-primary-500 bg-primary-50 text-primary-700 dark:bg-primary-500/15 dark:text-primary-200'
                      : 'border-line text-content-muted hover:bg-surface-hover'
                  }`}>
                  {rounds}
                </button>
              ))}
            </div>
            <p className="text-[11px] leading-4 text-content-muted">
              {t('modelCouncil.debateRoundsHelp')}
            </p>
          </div>

          <div className="space-y-2">
            <label
              htmlFor="model-council-judge-mode"
              className="text-xs font-medium text-content-secondary">
              {t('modelCouncil.judgeAgentLabel')}
            </label>
            <select
              id="model-council-judge-mode"
              value={judgeMode}
              onChange={e => {
                const mode = e.target.value as SeatMode;
                setJudgeMode(mode);
                setJudgeModel(mode === 'default' ? DEFAULT_JUDGE_MODEL : '');
              }}
              className="w-full rounded-lg border border-line bg-surface px-3 py-1.5 text-sm text-content focus:outline-none focus:ring-2 focus:ring-primary-400 dark:bg-surface-canvas">
              <option value="default">{t('modelCouncil.defaultJudge')}</option>
              <option value="profile">{t('modelCouncil.savedProfile')}</option>
              <option value="custom">{t('modelCouncil.customAgent')}</option>
            </select>

            {judgeMode === 'profile' && (
              <select
                value={judgeProfileId}
                aria-label={t('modelCouncil.judgeProfileLabel')}
                onChange={e => {
                  setJudgeProfileId(e.target.value);
                  setJudgeModel('');
                }}
                className="w-full rounded-lg border border-line bg-surface px-3 py-1.5 text-sm text-content focus:outline-none focus:ring-2 focus:ring-primary-400 dark:bg-surface-canvas">
                <option value="">{t('modelCouncil.chooseProfile')}</option>
                {profiles.map(profile => (
                  <option key={profile.id} value={profile.id}>
                    {profileLabel(profile)}
                  </option>
                ))}
              </select>
            )}

            {judgeMode === 'custom' && (
              <input
                type="text"
                value={judgeName}
                onChange={e => setJudgeName(e.target.value)}
                aria-label={t('modelCouncil.judgeNameLabel')}
                placeholder={t('modelCouncil.judgeNamePlaceholder')}
                className="w-full rounded-lg border border-line bg-surface px-3 py-1.5 text-sm text-content focus:outline-none focus:ring-2 focus:ring-primary-400 dark:bg-surface-canvas"
              />
            )}

            <button
              type="button"
              onClick={() =>
                setModelPicker({
                  title: t('modelCouncil.chairLabel'),
                  value: judgeModel,
                  onSelect: setJudgeModel,
                })
              }
              aria-label={t('modelCouncil.chairLabel')}
              className="flex w-full items-center justify-between gap-2 rounded-lg border border-line bg-surface px-3 py-1.5 text-left font-mono text-sm text-content hover:bg-surface-muted focus:outline-none focus:ring-2 focus:ring-primary-400 dark:bg-surface-canvas dark:hover:bg-surface">
              <span className="truncate">{judgeModel || DEFAULT_JUDGE_MODEL}</span>
              <span className="shrink-0 text-[11px] font-semibold text-primary-600 dark:text-primary-300">
                {t('modelCouncil.selectModel')}
              </span>
            </button>
          </div>
        </aside>
      )}

      {view === 'edit' && (
        <section aria-labelledby="model-council-roster-heading" className="space-y-3">
          <div className="flex flex-wrap items-end justify-between gap-2">
            <div>
              <h3 id="model-council-roster-heading" className="text-sm font-semibold text-content">
                {t('modelCouncil.rosterHeading')}
              </h3>
              <p className="text-xs text-content-muted">{t('modelCouncil.rosterHelp')}</p>
            </div>
            {profileStatus === 'loading' && (
              <span className="text-xs text-content-muted">
                {t('modelCouncil.loadingProfiles')}
              </span>
            )}
          </div>

          <div className="grid gap-3 md:grid-cols-2 xl:grid-cols-3">
            {seats.map((seat, index) => {
              const resolved = resolvedSeats[index];
              const colors = mascotColors(index);
              const activeFace = running
                ? ACTIVE_SEAT_FACES[index % ACTIVE_SEAT_FACES.length]
                : SEAT_FACES[index % SEAT_FACES.length];
              return (
                <article
                  key={seat.id}
                  className={`rounded-lg border bg-surface p-3 shadow-sm transition-colors ${
                    running
                      ? 'border-primary-300 ring-2 ring-primary-100 dark:border-primary-500/50 dark:ring-primary-500/10'
                      : 'border-line'
                  }`}>
                  <div className="flex gap-3">
                    <div
                      className={`h-20 w-20 shrink-0 overflow-hidden rounded-lg bg-surface-subtle ${
                        running ? 'animate-pulse' : ''
                      }`}>
                      <RiveMascot
                        size="100%"
                        face={activeFace}
                        primaryColor={colors.primaryColor}
                        secondaryColor={colors.secondaryColor}
                      />
                    </div>
                    <div className="min-w-0 flex-1 space-y-2">
                      <div className="flex items-center justify-between gap-2">
                        <p className="truncate text-sm font-semibold text-content">
                          {resolved.label}
                        </p>
                        <span className="rounded-md bg-surface-subtle px-1.5 py-0.5 text-[10px] font-semibold uppercase text-content-muted">
                          {t('modelCouncil.jurorLabel').replace('{n}', String(index + 1))}
                        </span>
                      </div>

                      <div
                        role="tablist"
                        aria-label={t('modelCouncil.profileModeLabel')}
                        className="grid grid-cols-3 gap-1">
                        {(['default', 'profile', 'custom'] as SeatMode[]).map(mode => (
                          <button
                            key={mode}
                            type="button"
                            role="tab"
                            aria-selected={seat.mode === mode}
                            onClick={() => setSeatMode(seat, mode)}
                            className={`rounded-md px-2 py-1 text-[11px] font-medium ${
                              seat.mode === mode
                                ? 'bg-primary-500 text-content-inverted'
                                : 'bg-surface-subtle text-content-secondary hover:bg-surface-strong dark:hover:bg-neutral-700'
                            }`}>
                            {t(`modelCouncil.mode.${mode}`)}
                          </button>
                        ))}
                      </div>
                    </div>
                  </div>

                  <div className="mt-3 space-y-2">
                    {seat.mode === 'profile' ? (
                      <select
                        value={seat.profileId}
                        aria-label={t('modelCouncil.memberProfileAria').replace(
                          '{n}',
                          String(index + 1)
                        )}
                        onChange={e =>
                          updateSeat(seat.id, { profileId: e.target.value, model: '' })
                        }
                        className="w-full rounded-lg border border-line bg-surface px-3 py-1.5 text-sm text-content focus:outline-none focus:ring-2 focus:ring-primary-400 dark:bg-surface-canvas">
                        <option value="">{t('modelCouncil.chooseProfile')}</option>
                        {profiles.map(profile => (
                          <option key={profile.id} value={profile.id}>
                            {profileLabel(profile)}
                          </option>
                        ))}
                      </select>
                    ) : (
                      <input
                        type="text"
                        value={seat.name}
                        onChange={e => updateSeat(seat.id, { name: e.target.value })}
                        aria-label={t('modelCouncil.memberNameAria').replace(
                          '{n}',
                          String(index + 1)
                        )}
                        placeholder={t('modelCouncil.memberNamePlaceholder')}
                        className="w-full rounded-lg border border-line bg-surface px-3 py-1.5 text-sm text-content focus:outline-none focus:ring-2 focus:ring-primary-400 dark:bg-surface-canvas"
                      />
                    )}

                    <button
                      type="button"
                      onClick={() =>
                        setModelPicker({
                          title: t('modelCouncil.memberAria').replace('{n}', String(index + 1)),
                          value: seat.model,
                          onSelect: model => updateSeat(seat.id, { model }),
                        })
                      }
                      aria-label={t('modelCouncil.memberAria').replace('{n}', String(index + 1))}
                      className="flex w-full items-center justify-between gap-2 rounded-lg border border-line bg-surface px-3 py-1.5 text-left font-mono text-sm text-content hover:bg-surface-muted focus:outline-none focus:ring-2 focus:ring-primary-400 dark:bg-surface-canvas dark:hover:bg-surface">
                      <span className="truncate">{seat.model || DEFAULT_MODEL}</span>
                      <span className="shrink-0 text-[11px] font-semibold text-primary-600 dark:text-primary-300">
                        {t('modelCouncil.selectModel')}
                      </span>
                    </button>

                    <textarea
                      value={seat.brief}
                      onChange={e => updateSeat(seat.id, { brief: e.target.value })}
                      rows={2}
                      aria-label={t('modelCouncil.memberBriefAria').replace(
                        '{n}',
                        String(index + 1)
                      )}
                      placeholder={t('modelCouncil.memberBriefPlaceholder')}
                      className="w-full rounded-lg border border-line bg-surface px-3 py-1.5 text-xs text-content-secondary resize-none focus:outline-none focus:ring-2 focus:ring-primary-400 dark:bg-surface-canvas"
                    />
                  </div>
                </article>
              );
            })}
          </div>
        </section>
      )}

      {view === 'run' && running && (
        <section
          aria-labelledby="model-council-deliberation-heading"
          className="space-y-3 rounded-lg border border-primary-200 bg-primary-50/60 p-3 dark:border-primary-500/30 dark:bg-primary-500/10">
          <div className="flex flex-wrap items-center justify-between gap-2">
            <div>
              <h3
                id="model-council-deliberation-heading"
                className="text-sm font-semibold text-content">
                {t('modelCouncil.deliberationHeading')}
              </h3>
              <p className="text-xs text-content-secondary">{t('modelCouncil.deliberationHelp')}</p>
            </div>
            <span
              role="status"
              aria-live="polite"
              className="rounded-md bg-surface px-2 py-1 text-xs font-medium text-primary-700 shadow-sm dark:bg-surface-canvas dark:text-primary-200">
              {t('modelCouncil.runningHint')}
            </span>
          </div>

          <div className="grid gap-2 md:grid-cols-2 xl:grid-cols-3">
            {resolvedSeats.map((seat, index) => {
              const colors = mascotColors(index);
              const liveMember = liveMembers[index];
              const answered = liveMember?.status === 'answered';
              const failed = liveMember?.status === 'failed';
              const turns = liveMember?.turns ?? [];
              const hasTurns = turns.length > 0;
              const waitingText = deliberationThought(seat, index, t);
              return (
                <div
                  key={`${seat.label}-${index}`}
                  className={`rounded-lg border bg-surface/90 p-3 shadow-sm dark:bg-surface-canvas/80 ${
                    failed
                      ? 'border-coral-200 dark:border-coral-500/30'
                      : answered
                        ? 'border-sage-200 dark:border-sage-500/30'
                        : 'border-white/80 dark:border-line'
                  }`}>
                  <div className="flex items-start gap-3">
                    <div
                      className={`h-14 w-14 shrink-0 overflow-hidden rounded-lg bg-surface-subtle ${
                        liveMember?.status === 'pending' || !liveMember ? 'animate-pulse' : ''
                      }`}>
                      <RiveMascot
                        size="100%"
                        face={
                          failed
                            ? 'curious'
                            : answered
                              ? 'proud'
                              : ACTIVE_SEAT_FACES[index % ACTIVE_SEAT_FACES.length]
                        }
                        primaryColor={colors.primaryColor}
                        secondaryColor={colors.secondaryColor}
                      />
                    </div>
                    <div className="min-w-0 flex-1">
                      <div className="flex items-center justify-between gap-2">
                        <p className="truncate text-sm font-semibold text-content">{seat.label}</p>
                        <span
                          className={`shrink-0 rounded px-1.5 py-0.5 text-[9px] font-semibold uppercase ${
                            failed
                              ? 'bg-coral-100 text-coral-700 dark:bg-coral-500/20 dark:text-coral-300'
                              : answered
                                ? 'bg-sage-100 text-sage-700 dark:bg-sage-500/20 dark:text-sage-300'
                                : 'bg-primary-100 text-primary-700 dark:bg-primary-500/20 dark:text-primary-200'
                          }`}>
                          {failed
                            ? t('modelCouncil.memberFailed')
                            : answered
                              ? t('modelCouncil.memberAnswered')
                              : t('modelCouncil.thinkingBadge')}
                        </span>
                      </div>
                      <div className="mt-2 max-h-52 space-y-1.5 overflow-y-auto pr-1">
                        {hasTurns ? (
                          turns.map(turn => (
                            <div
                              key={`${seat.label}-${index}-${turn.round}`}
                              className={`rounded-md border px-2 py-1.5 ${
                                turn.error
                                  ? 'border-coral-100 bg-coral-50/70 dark:border-coral-500/20 dark:bg-coral-500/10'
                                  : 'border-line bg-surface-muted'
                              }`}>
                              <p className="text-[10px] font-semibold uppercase text-content-muted">
                                {t('modelCouncil.roundLabel').replace(
                                  '{round}',
                                  String(turn.round)
                                )}
                              </p>
                              {turn.error ? (
                                <p className="mt-0.5 line-clamp-4 whitespace-pre-wrap text-xs text-coral-600 dark:text-coral-300">
                                  {turn.error}
                                </p>
                              ) : (
                                <div className="mt-0.5 text-content-secondary [&_.prose]:text-xs [&_.prose]:leading-5 [&_.prose_p]:my-0">
                                  <BubbleMarkdown content={turn.response || ''} />
                                </div>
                              )}
                            </div>
                          ))
                        ) : (
                          <p className="line-clamp-5 whitespace-pre-wrap text-xs text-content-secondary">
                            {waitingText}
                          </p>
                        )}
                        {liveMember?.status === 'pending' && hasTurns && (
                          <p className="text-[11px] text-primary-700 dark:text-primary-200">
                            {t('modelCouncil.currentRoundThinking')}
                          </p>
                        )}
                      </div>
                    </div>
                  </div>
                </div>
              );
            })}

            <div className="rounded-lg border border-primary-200 bg-surface p-3 shadow-sm dark:border-primary-500/30 dark:bg-surface-canvas">
              <div className="flex items-start gap-3">
                <div className="h-14 w-14 shrink-0 overflow-hidden rounded-lg bg-surface-subtle">
                  <RiveMascot size="100%" face="reading" />
                </div>
                <div className="min-w-0 flex-1">
                  <div className="flex items-center justify-between gap-2">
                    <p className="truncate text-sm font-semibold text-content">
                      {resolvedJudgeName}
                    </p>
                    <span className="shrink-0 rounded bg-amber-100 px-1.5 py-0.5 text-[9px] font-semibold uppercase text-amber-700 dark:bg-amber-500/20 dark:text-amber-200">
                      {judgeSynthesizing
                        ? t('modelCouncil.judgeSynthesizingBadge')
                        : t('modelCouncil.judgeWaitingBadge')}
                    </span>
                  </div>
                  <p className="mt-1 text-xs text-content-secondary">
                    {judgeSynthesizing
                      ? t('modelCouncil.judgeSynthesizingThought')
                      : t('modelCouncil.judgeWaitingThought')}
                  </p>
                </div>
              </div>
            </div>
          </div>

          <div className="rounded-lg border border-line bg-surface p-3 shadow-sm dark:bg-surface-canvas">
            <div className="flex flex-wrap items-center justify-between gap-2">
              <div>
                <h4 className="font-mono text-sm font-semibold text-content">
                  {SHARED_REASONING_FILE}
                </h4>
                <p className="text-xs text-content-muted">{t('modelCouncil.liveScratchpadHelp')}</p>
              </div>
              <span className="rounded bg-primary-100 px-1.5 py-0.5 text-[9px] font-semibold uppercase text-primary-700 dark:bg-primary-500/20 dark:text-primary-200">
                {t('modelCouncil.liveScratchpadBadge')}
              </span>
            </div>
            <div className="mt-3 max-h-72 overflow-y-auto rounded-md border border-line bg-surface-muted px-3 py-2 text-content-secondary [&_.prose]:text-xs [&_.prose]:leading-5">
              <BubbleMarkdown content={liveScratchpad || sharedReasoning} />
            </div>
          </div>
        </section>
      )}

      {view === 'run' && (
        <div className="flex flex-wrap items-center gap-2">
          <Button variant="primary" onClick={() => void handleRun()} disabled={!canRun}>
            {running ? t('modelCouncil.running') : t('modelCouncil.run')}
          </Button>
          {running && (
            <span role="status" aria-live="polite" className="text-xs text-content-muted">
              {t('modelCouncil.runningHint')}
            </span>
          )}
        </div>
      )}

      {view === 'run' && error && (
        <p role="alert" className="text-xs text-coral-700 dark:text-coral-300">
          {t('modelCouncil.errorPrefix')} {error}
        </p>
      )}

      {view === 'run' && result && (
        <section aria-labelledby="model-council-results-heading" className="space-y-3 pt-1">
          <h3 id="model-council-results-heading" className="text-sm font-semibold text-content">
            {t('modelCouncil.resultsHeading')}
          </h3>

          <div className="grid gap-2 sm:grid-cols-2">
            {result.members.map((member, index) => (
              <div
                key={`${member.model}-${index}`}
                className="rounded-lg border border-line p-3 space-y-1.5">
                <div className="flex items-center justify-between gap-2">
                  <span className="truncate font-mono text-xs font-medium text-content-secondary">
                    {member.model}
                  </span>
                  <span
                    className={`inline-flex shrink-0 items-center rounded px-1.5 py-0.5 text-[9px] font-semibold uppercase tracking-wide ${
                      member.error
                        ? 'bg-coral-100 text-coral-700 dark:bg-coral-500/20 dark:text-coral-300'
                        : 'bg-sage-100 text-sage-700 dark:bg-sage-500/20 dark:text-sage-300'
                    }`}>
                    {member.error
                      ? t('modelCouncil.memberFailed')
                      : t('modelCouncil.memberAnswered')}
                  </span>
                </div>
                {member.error ? (
                  <p className="text-xs text-coral-600 dark:text-coral-400">{member.error}</p>
                ) : (
                  <div className="break-words text-content-secondary [&_.prose]:text-xs [&_.prose]:leading-5 [&_.prose_p]:my-0">
                    <BubbleMarkdown content={member.response || ''} />
                  </div>
                )}
              </div>
            ))}
          </div>

          <div className="rounded-lg border border-primary-200 bg-primary-50 p-3 space-y-1 dark:border-primary-500/30 dark:bg-primary-500/10">
            <div className="flex items-center justify-between gap-2">
              <h4 className="text-xs font-semibold text-content">
                {t('modelCouncil.synthesisHeading')}
              </h4>
              <span className="truncate font-mono text-[10px] text-content-muted">
                {t('modelCouncil.synthesisBy').replace('{model}', result.chair_model)}
              </span>
            </div>
            <div className="break-words text-content-secondary [&_.prose]:text-sm [&_.prose]:leading-6">
              <BubbleMarkdown content={result.synthesis} />
            </div>
          </div>

          {usageEstimate && (
            <div className="rounded-lg border border-line bg-surface p-3 shadow-sm">
              <div className="flex flex-wrap items-center justify-between gap-2">
                <div>
                  <h4 className="text-xs font-semibold text-content">
                    {t('modelCouncil.usageHeading')}
                  </h4>
                  <p className="text-[11px] text-content-muted">
                    {t('modelCouncil.usageEstimated')}
                  </p>
                </div>
                <span className="rounded-md bg-surface-subtle px-2 py-1 text-xs font-semibold text-content-secondary">
                  {t('modelCouncil.usageEstimatedBadge')}
                </span>
              </div>
              <dl className="mt-3 grid gap-2 sm:grid-cols-3">
                <div className="rounded-md bg-surface-muted px-2 py-1.5">
                  <dt className="text-[10px] uppercase text-content-muted">
                    {t('modelCouncil.usageInputTokens')}
                  </dt>
                  <dd className="font-mono text-sm font-semibold text-content">
                    {formatTokenCount(usageEstimate.inputTokens)}
                  </dd>
                </div>
                <div className="rounded-md bg-surface-muted px-2 py-1.5">
                  <dt className="text-[10px] uppercase text-content-muted">
                    {t('modelCouncil.usageOutputTokens')}
                  </dt>
                  <dd className="font-mono text-sm font-semibold text-content">
                    {formatTokenCount(usageEstimate.outputTokens)}
                  </dd>
                </div>
                <div className="rounded-md bg-surface-muted px-2 py-1.5">
                  <dt className="text-[10px] uppercase text-content-muted">
                    {t('modelCouncil.usageTotalTokens')}
                  </dt>
                  <dd className="font-mono text-sm font-semibold text-content">
                    {formatTokenCount(usageEstimate.totalTokens)}
                  </dd>
                </div>
              </dl>
            </div>
          )}
        </section>
      )}

      {modelPicker && (
        <ModelPickerDialog picker={modelPicker} onClose={() => setModelPicker(null)} />
      )}
    </div>
  );
};

export default ModelCouncilTab;
