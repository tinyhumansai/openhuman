/**
 * CreateSkillForm
 * ----------------
 *
 * Body of the "create a new SKILL.md" flow, shared between
 * `CreateSkillModal` (modal chrome) and the `/skills/new` page wrapper.
 *
 * Owns:
 *   - All form fields (name, description, scope, license, author,
 *     tags, allowed-tools).
 *   - Slug preview + validation (name and description required).
 *   - Submit handler that calls `skillsApi.createSkill` and surfaces
 *     the result via `onCreated(skill)` / error string via inline
 *     `<div role="alert">`.
 *
 * Does NOT own:
 *   - The submit/cancel buttons (the wrapper provides them so the
 *     modal can use a footer bar and the page can render a top-right
 *     primary action).
 *   - Modal-specific concerns (focus capture, Escape-to-close,
 *     backdrop click). Those stay in `CreateSkillModal`.
 *
 * The wrapper drives submission by either calling the imperative
 * handle exposed via a ref (`<CreateSkillForm ref={ref} ... />` →
 * `ref.current.submit()`) OR by reading `formValid` + `submitting`
 * from the props the form raises and wiring its own submit button to
 * the underlying `<form>` via the standard `form="..."` attribute.
 * Both modal and page use the latter, so the form mounts a real
 * `<form id={formId}>` and they bind `<button form={formId}>`.
 */
import debug from 'debug';
import {
  forwardRef,
  useCallback,
  useEffect,
  useImperativeHandle,
  useMemo,
  useRef,
  useState,
} from 'react';

import { useT } from '../../lib/i18n/I18nContext';
import {
  type CreateSkillInput,
  type CreateSkillInputDef,
  type SkillScope,
  type SkillSummary,
  skillsApi,
} from '../../services/api/skillsApi';

/** Mirrors `SkillCreateInputDef` shape used as wire payload, with one
 *  extra `localId` for stable React keys across re-renders (the wire
 *  payload strips this field at submit time). */
interface InputRow {
  localId: string;
  name: string;
  description: string;
  required: boolean;
  type: 'string' | 'integer' | 'boolean';
}

const NAME_RE = /^[a-zA-Z][a-zA-Z0-9_-]{0,63}$/;
let nextLocalId = 0;
function newRow(): InputRow {
  nextLocalId += 1;
  return {
    localId: `row-${nextLocalId}`,
    name: '',
    description: '',
    required: true,
    type: 'string',
  };
}

const log = debug('skills:create-form');

export interface CreateSkillFormHandle {
  /** True iff name+description are present and no submit is in flight. */
  isValid: () => boolean;
  /** True while skillsApi.createSkill is in flight. */
  isSubmitting: () => boolean;
  /** Imperatively trigger submit. Resolves once the round-trip finishes. */
  submit: () => Promise<void>;
}

export interface CreateSkillFormProps {
  /**
   * The id assigned to the underlying `<form>` element. Wrappers that
   * render their submit button outside the form (modal footer / page
   * header) set `<button form={formId}>` to fire submit via this id.
   */
  formId: string;
  /** Called with the freshly-created skill on success. */
  onCreated: (skill: SkillSummary) => void;
  /**
   * Called whenever validity / submission state changes so the
   * wrapper can sync its submit button's disabled state without
   * needing to introspect via a ref every render.
   */
  onStateChange?: (state: { valid: boolean; submitting: boolean }) => void;
  /** If true, autofocus the first field on mount (modal default). */
  autoFocus?: boolean;
}

/**
 * Client-side slug preview — mirrors the Rust `slugify_skill_name`
 * heuristic (lowercase, ASCII alphanumerics + `-`, collapse repeats,
 * trim hyphens at the edges). The preview is advisory only; the Rust
 * side is authoritative when the skill is persisted.
 */
export function previewSlug(name: string): string {
  const lower = name.normalize('NFKD').toLowerCase();
  let out = '';
  let prevHyphen = false;
  for (const ch of lower) {
    if ((ch >= 'a' && ch <= 'z') || (ch >= '0' && ch <= '9')) {
      out += ch;
      prevHyphen = false;
      continue;
    }
    if ((ch === '-' || ch === '_' || /\s/.test(ch)) && !prevHyphen) {
      out += '-';
      prevHyphen = true;
    }
  }
  return out.replace(/^-+|-+$/g, '');
}

const CreateSkillForm = forwardRef<CreateSkillFormHandle, CreateSkillFormProps>(
  ({ formId, onCreated, onStateChange, autoFocus = false }, ref) => {
    const { t } = useT();
    const [name, setName] = useState('');
    const [description, setDescription] = useState('');
    // Scope is fixed to 'user' — the form previously exposed a radio
    // toggle for user/project plus license/author/tags/allowed-tools
    // fields. None of those were useful in practice and they cluttered
    // the create flow; user-scoped is the only sensible default for
    // dashboard-created skills. Project-scoped skills are still
    // creatable by editing the workspace skill files directly. The
    // backend payload still requires `scope` so we hold it as a const.
    const scope: SkillScope = 'user';
    const [submitting, setSubmitting] = useState(false);
    const [error, setError] = useState<string | null>(null);
    const [inputs, setInputs] = useState<InputRow[]>([]);

    const firstFieldRef = useRef<HTMLInputElement | null>(null);

    const slug = useMemo(() => previewSlug(name), [name]);

    const nameValid = slug.length > 0;
    const descriptionValid = description.trim().length > 0;
    // Each row must have a non-empty, regex-valid name. Empty rows block
    // submission so the user explicitly removes them rather than getting
    // a malformed [[inputs]] entry silently dropped on the Rust side.
    const inputsValid = inputs.every((r) => NAME_RE.test(r.name.trim()));
    const formValid = nameValid && descriptionValid && inputsValid && !submitting;

    const addRow = useCallback(() => {
      setInputs((cur) => [...cur, newRow()]);
    }, []);
    const removeRow = useCallback((localId: string) => {
      setInputs((cur) => cur.filter((r) => r.localId !== localId));
    }, []);
    const updateRow = useCallback((localId: string, patch: Partial<InputRow>) => {
      setInputs((cur) => cur.map((r) => (r.localId === localId ? { ...r, ...patch } : r)));
    }, []);

    // Surface state to the wrapper for its submit button's disabled prop.
    useEffect(() => {
      onStateChange?.({ valid: formValid, submitting });
    }, [formValid, submitting, onStateChange]);

    useEffect(() => {
      if (!autoFocus) return;
      const raf = window.requestAnimationFrame(() => {
        firstFieldRef.current?.focus();
      });
      return () => {
        window.cancelAnimationFrame(raf);
      };
    }, [autoFocus]);

    const submit = useCallback(async () => {
      if (!formValid) return;
      const payload: CreateSkillInput = {
        name: name.trim(),
        description: description.trim(),
        scope,
      };
      if (inputs.length > 0) {
        payload.inputs = inputs.map<CreateSkillInputDef>((r) => {
          const def: CreateSkillInputDef = {
            name: r.name.trim(),
            required: r.required,
          };
          const desc = r.description.trim();
          if (desc) def.description = desc;
          // Default 'string' on the Rust side, omit to keep payload tidy.
          if (r.type !== 'string') def.type = r.type;
          return def;
        });
      }

      log('submit name=%s scope=%s inputs=%d', payload.name, payload.scope, inputs.length);
      setSubmitting(true);
      setError(null);
      try {
        const created = await skillsApi.createSkill(payload);
        log('submit-ok id=%s', created.id);
        onCreated(created);
      } catch (err) {
        const message = err instanceof Error ? err.message : String(err);
        log('submit-err %s', message);
        setError(message);
      } finally {
        setSubmitting(false);
      }
    }, [description, formValid, inputs, name, onCreated]);

    useImperativeHandle(
      ref,
      () => ({
        isValid: () => formValid,
        isSubmitting: () => submitting,
        submit,
      }),
      [formValid, submitting, submit]
    );

    const handleFormSubmit = (e: React.FormEvent) => {
      e.preventDefault();
      void submit();
    };

    return (
      <form id={formId} onSubmit={handleFormSubmit} className="space-y-4">
        {/* Name */}
        <div>
          <label
            htmlFor="create-skill-name"
            className="block text-xs font-medium text-stone-600 dark:text-neutral-300"
          >
            {t('skills.create.name')}
            <span className="text-coral-500"> *</span>
          </label>
          <input
            id="create-skill-name"
            ref={firstFieldRef}
            type="text"
            value={name}
            onChange={(e) => setName(e.target.value)}
            required
            maxLength={128}
            className="mt-1 w-full rounded-lg border border-stone-200 dark:border-neutral-800 bg-white dark:bg-neutral-900 px-3 py-2 text-sm text-stone-900 dark:text-neutral-100 shadow-sm transition-colors focus:border-primary-500 focus:outline-none focus:ring-2 focus:ring-primary-500/30"
            placeholder={t('skills.create.namePlaceholder')}
          />
          <p className="mt-1 text-[11px] text-stone-500 dark:text-neutral-400">
            {t('skills.create.slugLabel')}{' '}
            <code className="rounded bg-stone-100 dark:bg-neutral-800 px-1 py-[1px] font-mono text-stone-700 dark:text-neutral-200">
              {slug || '—'}
            </code>
          </p>
        </div>

        {/* Description */}
        <div>
          <label
            htmlFor="create-skill-description"
            className="block text-xs font-medium text-stone-600 dark:text-neutral-300"
          >
            {t('skills.create.description')}
            <span className="text-coral-500"> *</span>
          </label>
          <textarea
            id="create-skill-description"
            value={description}
            onChange={(e) => setDescription(e.target.value)}
            required
            rows={3}
            maxLength={500}
            className="mt-1 w-full rounded-lg border border-stone-200 dark:border-neutral-800 bg-white dark:bg-neutral-900 px-3 py-2 text-sm text-stone-900 dark:text-neutral-100 shadow-sm transition-colors focus:border-primary-500 focus:outline-none focus:ring-2 focus:ring-primary-500/30"
            placeholder={t('skills.create.descriptionPlaceholder')}
          />
        </div>

        {/* Inputs (optional) — declare [[inputs]] for the generated
            skill.toml. The Skills Runner reads this to render dynamic
            form controls per input (text / number / checkbox). The
            section stays optional — formValid doesn't depend on
            non-empty rows — but every row that exists must have a
            valid, non-empty name (regex enforced) so the Rust side
            never receives a malformed [[inputs]] entry. */}
        <div>
          <div className="flex items-baseline justify-between">
            <label className="block text-xs font-medium text-stone-600 dark:text-neutral-300">
              {t('skills.create.inputs.heading')}
              <span className="ml-1 font-normal text-stone-400 dark:text-neutral-500">
                {t('skills.create.optional')}
              </span>
            </label>
            <button
              type="button"
              data-testid="create-skill-add-input"
              onClick={addRow}
              className="text-xs font-medium text-primary-600 hover:text-primary-700"
            >
              + {t('skills.create.inputs.add')}
            </button>
          </div>
          <p className="mt-0.5 text-[11px] text-stone-500 dark:text-neutral-400">
            {t('skills.create.inputs.help')}
          </p>
          {inputs.length > 0 && (
            <div className="mt-2 space-y-2">
              {inputs.map((row) => {
                const trimmed = row.name.trim();
                const showNameErr = row.name.length > 0 && !NAME_RE.test(trimmed);
                return (
                  <div
                    key={row.localId}
                    data-testid={`create-skill-input-row-${row.localId}`}
                    className="rounded-lg border border-stone-200 dark:border-neutral-800 bg-stone-50 dark:bg-neutral-950/40 p-3"
                  >
                    <div className="grid grid-cols-1 gap-2 sm:grid-cols-[1fr_1fr_auto]">
                      <div>
                        <input
                          type="text"
                          value={row.name}
                          onChange={(e) => updateRow(row.localId, { name: e.target.value })}
                          maxLength={64}
                          placeholder={t('skills.create.inputs.row.namePlaceholder')}
                          aria-label={t('skills.create.inputs.row.name')}
                          className={`w-full rounded-md border bg-white dark:bg-neutral-900 px-2 py-1.5 text-xs text-stone-900 dark:text-neutral-100 shadow-sm focus:outline-none focus:ring-2 focus:ring-primary-500/30 ${showNameErr ? 'border-coral-400' : 'border-stone-200 dark:border-neutral-800 focus:border-primary-500'}`}
                        />
                        {showNameErr && (
                          <p className="mt-0.5 text-[10px] text-coral-600">
                            {t('skills.create.inputs.row.nameError')}
                          </p>
                        )}
                      </div>
                      <input
                        type="text"
                        value={row.description}
                        onChange={(e) =>
                          updateRow(row.localId, { description: e.target.value })
                        }
                        maxLength={256}
                        placeholder={t('skills.create.inputs.row.descriptionPlaceholder')}
                        aria-label={t('skills.create.inputs.row.description')}
                        className="w-full rounded-md border border-stone-200 dark:border-neutral-800 bg-white dark:bg-neutral-900 px-2 py-1.5 text-xs text-stone-900 dark:text-neutral-100 shadow-sm focus:border-primary-500 focus:outline-none focus:ring-2 focus:ring-primary-500/30"
                      />
                      <button
                        type="button"
                        data-testid={`create-skill-remove-input-${row.localId}`}
                        onClick={() => removeRow(row.localId)}
                        aria-label={t('skills.create.inputs.row.remove')}
                        className="self-center rounded-md px-2 py-1.5 text-xs text-stone-500 hover:bg-coral-100 hover:text-coral-700"
                      >
                        🗑
                      </button>
                    </div>
                    <div className="mt-2 flex items-center gap-3 text-[11px]">
                      <label className="flex items-center gap-1">
                        <span className="text-stone-500 dark:text-neutral-400">
                          {t('skills.create.inputs.row.type')}:
                        </span>
                        <select
                          value={row.type}
                          onChange={(e) =>
                            updateRow(row.localId, {
                              type: e.target.value as InputRow['type'],
                            })
                          }
                          aria-label={t('skills.create.inputs.row.type')}
                          className="rounded border border-stone-200 dark:border-neutral-800 bg-white dark:bg-neutral-900 px-1 py-0.5 text-[11px] text-stone-900 dark:text-neutral-100"
                        >
                          <option value="string">{t('skills.create.inputs.type.string')}</option>
                          <option value="integer">
                            {t('skills.create.inputs.type.integer')}
                          </option>
                          <option value="boolean">
                            {t('skills.create.inputs.type.boolean')}
                          </option>
                        </select>
                      </label>
                      <label className="flex items-center gap-1">
                        <input
                          type="checkbox"
                          checked={row.required}
                          onChange={(e) =>
                            updateRow(row.localId, { required: e.target.checked })
                          }
                          className="h-3 w-3 accent-primary-500"
                        />
                        <span className="text-stone-500 dark:text-neutral-400">
                          {t('skills.create.inputs.row.required')}
                        </span>
                      </label>
                    </div>
                  </div>
                );
              })}
            </div>
          )}
        </div>

        {/* Error */}
        {error ? (
          <div
            role="alert"
            className="rounded-xl border border-coral-200 bg-coral-50 p-3 text-xs text-coral-900"
          >
            <p className="font-semibold">{t('skills.create.createError')}</p>
            <p className="mt-1 whitespace-pre-wrap font-mono">{error}</p>
          </div>
        ) : null}
      </form>
    );
  }
);

export default CreateSkillForm;
