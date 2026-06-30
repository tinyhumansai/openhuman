/**
 * ProfileEditorPage — Settings > Agent Profiles > (New | Edit).
 *
 * Full-page editor for an agent *profile* (a "flavour": custom name + SOUL.md,
 * runtime defaults, and per-profile allowlists for memory sources, connectors,
 * skills, and MCP servers). Distinct from the agent *registry* editor
 * (`AgentEditorPage`), which edits sub-agent definitions.
 *
 * Routes: `/settings/profiles/new` and `/settings/profiles/edit/:id`.
 *
 * Each allowlist follows the "All / Selected" contract: `null`/`undefined` =
 * all (unrestricted); an array = restrict to those ids (empty = none).
 */
import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { LuX } from 'react-icons/lu';
import { useLocation, useNavigate, useParams } from 'react-router-dom';

import { useT } from '../../../lib/i18n/I18nContext';
import { selectAgentProfiles, upsertAgentProfile } from '../../../store/agentProfileSlice';
import { useAppDispatch, useAppSelector } from '../../../store/hooks';
import type { AgentProfile } from '../../../types/agentProfile';
import Button from '../../ui/Button';
import {
  SettingsRow,
  SettingsSection,
  SettingsSwitch,
  SettingsTextArea,
  SettingsTextField,
} from '../controls';
import SettingsPanel from '../layout/SettingsPanel';
import { settingsNavState } from '../modal/settingsOverlay';

const MODEL_HINTS = ['hint:reasoning', 'hint:chat', 'hint:agentic', 'hint:coding'];

function slugify(name: string): string {
  return name
    .trim()
    .toLowerCase()
    .replace(/[^a-z0-9_-]+/g, '-')
    .replace(/^-+|-+$/g, '');
}

/** Normalize an allowlist for save: empty array stays `[]` (= none selected);
 * `null` means "all". The chip editor only produces arrays, so callers pass
 * `null` explicitly via the All/Selected toggle. */
type Allowlist = string[] | null;

const ProfileEditorPage = () => {
  const { t } = useT();
  const navigate = useNavigate();
  const location = useLocation();
  const dispatch = useAppDispatch();
  const { id: routeId } = useParams<{ id: string }>();
  const profiles = useAppSelector(selectAgentProfiles);
  const backToList = useCallback(
    () => navigate('/settings/profiles', settingsNavState(location)),
    [navigate, location]
  );
  const isCreate = !routeId;

  const existing = useMemo(
    () => (routeId ? profiles.find(p => p.id === routeId) : undefined),
    [profiles, routeId]
  );

  const [name, setName] = useState('');
  const [agentId, setAgentId] = useState('orchestrator');
  const [idTouched, setIdTouched] = useState(!isCreate);
  const [profileId, setProfileId] = useState('');
  const [description, setDescription] = useState('');
  const [model, setModel] = useState('');
  const [temperature, setTemperature] = useState('');
  const [systemPromptSuffix, setSystemPromptSuffix] = useState('');
  const [soulMd, setSoulMd] = useState('');
  const [includeAgentConversations, setIncludeAgentConversations] = useState(true);
  const [memorySources, setMemorySources] = useState<Allowlist>(null);
  const [composioIntegrations, setComposioIntegrations] = useState<Allowlist>(null);
  const [allowedSkills, setAllowedSkills] = useState<Allowlist>(null);
  const [allowedMcpServers, setAllowedMcpServers] = useState<Allowlist>(null);

  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [notFound, setNotFound] = useState(false);
  const mountedRef = useRef(true);

  useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
    };
  }, []);

  // Hydrate form state from the loaded profile (edit mode). This intentionally
  // fans out into multiple setters in an effect: the source is async Redux
  // state that may arrive after mount, so a keyed remount / lazy initial state
  // can't capture it. Mirrors the suppression used by other settings panels.

  useEffect(() => {
    if (isCreate) return;
    if (!existing) {
      // Profiles may still be loading; only flag not-found once a list exists.
      if (profiles.length > 0) setNotFound(true);
      return;
    }
    setNotFound(false);
    setName(existing.name);
    setAgentId(existing.agentId || 'orchestrator');
    setProfileId(existing.id);
    setDescription(existing.description ?? '');
    setModel(existing.modelOverride ?? '');
    setTemperature(
      existing.temperature === null || existing.temperature === undefined
        ? ''
        : String(existing.temperature)
    );
    setSystemPromptSuffix(existing.systemPromptSuffix ?? '');
    setSoulMd(existing.soulMd ?? '');
    setIncludeAgentConversations(existing.includeAgentConversations ?? true);
    setMemorySources(existing.memorySources ?? null);
    setComposioIntegrations(existing.composioIntegrations ?? null);
    setAllowedSkills(existing.allowedSkills ?? null);
    setAllowedMcpServers(existing.allowedMcpServers ?? null);
  }, [existing, isCreate, profiles.length]);

  const handleName = (value: string) => {
    setName(value);
    if (isCreate && !idTouched) setProfileId(slugify(value));
  };

  // Resolved profile id: explicit id on create (falling back to a slug of the
  // name), or the existing id on edit. Must be non-empty to submit — a
  // punctuation-only name slugs to '' and must not reach the RPC layer.
  const resolvedId = (isCreate ? profileId.trim() || slugify(name) : profileId).trim();

  const canSubmit = !submitting && (isCreate ? resolvedId.length > 0 : true);

  const handleSubmit = async () => {
    if (!canSubmit) return;
    setSubmitting(true);
    setError(null);
    const id = resolvedId;
    if (!id) {
      setError(t('settings.profiles.editor.idRequired'));
      setSubmitting(false);
      return;
    }
    const tempNum = temperature.trim() === '' ? null : Number(temperature);
    const profile: AgentProfile = {
      id,
      name: name.trim() || id,
      description: description.trim(),
      agentId: agentId.trim() || 'orchestrator',
      modelOverride: model.trim() || null,
      temperature: tempNum !== null && Number.isFinite(tempNum) ? tempNum : null,
      systemPromptSuffix: systemPromptSuffix.trim() || null,
      soulMd: soulMd.trim() || null,
      includeAgentConversations,
      memorySources,
      composioIntegrations,
      allowedSkills,
      allowedMcpServers,
      builtIn: existing?.builtIn ?? false,
    };
    try {
      await dispatch(upsertAgentProfile(profile)).unwrap();
      if (mountedRef.current) backToList();
    } catch (err) {
      if (mountedRef.current) setError(err instanceof Error ? err.message : String(err));
    } finally {
      if (mountedRef.current) setSubmitting(false);
    }
  };

  return (
    <SettingsPanel
      title={
        isCreate
          ? t('settings.profiles.editor.createTitle')
          : t('settings.profiles.editor.editTitle')
      }
      description={t('settings.profiles.menuDesc')}>
      {notFound ? (
        <div className="space-y-3">
          <div className="rounded-lg border border-coral-200 bg-coral-50 px-4 py-3 text-sm text-coral-700 dark:border-coral-500/30 dark:bg-coral-500/10 dark:text-coral-300">
            {t('settings.profiles.editor.notFound')}
          </div>
        </div>
      ) : (
        <div className="space-y-5">
          {/* Identity */}
          <SettingsSection>
            <SettingsRow
              htmlFor="profile-name"
              label={t('settings.profiles.editor.name')}
              stacked
              control={
                <SettingsTextField
                  id="profile-name"
                  autoFocus={isCreate}
                  value={name}
                  onChange={e => handleName(e.target.value)}
                  aria-label={t('settings.profiles.editor.name')}
                />
              }
            />
            {isCreate ? (
              <SettingsRow
                htmlFor="profile-id"
                label={t('settings.profiles.editor.id')}
                description={t('settings.profiles.editor.idHint')}
                stacked
                control={
                  <SettingsTextField
                    id="profile-id"
                    mono
                    value={profileId}
                    onChange={e => {
                      setIdTouched(true);
                      setProfileId(e.target.value);
                    }}
                    aria-label={t('settings.profiles.editor.id')}
                  />
                }
              />
            ) : (
              <SettingsRow
                label={t('settings.profiles.editor.id')}
                control={<code className="font-mono text-xs text-content-muted">{profileId}</code>}
              />
            )}
            <SettingsRow
              htmlFor="profile-description"
              label={t('settings.profiles.editor.description')}
              stacked
              control={
                <SettingsTextArea
                  id="profile-description"
                  value={description}
                  onChange={e => setDescription(e.target.value)}
                  rows={2}
                  aria-label={t('settings.profiles.editor.description')}
                />
              }
            />
          </SettingsSection>

          {/* Soul */}
          <SettingsSection>
            <SettingsRow
              htmlFor="profile-soul"
              label={t('settings.profiles.editor.soul')}
              description={t('settings.profiles.editor.soulHint')}
              stacked
              control={
                <SettingsTextArea
                  id="profile-soul"
                  value={soulMd}
                  onChange={e => setSoulMd(e.target.value)}
                  rows={6}
                  aria-label={t('settings.profiles.editor.soul')}
                />
              }
            />
          </SettingsSection>

          {/* Runtime defaults */}
          <SettingsSection>
            <SettingsRow
              htmlFor="profile-base-agent"
              label={t('settings.profiles.editor.baseAgent')}
              description={t('settings.profiles.editor.baseAgentHint')}
              stacked
              control={
                <SettingsTextField
                  id="profile-base-agent"
                  mono
                  value={agentId}
                  onChange={e => setAgentId(e.target.value)}
                  aria-label={t('settings.profiles.editor.baseAgent')}
                />
              }
            />
            <SettingsRow
              htmlFor="profile-model"
              label={t('settings.profiles.editor.model')}
              description={t('settings.profiles.editor.modelHint')}
              stacked
              control={
                <SettingsTextField
                  id="profile-model"
                  mono
                  value={model}
                  onChange={e => setModel(e.target.value)}
                  placeholder={MODEL_HINTS.join(', ')}
                  aria-label={t('settings.profiles.editor.model')}
                />
              }
            />
            <SettingsRow
              htmlFor="profile-temperature"
              label={t('settings.profiles.editor.temperature')}
              stacked
              control={
                <SettingsTextField
                  id="profile-temperature"
                  value={temperature}
                  onChange={e => setTemperature(e.target.value)}
                  placeholder="0.0 – 1.0"
                  aria-label={t('settings.profiles.editor.temperature')}
                />
              }
            />
            <SettingsRow
              htmlFor="profile-suffix"
              label={t('settings.profiles.editor.systemPromptSuffix')}
              stacked
              control={
                <SettingsTextArea
                  id="profile-suffix"
                  value={systemPromptSuffix}
                  onChange={e => setSystemPromptSuffix(e.target.value)}
                  rows={2}
                  aria-label={t('settings.profiles.editor.systemPromptSuffix')}
                />
              }
            />
          </SettingsSection>

          {/* Memory */}
          <SettingsSection>
            <SettingsRow
              label={t('settings.profiles.editor.agentConversations')}
              description={t('settings.profiles.editor.agentConversationsHint')}
              control={
                <SettingsSwitch
                  id="profile-agent-conversations"
                  checked={includeAgentConversations}
                  onCheckedChange={setIncludeAgentConversations}
                  aria-label={t('settings.profiles.editor.agentConversations')}
                />
              }
            />
            <AllowlistField
              label={t('settings.profiles.editor.memorySources')}
              hint={t('settings.profiles.editor.memorySourcesHint')}
              value={memorySources}
              onChange={setMemorySources}
            />
          </SettingsSection>

          {/* Capabilities */}
          <SettingsSection>
            <AllowlistField
              label={t('settings.profiles.editor.connectors')}
              hint={t('settings.profiles.editor.connectorsHint')}
              value={composioIntegrations}
              onChange={setComposioIntegrations}
            />
            <AllowlistField
              label={t('settings.profiles.editor.skills')}
              hint={t('settings.profiles.editor.skillsHint')}
              value={allowedSkills}
              onChange={setAllowedSkills}
            />
            <AllowlistField
              label={t('settings.profiles.editor.mcpServers')}
              hint={t('settings.profiles.editor.mcpServersHint')}
              value={allowedMcpServers}
              onChange={setAllowedMcpServers}
            />
          </SettingsSection>

          {error && (
            <p className="rounded-md border border-coral-200 bg-coral-50 px-3 py-2 text-xs text-coral-700 dark:border-coral-500/30 dark:bg-coral-500/10 dark:text-coral-300">
              {error}
            </p>
          )}

          <div className="flex justify-end gap-2 pt-1">
            <Button type="button" variant="secondary" size="sm" onClick={backToList}>
              {t('common.cancel')}
            </Button>
            <Button
              type="button"
              variant="primary"
              size="sm"
              onClick={() => void handleSubmit()}
              disabled={!canSubmit}>
              {submitting
                ? t('settings.profiles.editor.saving')
                : isCreate
                  ? t('common.create')
                  : t('common.save')}
            </Button>
          </div>
        </div>
      )}
    </SettingsPanel>
  );
};

/**
 * "All / Selected" allowlist editor. `null` value = all (unrestricted); an
 * array switches to restricted mode with a chip editor. Toggling back to All
 * emits `null`; toggling to Selected emits the current array (or `[]`).
 */
function AllowlistField({
  label,
  hint,
  value,
  onChange,
}: {
  label: string;
  hint: string;
  value: Allowlist;
  onChange: (next: Allowlist) => void;
}) {
  const { t } = useT();
  const [draft, setDraft] = useState('');
  const restricted = value !== null;
  const items = value ?? [];

  const commitDraft = () => {
    const next = draft
      .split(',')
      .map(s => s.trim())
      .filter(Boolean);
    if (next.length === 0) return;
    const merged = Array.from(new Set([...(value ?? []), ...next]));
    onChange(merged);
    setDraft('');
  };

  return (
    <SettingsRow
      label={label}
      description={hint}
      stacked
      control={
        <div className="space-y-2">
          <div className="inline-flex gap-1.5">
            <Button
              type="button"
              variant={!restricted ? 'primary' : 'secondary'}
              size="xs"
              onClick={() => onChange(null)}>
              {t('settings.profiles.editor.all')}
            </Button>
            <Button
              type="button"
              variant={restricted ? 'primary' : 'secondary'}
              size="xs"
              onClick={() => onChange(value ?? [])}>
              {t('settings.profiles.editor.selected')}
            </Button>
          </div>

          {restricted && (
            <div className="rounded-md border border-line p-2 dark:border-line-strong">
              <div className="mb-1.5 flex flex-wrap gap-1.5">
                {items.map(item => (
                  <span
                    key={item}
                    className="inline-flex items-center gap-1 rounded-full bg-surface-subtle px-2.5 py-1 font-mono text-xs text-content-secondary">
                    {item}
                    <button
                      type="button"
                      aria-label={t('settings.profiles.editor.removeAria').replace('{item}', item)}
                      onClick={() => onChange(items.filter(x => x !== item))}
                      className="rounded-full text-content-faint hover:text-coral-600 dark:hover:text-coral-300">
                      <LuX className="h-3 w-3" />
                    </button>
                  </span>
                ))}
              </div>
              <SettingsTextField
                mono
                value={draft}
                onChange={e => setDraft(e.target.value)}
                onKeyDown={e => {
                  if (e.key === 'Enter' || e.key === ',') {
                    e.preventDefault();
                    commitDraft();
                  }
                }}
                onBlur={commitDraft}
                placeholder={t('settings.profiles.editor.addPlaceholder')}
                aria-label={label}
              />
            </div>
          )}
        </div>
      }
    />
  );
}

export default ProfileEditorPage;
