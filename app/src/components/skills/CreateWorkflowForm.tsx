/**
 * CreateWorkflowForm
 * ----------------
 *
 * Body of the "create a new SKILL.md" flow, shared between
 * `CreateSkillModal` (modal chrome) and the `/skills/new` page wrapper.
 *
 * Owns:
 *   - All form fields (name, description, scope, license, author,
 *     tags, allowed-tools).
 *   - Slug preview + validation (name and description required).
 *   - Submit handler that calls `workflowsApi.createWorkflow` and surfaces
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
 * handle exposed via a ref (`<CreateWorkflowForm ref={ref} ... />` →
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
  type CreateWorkflowInput,
  type CreateWorkflowInputDef,
  workflowsApi,
  type WorkflowScope,
  type WorkflowSummary,
} from '../../services/api/workflowsApi';
import Button from '../ui/Button';

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
    // Additional inputs are optional by default — the author opts a row INTO
    // required via the checkbox. (A workflow run only blocks on inputs the
    // author explicitly marks required.)
    required: false,
    type: 'string',
  };
}

const log = debug('skills:create-form');

export interface CreateSkillFormHandle {
  /** True iff name+description are present and no submit is in flight. */
  isValid: () => boolean;
  /** True while workflowsApi.createWorkflow is in flight. */
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
  onCreated: (skill: WorkflowSummary) => void;
  /**
   * Called whenever validity / submission state changes so the
   * wrapper can sync its submit button's disabled state without
   * needing to introspect via a ref every render.
   */
  onStateChange?: (state: { valid: boolean; submitting: boolean }) => void;
  /** If true, autofocus the first field on mount (modal default). */
  autoFocus?: boolean;
  /**
   * When set, the form is in EDIT mode for this workflow: fields are
   * pre-filled (name read-only — the slug is identity), and submit calls
   * `workflows_update` instead of `workflows_create`. Tags / author /
   * allowed-tools (not exposed as editable fields) are carried through so
   * they're preserved on save.
   */
  editing?: WorkflowSummary;
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

const CreateWorkflowForm = forwardRef<CreateSkillFormHandle, CreateSkillFormProps>(
  ({ formId, onCreated, onStateChange, autoFocus = false, editing }, ref) => {
    const { t } = useT();
    const isEdit = !!editing;
    // Fields the form doesn't expose but must preserve across an edit
    // (otherwise workflows_update would regenerate frontmatter without them).
    const preservedRef = useRef<{ tags?: string[]; author?: string; allowedTools?: string[] }>({});
    const [name, setName] = useState('');
    const [description, setDescription] = useState('');
    // The workflow half of the unified form: *when* an agent should reach for
    // this workflow. Optional — falls back to the description on the backend.
    const [whenToUse, setWhenToUse] = useState('');
    // Scope is fixed to 'user' — the form previously exposed a radio
    // toggle for user/project plus license/author/tags/allowed-tools
    // fields. None of those were useful in practice and they cluttered
    // the create flow; user-scoped is the only sensible default for
    // dashboard-created skills. Project-scoped skills are still
    // creatable by editing the workspace skill files directly. The
    // backend payload still requires `scope` so we hold it as a const.
    const scope: WorkflowScope = 'user';
    const [submitting, setSubmitting] = useState(false);
    const [error, setError] = useState<string | null>(null);
    const [inputs, setInputs] = useState<InputRow[]>([]);

    const firstFieldRef = useRef<HTMLInputElement | null>(null);

    const slug = useMemo(() => previewSlug(name), [name]);

    const nameValid = slug.length > 0;
    const descriptionValid = description.trim().length > 0;
    // Each row must have a non-empty, regex-valid name AND a non-empty
    // description. Empty/under-specified rows block submission so the user
    // explicitly removes them rather than shipping a malformed or
    // undocumented [[inputs]] entry. The description is what the agent (and
    // the run form) shows to explain the input, so it's mandatory per row.
    const inputsValid = inputs.every(
      r => NAME_RE.test(r.name.trim()) && r.description.trim().length > 0
    );
    const formValid = nameValid && descriptionValid && inputsValid && !submitting;

    const addRow = useCallback(() => {
      setInputs(cur => [...cur, newRow()]);
    }, []);
    const removeRow = useCallback((localId: string) => {
      setInputs(cur => cur.filter(r => r.localId !== localId));
    }, []);
    const updateRow = useCallback((localId: string, patch: Partial<InputRow>) => {
      setInputs(cur => cur.map(r => (r.localId === localId ? { ...r, ...patch } : r)));
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

    // Edit mode: pre-fill from the existing workflow. Name + description come
    // from the summary; when_to_use + declared inputs come from describe.
    // tags / author / allowed-tools are stashed for pass-through on save.
    useEffect(() => {
      if (!editing) return;
      setName(editing.name);
      setDescription(editing.description);
      preservedRef.current = {
        tags: editing.tags,
        author: editing.author ?? undefined,
        allowedTools: editing.tools,
      };
      let cancelled = false;
      void (async () => {
        try {
          const desc = await workflowsApi.describeWorkflow(editing.id);
          if (cancelled) return;
          if (desc.when_to_use) setWhenToUse(desc.when_to_use);
          setInputs(
            desc.inputs.map(i => {
              nextLocalId += 1;
              return {
                localId: `row-${nextLocalId}`,
                name: i.name,
                description: i.description ?? '',
                required: i.required,
                type: (i.type as InputRow['type']) ?? 'string',
              };
            })
          );
        } catch {
          // Pre-fill is best-effort; the user can still edit the visible fields.
        }
      })();
      return () => {
        cancelled = true;
      };
    }, [editing]);

    const submit = useCallback(async () => {
      if (!formValid) return;
      const payload: CreateWorkflowInput = {
        name: name.trim(),
        description: description.trim(),
        scope,
      };
      if (whenToUse.trim()) {
        payload.whenToUse = whenToUse.trim();
      }
      if (editing) {
        // Preserve fields the form doesn't expose as editable.
        if (preservedRef.current.tags && preservedRef.current.tags.length > 0) {
          payload.tags = preservedRef.current.tags;
        }
        if (preservedRef.current.author) {
          payload.author = preservedRef.current.author;
        }
        if (preservedRef.current.allowedTools && preservedRef.current.allowedTools.length > 0) {
          payload.allowedTools = preservedRef.current.allowedTools;
        }
      }
      if (inputs.length > 0) {
        payload.inputs = inputs.map<CreateWorkflowInputDef>(r => {
          const def: CreateWorkflowInputDef = { name: r.name.trim(), required: r.required };
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
        const saved = editing
          ? await workflowsApi.updateWorkflow(payload)
          : await workflowsApi.createWorkflow(payload);
        log('submit-ok id=%s edit=%s', saved.id, isEdit);
        onCreated(saved);
      } catch (err) {
        const message = err instanceof Error ? err.message : String(err);
        log('submit-err %s', message);
        setError(message);
      } finally {
        setSubmitting(false);
      }
    }, [description, formValid, inputs, name, whenToUse, onCreated, editing, isEdit]);

    useImperativeHandle(
      ref,
      () => ({ isValid: () => formValid, isSubmitting: () => submitting, submit }),
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
            className="block text-xs font-medium text-content-secondary">
            {t('skills.create.name')}
            <span className="text-coral-500"> *</span>
          </label>
          <input
            id="create-skill-name"
            ref={firstFieldRef}
            type="text"
            value={name}
            onChange={e => setName(e.target.value)}
            required
            readOnly={isEdit}
            maxLength={128}
            className={`mt-1 w-full rounded-lg border border-line px-3 py-2 text-sm text-content shadow-sm transition-colors focus:border-primary-500 focus:outline-none focus:ring-2 focus:ring-primary-500/30 ${
              isEdit
                ? 'cursor-not-allowed bg-surface-muted'
                : 'bg-surface'
            }`}
            placeholder={t('skills.create.namePlaceholder')}
          />
          <p className="mt-1 text-[11px] text-content-muted">
            {t('skills.create.slugLabel')}{' '}
            <code className="rounded bg-surface-subtle px-1 py-[1px] font-mono text-content-secondary">
              {slug || '—'}
            </code>
          </p>
        </div>

        {/* Description */}
        <div>
          <label
            htmlFor="create-skill-description"
            className="block text-xs font-medium text-content-secondary">
            {t('skills.create.description')}
            <span className="text-coral-500"> *</span>
          </label>
          <textarea
            id="create-skill-description"
            value={description}
            onChange={e => setDescription(e.target.value)}
            required
            rows={3}
            maxLength={500}
            className="mt-1 w-full rounded-lg border border-line bg-surface px-3 py-2 text-sm text-content shadow-sm transition-colors focus:border-primary-500 focus:outline-none focus:ring-2 focus:ring-primary-500/30"
            placeholder={t('skills.create.descriptionPlaceholder')}
          />
        </div>

        {/* When to use — the workflow's trigger/goal. Optional; the backend
            falls back to the description. This is what makes a workflow more
            than a bare procedure: it tells the agent WHEN to run it. */}
        <div>
          <label
            htmlFor="create-skill-when-to-use"
            className="block text-xs font-medium text-content-secondary">
            {t('skills.create.whenToUse')}
            <span className="ml-1 font-normal text-content-faint">
              {t('skills.create.optional')}
            </span>
          </label>
          <textarea
            id="create-skill-when-to-use"
            value={whenToUse}
            onChange={e => setWhenToUse(e.target.value)}
            rows={2}
            maxLength={500}
            className="mt-1 w-full rounded-lg border border-line bg-surface px-3 py-2 text-sm text-content shadow-sm transition-colors focus:border-primary-500 focus:outline-none focus:ring-2 focus:ring-primary-500/30"
            placeholder={t('skills.create.whenToUsePlaceholder')}
          />
          <p className="mt-1 text-[11px] text-content-muted">
            {t('skills.create.whenToUseHelp')}
          </p>
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
            <label className="block text-xs font-medium text-content-secondary">
              {t('skills.create.inputs.heading')}
              <span className="ml-1 font-normal text-content-faint">
                {t('skills.create.optional')}
              </span>
            </label>
            <Button
              variant="tertiary"
              size="xs"
              data-testid="create-skill-add-input"
              onClick={addRow}
              className="px-0 text-primary-600 hover:bg-transparent hover:text-primary-700">
              + {t('skills.create.inputs.add')}
            </Button>
          </div>
          <p className="mt-0.5 text-[11px] text-content-muted">
            {t('skills.create.inputs.help')}
          </p>
          {inputs.length > 0 && (
            <div className="mt-2 space-y-2">
              {inputs.map(row => {
                const trimmed = row.name.trim();
                const showNameErr = row.name.length > 0 && !NAME_RE.test(trimmed);
                // A row is "in use" once it has a name; at that point its
                // description is required (see `inputsValid`).
                const showDescErr =
                  row.name.trim().length > 0 && row.description.trim().length === 0;
                return (
                  <div
                    key={row.localId}
                    data-testid={`create-skill-input-row-${row.localId}`}
                    className="rounded-lg border border-line bg-surface-muted dark:bg-surface-canvas/40 p-3">
                    <div className="grid grid-cols-1 gap-2 sm:grid-cols-[1fr_1fr_auto]">
                      <div>
                        <input
                          type="text"
                          value={row.name}
                          onChange={e => updateRow(row.localId, { name: e.target.value })}
                          maxLength={64}
                          placeholder={t('skills.create.inputs.row.namePlaceholder')}
                          aria-label={t('skills.create.inputs.row.name')}
                          className={`w-full rounded-md border bg-surface px-2 py-1.5 text-xs text-content shadow-sm focus:outline-none focus:ring-2 focus:ring-primary-500/30 ${showNameErr ? 'border-coral-400' : 'border-line focus:border-primary-500'}`}
                        />
                        {showNameErr && (
                          <p className="mt-0.5 text-[10px] text-coral-600">
                            {t('skills.create.inputs.row.nameError')}
                          </p>
                        )}
                      </div>
                      <div>
                        <input
                          type="text"
                          value={row.description}
                          onChange={e => updateRow(row.localId, { description: e.target.value })}
                          maxLength={256}
                          placeholder={t('skills.create.inputs.row.descriptionPlaceholder')}
                          aria-label={t('skills.create.inputs.row.description')}
                          className={`w-full rounded-md border bg-surface px-2 py-1.5 text-xs text-content shadow-sm focus:outline-none focus:ring-2 focus:ring-primary-500/30 ${showDescErr ? 'border-coral-400' : 'border-line focus:border-primary-500'}`}
                        />
                        {showDescErr && (
                          <p className="mt-0.5 text-[10px] text-coral-600">
                            {t('skills.create.inputs.row.descriptionError')}
                          </p>
                        )}
                      </div>
                      <Button
                        iconOnly
                        variant="tertiary"
                        tone="danger"
                        size="sm"
                        data-testid={`create-skill-remove-input-${row.localId}`}
                        onClick={() => removeRow(row.localId)}
                        aria-label={t('skills.create.inputs.row.remove')}
                        className="self-center">
                        🗑
                      </Button>
                    </div>
                    <div className="mt-2 flex items-center gap-3 text-[11px]">
                      <label className="flex items-center gap-1">
                        <span className="text-content-muted">
                          {t('skills.create.inputs.row.type')}:
                        </span>
                        <select
                          value={row.type}
                          onChange={e =>
                            updateRow(row.localId, { type: e.target.value as InputRow['type'] })
                          }
                          aria-label={t('skills.create.inputs.row.type')}
                          className="rounded border border-line bg-surface px-1 py-0.5 text-[11px] text-content">
                          <option value="string">{t('skills.create.inputs.type.string')}</option>
                          <option value="integer">{t('skills.create.inputs.type.integer')}</option>
                          <option value="boolean">{t('skills.create.inputs.type.boolean')}</option>
                        </select>
                      </label>
                      <label className="flex items-center gap-1">
                        <input
                          type="checkbox"
                          checked={row.required}
                          onChange={e => updateRow(row.localId, { required: e.target.checked })}
                          className="h-3 w-3 accent-primary-500"
                        />
                        <span className="text-content-muted">
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
            className="rounded-xl border border-coral-200 bg-coral-50 p-3 text-xs text-coral-900">
            <p className="font-semibold">{t('workflows.create.createError')}</p>
            <p className="mt-1 whitespace-pre-wrap font-mono">{error}</p>
          </div>
        ) : null}
      </form>
    );
  }
);

export default CreateWorkflowForm;
