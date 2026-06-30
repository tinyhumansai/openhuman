import { useState } from 'react';

import { useT } from '../../../lib/i18n/I18nContext';
import { channelLuminance } from '../../../lib/theme/color';
import { resolveFamilyVariant } from '../../../lib/theme/presets';
import {
  ACCENT_FAMILIES,
  ACCENT_SHADES,
  COLOR_GROUPS,
  FONT_CHOICES,
  FONT_ROLES,
  fontChoiceForStack,
  type FontRole,
} from '../../../lib/theme/tokens';
import type { BackdropKind, Theme } from '../../../lib/theme/types';
import { useAppDispatch, useAppSelector } from '../../../store/hooks';
import {
  deleteCustomTheme,
  resetActiveTheme,
  resolveTheme,
  selectActiveFamilyId,
  selectActiveThemeId,
  selectCustomThemes,
  selectEffectiveTheme,
  selectThemeFamilies,
  selectThemeVariant,
  setActiveFamily,
  setActiveTheme,
  setFontRole,
  setThemeBackdrop,
  setThemeToken,
  setThemeVariant,
  type ThemeVariant,
  upsertCustomTheme,
} from '../../../store/themeSlice';
import { SettingsSection, SettingsSelect } from '../controls';
import SettingsPanel from '../layout/SettingsPanel';
import ColorTokenField from './theme/ColorTokenField';

/** Minimal base swatch values used only for preview tiles of built-in themes. */
const BASE_SWATCH: Record<'light' | 'dark', Record<string, string>> = {
  light: {
    'surface-canvas': '245 245 245',
    surface: '255 255 255',
    content: '23 23 23',
    'primary-500': '47 110 244',
  },
  dark: {
    'surface-canvas': '0 0 0',
    surface: '23 23 23',
    content: '245 245 245',
    'primary-500': '47 110 244',
  },
};

/** Read the live effective value of a token (override or tokens.css default). */
function readToken(key: string): string {
  if (typeof document === 'undefined') return '0 0 0';
  const v = window.getComputedStyle(document.documentElement).getPropertyValue(`--${key}`).trim();
  return v || '0 0 0';
}

function readFontRole(role: FontRole): string {
  if (typeof document === 'undefined') return '';
  return window
    .getComputedStyle(document.documentElement)
    .getPropertyValue(`--font-${role}`)
    .trim();
}

/** "surface-canvas" → "Surface canvas". */
function humanize(key: string): string {
  const s = key.replace(/-/g, ' ');
  return s.charAt(0).toUpperCase() + s.slice(1);
}

function swatchChannels(theme: Theme, key: string): string {
  return theme.colors[key] ?? BASE_SWATCH[theme.isDark ? 'dark' : 'light'][key] ?? '128 128 128';
}

function channelsToCss(channels: string): string {
  return `rgb(${channels.trim().split(/\s+/).join(' ')} / 1)`;
}

/** Tile background: the theme's canvas gradient if any, else its flat canvas. */
function tileCanvas(theme: Theme): string {
  return theme.gradient?.canvas ?? channelsToCss(swatchChannels(theme, 'surface-canvas'));
}

function importedGradient(parsed: Partial<Theme>): Theme['gradient'] {
  if (!parsed.gradient || typeof parsed.gradient !== 'object') return undefined;
  return typeof parsed.gradient.canvas === 'string' ? { canvas: parsed.gradient.canvas } : {};
}

function importedBackdrop(parsed: Partial<Theme>): Theme['backdrop'] {
  if (!parsed.backdrop || typeof parsed.backdrop !== 'object') return undefined;
  const { kind } = parsed.backdrop;
  if (kind !== 'mesh' && kind !== 'solid' && kind !== 'image') return undefined;
  return {
    kind,
    imageUrl: typeof parsed.backdrop.imageUrl === 'string' ? parsed.backdrop.imageUrl : undefined,
    dots: typeof parsed.backdrop.dots === 'boolean' ? parsed.backdrop.dots : undefined,
  };
}

const ThemeStudioPanel = () => {
  const { t } = useT();
  const dispatch = useAppDispatch();
  const families = selectThemeFamilies();
  const customThemes = useAppSelector(selectCustomThemes);
  const activeThemeId = useAppSelector(selectActiveThemeId);
  const activeFamilyId = useAppSelector(selectActiveFamilyId);
  const variant = useAppSelector(selectThemeVariant);
  const effectiveTheme = useAppSelector(selectEffectiveTheme);

  const isActiveCustom = customThemes.some(th => th.id === activeThemeId);
  // Which variant to render in family preview tiles (Auto → resolved OS variant).
  const previewVariant: 'light' | 'dark' = variant === 'system' ? resolveTheme('system') : variant;

  const VARIANT_OPTIONS: { id: ThemeVariant; label: string }[] = [
    { id: 'light', label: t('settings.theme.variantLight', 'Light') },
    { id: 'dark', label: t('settings.theme.variantDark', 'Dark') },
    { id: 'system', label: t('settings.theme.variantAuto', 'Auto') },
  ];
  const [showAdvanced, setShowAdvanced] = useState(false);
  const [importText, setImportText] = useState('');
  const [importError, setImportError] = useState('');
  const [copied, setCopied] = useState(false);

  const handleExport = async () => {
    const active = customThemes.find(th => th.id === activeThemeId) ?? effectiveTheme;
    const json = JSON.stringify(active, null, 2);
    try {
      await navigator.clipboard?.writeText(json);
      setCopied(true);
      setTimeout(() => setCopied(false), 1500);
    } catch {
      // Clipboard blocked — the textarea below still shows the JSON to copy.
    }
  };

  const handleImport = () => {
    setImportError('');
    try {
      const parsed = JSON.parse(importText) as Partial<Theme>;
      if (!parsed || typeof parsed !== 'object' || typeof parsed.colors !== 'object') {
        throw new Error('shape');
      }
      const theme: Theme = {
        id: `custom-${Date.now()}`,
        name: parsed.name
          ? String(parsed.name)
          : t('settings.theme.importedName', 'Imported theme'),
        isDark: Boolean(parsed.isDark),
        builtIn: false,
        colors: { ...(parsed.colors as Record<string, string>) },
        fonts: { ...(parsed.fonts ?? {}) },
        gradient: importedGradient(parsed),
        backdrop: importedBackdrop(parsed),
      };
      dispatch(upsertCustomTheme(theme));
      setImportText('');
    } catch {
      setImportError(t('settings.theme.importError', 'Could not parse that theme JSON.'));
    }
  };

  const activeMeta = customThemes.find(th => th.id === activeThemeId);
  const exportJson = JSON.stringify(activeMeta ?? effectiveTheme, null, 2);

  // Contrast guard: warn if the editable theme's primary text on its canvas is low-contrast.
  const contrastRisk =
    isActiveCustom &&
    Math.abs(
      channelLuminance(readToken('content')) - channelLuminance(readToken('surface-canvas'))
    ) < 0.2;

  return (
    <SettingsPanel description={t('settings.theme.menuDesc', 'Customize colours and fonts.')}>
      {/* ── Theme gallery: family tiles + one Light/Dark/Auto toggle ──── */}
      <div>
        <div className="mb-2 flex items-center justify-between px-1">
          <h3 className="text-xs font-semibold uppercase tracking-wider text-content-faint">
            {t('settings.theme.presetsHeading', 'Themes')}
          </h3>
          <div
            className="inline-flex overflow-hidden rounded-lg border border-line"
            role="radiogroup"
            aria-label={t('settings.theme.variantAria', 'Theme variant')}>
            {VARIANT_OPTIONS.map(opt => {
              const sel = opt.id === variant;
              return (
                <button
                  key={opt.id}
                  type="button"
                  role="radio"
                  aria-checked={sel}
                  onClick={() => dispatch(setThemeVariant(opt.id))}
                  className={`px-2.5 py-1 text-xs font-medium transition-colors ${
                    sel
                      ? 'bg-primary-500 text-content-inverted'
                      : 'text-content-secondary hover:bg-surface-hover'
                  }`}>
                  {opt.label}
                </button>
              );
            })}
          </div>
        </div>
        <div className="grid grid-cols-2 gap-2 sm:grid-cols-3">
          {families.map(fam => {
            const preview = resolveFamilyVariant(fam, previewVariant);
            const selected = !isActiveCustom && fam.id === activeFamilyId;
            return (
              <button
                key={fam.id}
                type="button"
                aria-pressed={selected}
                onClick={() => dispatch(setActiveFamily(fam.id))}
                className={`flex flex-col gap-2 rounded-xl border p-3 text-left transition-colors ${
                  selected
                    ? 'border-primary-500 ring-1 ring-primary-500'
                    : 'border-line hover:bg-surface-hover'
                }`}>
                <span
                  className="flex h-10 items-center gap-1 rounded-lg px-2"
                  style={{ background: tileCanvas(preview) }}>
                  <span
                    className="h-5 w-5 rounded-full border border-black/10"
                    style={{ background: channelsToCss(swatchChannels(preview, 'surface')) }}
                  />
                  <span
                    className="h-3 w-8 rounded-full"
                    style={{ background: channelsToCss(swatchChannels(preview, 'content')) }}
                  />
                  <span
                    className="ml-auto h-4 w-4 rounded-full"
                    style={{ background: channelsToCss(swatchChannels(preview, 'primary-500')) }}
                  />
                </span>
                <span className="text-sm font-medium text-content truncate">{fam.name}</span>
              </button>
            );
          })}
          {customThemes.map(th => {
            const selected = th.id === activeThemeId;
            return (
              <button
                key={th.id}
                type="button"
                aria-pressed={selected}
                onClick={() => dispatch(setActiveTheme(th.id))}
                className={`flex flex-col gap-2 rounded-xl border p-3 text-left transition-colors ${
                  selected
                    ? 'border-primary-500 ring-1 ring-primary-500'
                    : 'border-line hover:bg-surface-hover'
                }`}>
                <span
                  className="flex h-10 items-center gap-1 rounded-lg px-2"
                  style={{ background: tileCanvas(th) }}>
                  <span
                    className="h-5 w-5 rounded-full border border-black/10"
                    style={{ background: channelsToCss(swatchChannels(th, 'surface')) }}
                  />
                  <span
                    className="h-3 w-8 rounded-full"
                    style={{ background: channelsToCss(swatchChannels(th, 'content')) }}
                  />
                  <span
                    className="ml-auto h-4 w-4 rounded-full"
                    style={{ background: channelsToCss(swatchChannels(th, 'primary-500')) }}
                  />
                </span>
                <span className="flex items-center justify-between gap-1">
                  <span className="text-sm font-medium text-content truncate">{th.name}</span>
                  <span className="text-[10px] uppercase tracking-wide text-content-faint">
                    {t('settings.theme.customBadge', 'Custom')}
                  </span>
                </span>
              </button>
            );
          })}
        </div>
      </div>

      {/* ── Editing hint (presets auto-fork) / contrast guard ──────── */}
      {!isActiveCustom && (
        <p className="px-1 text-xs text-content-muted">
          {t(
            'settings.theme.autoForkHint',
            'Editing a preset automatically saves your changes as a new custom theme.'
          )}
        </p>
      )}
      {isActiveCustom && contrastRisk && (
        <div className="rounded-xl border border-amber-200 bg-amber-50 p-3 dark:border-amber-500/30 dark:bg-amber-500/10">
          <p className="text-xs text-amber-700 dark:text-amber-300">
            {t(
              'settings.theme.contrastWarn',
              'Low contrast between text and background — this theme may be hard to read.'
            )}
          </p>
        </div>
      )}

      {/* ── Colour editor ──────────────────────────────────────────── */}
      {COLOR_GROUPS.map(group => (
        <SettingsSection key={group.id} title={t(group.i18nKey, humanize(group.id))}>
          <div className="px-1">
            {group.keys.map(key => (
              <ColorTokenField
                key={key}
                tokenKey={key}
                label={humanize(key)}
                value={effectiveTheme.colors[key] ?? readToken(key)}
                disabled={false}
                onChange={channels => dispatch(setThemeToken({ key, value: channels }))}
              />
            ))}
          </div>
        </SettingsSection>
      ))}

      {/* ── Advanced accent shades ─────────────────────────────────── */}
      <div>
        <button
          type="button"
          onClick={() => setShowAdvanced(v => !v)}
          className="text-xs font-medium text-primary-600 hover:underline dark:text-primary-300">
          {showAdvanced
            ? t('settings.theme.hideShades', 'Hide all accent shades')
            : t('settings.theme.showShades', 'Show all accent shades')}
        </button>
        {showAdvanced &&
          ACCENT_FAMILIES.map(fam => (
            <SettingsSection key={fam} title={humanize(fam)}>
              <div className="px-1">
                {ACCENT_SHADES.map(shade => {
                  const key = `${fam}-${shade}`;
                  return (
                    <ColorTokenField
                      key={key}
                      tokenKey={key}
                      label={`${humanize(fam)} ${shade}`}
                      value={effectiveTheme.colors[key] ?? readToken(key)}
                      disabled={false}
                      onChange={channels => dispatch(setThemeToken({ key, value: channels }))}
                    />
                  );
                })}
              </div>
            </SettingsSection>
          ))}
      </div>

      {/* ── Fonts ──────────────────────────────────────────────────── */}
      <SettingsSection title={t('settings.theme.fontsHeading', 'Fonts')}>
        <div className="space-y-2 px-1">
          {FONT_ROLES.map(role => {
            const current = fontChoiceForStack(effectiveTheme.fonts[role] ?? readFontRole(role));
            return (
              <div key={role} className="flex items-center justify-between gap-3">
                <span className="text-sm text-content">
                  {t(`settings.theme.fontRole.${role}`, humanize(role))}
                </span>
                <SettingsSelect
                  inputSize="sm"
                  value={current?.id ?? '__current__'}
                  disabled={false}
                  aria-label={t(`settings.theme.fontRole.${role}`, humanize(role))}
                  onChange={e => {
                    const choice = FONT_CHOICES.find(c => c.id === e.target.value);
                    if (choice) dispatch(setFontRole({ role, stack: choice.stack }));
                  }}>
                  {!current && (
                    <option value="__current__" disabled>
                      {t('settings.theme.fontCurrent', 'Current')}
                    </option>
                  )}
                  {FONT_CHOICES.map(c => (
                    <option key={c.id} value={c.id}>
                      {c.label}
                    </option>
                  ))}
                </SettingsSelect>
              </div>
            );
          })}
        </div>
      </SettingsSection>

      {/* ── Backdrop (mesh / solid / image) ────────────────────────── */}
      <SettingsSection title={t('settings.theme.backdropHeading', 'Background')}>
        <div className="space-y-2 px-1">
          <div
            className="inline-flex overflow-hidden rounded-lg border border-line"
            role="radiogroup"
            aria-label={t('settings.theme.backdropHeading', 'Background')}>
            {(['mesh', 'solid', 'image'] as BackdropKind[]).map(kind => {
              const current = effectiveTheme.backdrop?.kind ?? 'mesh';
              const sel = current === kind;
              return (
                <button
                  key={kind}
                  type="button"
                  role="radio"
                  aria-checked={sel}
                  disabled={false}
                  onClick={() =>
                    dispatch(
                      setThemeBackdrop({ kind, imageUrl: effectiveTheme.backdrop?.imageUrl })
                    )
                  }
                  className={`px-3 py-1.5 text-xs font-medium transition-colors disabled:opacity-50 ${
                    sel
                      ? 'bg-primary-500 text-content-inverted'
                      : 'text-content-secondary hover:bg-surface-hover'
                  }`}>
                  {t(`settings.theme.backdrop.${kind}`, kind)}
                </button>
              );
            })}
          </div>
          {effectiveTheme.backdrop?.kind === 'image' && (
            <input
              type="url"
              disabled={false}
              value={effectiveTheme.backdrop?.imageUrl ?? ''}
              placeholder="https://…/background.jpg"
              aria-label={t('settings.theme.backdropImageUrl', 'Background image URL')}
              onChange={e =>
                dispatch(setThemeBackdrop({ kind: 'image', imageUrl: e.target.value }))
              }
              className="w-full rounded-lg border border-line bg-surface px-2 py-1.5 text-xs text-content"
            />
          )}
          <label className="flex items-center gap-2 text-xs text-content-secondary">
            <input
              type="checkbox"
              checked={effectiveTheme.backdrop?.dots !== false}
              onChange={e => dispatch(setThemeBackdrop({ dots: e.target.checked }))}
              className="h-3.5 w-3.5 accent-primary-500"
            />
            {t('settings.theme.backdropDots', 'Show background dots')}
          </label>
          <p className="text-[11px] text-content-faint">
            {t(
              'settings.theme.backdropHint',
              'Mesh shows the animated gradient; Solid uses a flat background; Image paints your own.'
            )}
          </p>
        </div>
      </SettingsSection>

      {/* ── Actions: reset / delete / export / import ──────────────── */}
      {isActiveCustom && (
        <SettingsSection title={t('settings.theme.actions', 'Manage theme')}>
          <div className="flex flex-wrap gap-2 px-1">
            <button
              type="button"
              onClick={() => dispatch(resetActiveTheme())}
              className="rounded-lg border border-line px-3 py-1.5 text-sm text-content-secondary hover:bg-surface-hover">
              {t('settings.theme.reset', 'Reset overrides')}
            </button>
            <button
              type="button"
              onClick={() => dispatch(deleteCustomTheme(activeThemeId))}
              className="rounded-lg border border-coral-200 px-3 py-1.5 text-sm text-coral-600 hover:bg-coral-50 dark:border-coral-500/30 dark:text-coral-300 dark:hover:bg-coral-500/10">
              {t('settings.theme.delete', 'Delete theme')}
            </button>
            <button
              type="button"
              onClick={handleExport}
              className="rounded-lg border border-line px-3 py-1.5 text-sm text-content-secondary hover:bg-surface-hover">
              {copied
                ? t('settings.theme.copied', 'Copied!')
                : t('settings.theme.export', 'Copy JSON')}
            </button>
          </div>
          <div className="px-1 pt-2">
            <textarea
              readOnly
              value={exportJson}
              rows={4}
              aria-label={t('settings.theme.export', 'Copy JSON')}
              className="w-full resize-none rounded-lg border border-line bg-surface-muted p-2 font-mono text-[11px] text-content-secondary"
            />
          </div>
        </SettingsSection>
      )}

      {/* ── Import (always available) ──────────────────────────────── */}
      <SettingsSection
        title={t('settings.theme.import', 'Import theme')}
        description={t(
          'settings.theme.importHint',
          'Paste exported theme JSON to add it as a custom theme.'
        )}>
        <div className="space-y-2 px-1">
          <textarea
            value={importText}
            onChange={e => setImportText(e.target.value)}
            rows={4}
            placeholder='{ "name": "...", "isDark": false, "colors": { ... } }'
            aria-label={t('settings.theme.import', 'Import theme')}
            className="w-full resize-none rounded-lg border border-line bg-surface p-2 font-mono text-[11px] text-content"
          />
          {importError && (
            <p className="text-xs text-coral-600 dark:text-coral-300">{importError}</p>
          )}
          <button
            type="button"
            onClick={handleImport}
            disabled={!importText.trim()}
            className="rounded-lg bg-primary-500 px-3 py-1.5 text-sm font-medium text-content-inverted hover:bg-primary-600 disabled:opacity-50">
            {t('settings.theme.importApply', 'Import')}
          </button>
        </div>
      </SettingsSection>
    </SettingsPanel>
  );
};

export default ThemeStudioPanel;
