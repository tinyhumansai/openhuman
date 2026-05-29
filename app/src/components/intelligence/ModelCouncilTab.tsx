/**
 * Model Council tab — ask one question, get independent answers from several
 * models in parallel, then a chair model's synthesis of where they agree,
 * disagree, and what unique insight each added.
 *
 * The orchestration (parallel member calls + chair synthesis) lives in the
 * Rust core behind `openhuman.model_council_run`; this tab is the compose +
 * compare surface. Model ids are entered as free text because the available
 * set is provider-specific (local Ollama + any configured cloud providers)
 * and the council accepts arbitrary ids.
 */
import { useCallback, useMemo, useState } from 'react';

import { useT } from '../../lib/i18n/I18nContext';
import { modelCouncilApi, type ModelCouncilResult } from '../../services/api/modelCouncilApi';

/** Matches the server-side MAX_COUNCIL_MEMBERS cap. */
const MAX_MEMBERS = 5;

/** A member row carries a stable id so React keys survive mid-list removal. */
interface MemberRow {
  id: number;
  value: string;
}

/** Next id = max existing + 1: unique among current rows, no ref/StrictMode hazard. */
const nextMemberId = (rows: MemberRow[]): number =>
  rows.reduce((max, r) => Math.max(max, r.id), -1) + 1;

const ModelCouncilTab = () => {
  const { t } = useT();
  const [question, setQuestion] = useState('');
  const [members, setMembers] = useState<MemberRow[]>([
    { id: 0, value: '' },
    { id: 1, value: '' },
  ]);
  const [chair, setChair] = useState('');
  const [running, setRunning] = useState(false);
  const [result, setResult] = useState<ModelCouncilResult | null>(null);
  const [error, setError] = useState<string | null>(null);

  const filledMembers = useMemo(
    () => members.map(m => m.value.trim()).filter(v => v.length > 0),
    [members]
  );

  const canRun =
    !running && question.trim().length > 0 && filledMembers.length > 0 && chair.trim().length > 0;

  const updateMember = useCallback((id: number, value: string) => {
    setMembers(prev => prev.map(m => (m.id === id ? { ...m, value } : m)));
  }, []);

  const addMember = useCallback(() => {
    setMembers(prev =>
      prev.length >= MAX_MEMBERS ? prev : [...prev, { id: nextMemberId(prev), value: '' }]
    );
  }, []);

  const removeMember = useCallback((id: number) => {
    setMembers(prev => (prev.length <= 1 ? prev : prev.filter(m => m.id !== id)));
  }, []);

  const handleRun = useCallback(async () => {
    if (running) return;
    const trimmedMembers = members.map(m => m.value.trim()).filter(v => v.length > 0);
    if (question.trim().length === 0 || trimmedMembers.length === 0 || chair.trim().length === 0) {
      return;
    }
    setRunning(true);
    setError(null);
    setResult(null);
    try {
      const res = await modelCouncilApi.runCouncil({
        question: question.trim(),
        member_models: trimmedMembers,
        chair_model: chair.trim(),
      });
      setResult(res);
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setRunning(false);
    }
  }, [running, members, question, chair]);

  return (
    <div className="space-y-4">
      <div
        role="note"
        className="rounded-lg border border-primary-200 dark:border-primary-500/30 bg-primary-50 dark:bg-primary-500/10 px-3 py-2 text-xs text-stone-700 dark:text-neutral-200">
        <p className="font-medium mb-1">{t('modelCouncil.title')}</p>
        <p>{t('modelCouncil.intro')}</p>
      </div>

      {/* Question */}
      <div className="space-y-1.5">
        <label
          htmlFor="model-council-question"
          className="text-xs font-medium text-stone-600 dark:text-neutral-300">
          {t('modelCouncil.questionLabel')}
        </label>
        <textarea
          id="model-council-question"
          value={question}
          onChange={e => setQuestion(e.target.value)}
          rows={3}
          placeholder={t('modelCouncil.questionPlaceholder')}
          aria-label={t('modelCouncil.questionLabel')}
          className="w-full rounded-lg border border-stone-200 dark:border-neutral-700 bg-white dark:bg-neutral-900 px-3 py-2 text-sm text-stone-800 dark:text-neutral-100 focus:outline-none focus:ring-2 focus:ring-primary-400 resize-y"
        />
      </div>

      {/* Member models */}
      <div className="space-y-1.5">
        <div className="flex items-center justify-between">
          <span className="text-xs font-medium text-stone-600 dark:text-neutral-300">
            {t('modelCouncil.membersLabel')}
          </span>
          <span className="text-[10px] text-stone-400 dark:text-neutral-500">
            {t('modelCouncil.maxMembersNote').replace('{max}', String(MAX_MEMBERS))}
          </span>
        </div>
        <ul className="space-y-1.5">
          {members.map((member, index) => (
            <li key={member.id} className="flex items-center gap-2">
              <input
                type="text"
                value={member.value}
                onChange={e => updateMember(member.id, e.target.value)}
                placeholder={t('modelCouncil.memberPlaceholder')}
                aria-label={t('modelCouncil.memberAria').replace('{n}', String(index + 1))}
                className="flex-1 rounded-lg border border-stone-200 dark:border-neutral-700 bg-white dark:bg-neutral-900 px-3 py-1.5 text-sm font-mono text-stone-800 dark:text-neutral-100 focus:outline-none focus:ring-2 focus:ring-primary-400"
              />
              <button
                type="button"
                onClick={() => removeMember(member.id)}
                disabled={members.length <= 1}
                aria-label={t('modelCouncil.removeMemberAria').replace('{n}', String(index + 1))}
                className="shrink-0 rounded-lg border border-stone-200 dark:border-neutral-700 px-2 py-1.5 text-xs text-stone-500 dark:text-neutral-400 hover:bg-stone-50 dark:hover:bg-neutral-800 disabled:opacity-40 disabled:cursor-not-allowed">
                ✕
              </button>
            </li>
          ))}
        </ul>
        <button
          type="button"
          onClick={addMember}
          disabled={members.length >= MAX_MEMBERS}
          className="rounded-lg border border-stone-200 dark:border-neutral-700 px-3 py-1 text-xs font-medium text-stone-700 dark:text-neutral-200 hover:bg-stone-50 dark:hover:bg-neutral-800 disabled:opacity-40 disabled:cursor-not-allowed">
          {t('modelCouncil.addMember')}
        </button>
      </div>

      {/* Chair model */}
      <div className="space-y-1.5">
        <label
          htmlFor="model-council-chair"
          className="text-xs font-medium text-stone-600 dark:text-neutral-300">
          {t('modelCouncil.chairLabel')}
        </label>
        <input
          id="model-council-chair"
          type="text"
          value={chair}
          onChange={e => setChair(e.target.value)}
          placeholder={t('modelCouncil.chairPlaceholder')}
          aria-label={t('modelCouncil.chairLabel')}
          className="w-full rounded-lg border border-stone-200 dark:border-neutral-700 bg-white dark:bg-neutral-900 px-3 py-1.5 text-sm font-mono text-stone-800 dark:text-neutral-100 focus:outline-none focus:ring-2 focus:ring-primary-400"
        />
        <p className="text-[10px] text-stone-400 dark:text-neutral-500">
          {t('modelCouncil.chairHelp')}
        </p>
      </div>

      <div className="flex items-center gap-2">
        <button
          type="button"
          onClick={() => void handleRun()}
          disabled={!canRun}
          className="rounded-lg bg-primary-500 px-4 py-1.5 text-sm font-semibold text-white hover:bg-primary-600 disabled:opacity-50 disabled:cursor-not-allowed">
          {running ? t('modelCouncil.running') : t('modelCouncil.run')}
        </button>
        {running && (
          <span
            role="status"
            aria-live="polite"
            className="text-xs text-stone-500 dark:text-neutral-400">
            {t('modelCouncil.runningHint')}
          </span>
        )}
      </div>

      {error && (
        <p role="alert" className="text-xs text-coral-700 dark:text-coral-300">
          {t('modelCouncil.errorPrefix')} {error}
        </p>
      )}

      {result && (
        <section aria-labelledby="model-council-results-heading" className="space-y-3 pt-1">
          <h3
            id="model-council-results-heading"
            className="text-sm font-semibold text-stone-800 dark:text-neutral-100">
            {t('modelCouncil.resultsHeading')}
          </h3>

          {/* Member answers, side by side */}
          <div className="grid gap-2 sm:grid-cols-2">
            {result.members.map((member, index) => (
              <div
                key={`${member.model}-${index}`}
                className="rounded-lg border border-stone-200 dark:border-neutral-800 p-3 space-y-1.5">
                <div className="flex items-center justify-between gap-2">
                  <span className="text-xs font-mono font-medium text-stone-700 dark:text-neutral-200 truncate">
                    {member.model}
                  </span>
                  <span
                    className={`inline-flex items-center px-1.5 py-0.5 rounded text-[9px] font-semibold uppercase tracking-wider shrink-0 ${
                      member.error
                        ? 'bg-coral-100 dark:bg-coral-500/20 text-coral-700 dark:text-coral-300'
                        : 'bg-sage-100 dark:bg-sage-500/20 text-sage-700 dark:text-sage-300'
                    }`}>
                    {member.error
                      ? t('modelCouncil.memberFailed')
                      : t('modelCouncil.memberAnswered')}
                  </span>
                </div>
                {member.error ? (
                  <p className="text-xs text-coral-600 dark:text-coral-400">{member.error}</p>
                ) : (
                  <p className="text-xs text-stone-600 dark:text-neutral-300 whitespace-pre-wrap break-words">
                    {member.response}
                  </p>
                )}
              </div>
            ))}
          </div>

          {/* Chair synthesis */}
          <div className="rounded-lg border border-primary-200 dark:border-primary-500/30 bg-primary-50 dark:bg-primary-500/10 p-3 space-y-1">
            <div className="flex items-center justify-between gap-2">
              <h4 className="text-xs font-semibold text-stone-800 dark:text-neutral-100">
                {t('modelCouncil.synthesisHeading')}
              </h4>
              <span className="text-[10px] font-mono text-stone-500 dark:text-neutral-400 truncate">
                {t('modelCouncil.synthesisBy').replace('{model}', result.chair_model)}
              </span>
            </div>
            <p className="text-sm text-stone-700 dark:text-neutral-200 whitespace-pre-wrap break-words">
              {result.synthesis}
            </p>
          </div>
        </section>
      )}
    </div>
  );
};

export default ModelCouncilTab;
