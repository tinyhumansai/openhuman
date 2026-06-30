/**
 * GoalsPanel — Brain > Goals.
 *
 * Views and edits the agent's long-term goals list (`openhuman.memory_goals_*`)
 * and triggers the turn-based enrichment agent ("Reflect"). The same list is
 * curated automatically by the background goals agent when context is
 * summarized; this panel is the manual surface.
 */
import { useCallback, useEffect, useRef, useState } from 'react';
import { LuCheck, LuPencil, LuPlus, LuSparkles, LuTrash2, LuX } from 'react-icons/lu';

import { useT } from '../../lib/i18n/I18nContext';
import { type GoalItem, goalsApi } from '../../services/api/goalsApi';
import Button from '../ui/Button';
import Input from '../ui/Input';

const cardClass = 'rounded-lg border border-line bg-surface p-4';

export default function GoalsPanel() {
  const { t } = useT();
  const [goals, setGoals] = useState<GoalItem[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [actionError, setActionError] = useState<string | null>(null);

  const [newText, setNewText] = useState('');
  const [adding, setAdding] = useState(false);
  const [reflecting, setReflecting] = useState(false);
  const [reflectSummary, setReflectSummary] = useState<string | null>(null);

  const [editingId, setEditingId] = useState<string | null>(null);
  const [editText, setEditText] = useState('');
  const [busyId, setBusyId] = useState<string | null>(null);

  const mountedRef = useRef(true);

  const load = useCallback(async () => {
    setError(null);
    try {
      const list = await goalsApi.list();
      if (mountedRef.current) setGoals(list);
    } catch (err) {
      if (mountedRef.current) setError(err instanceof Error ? err.message : String(err));
    } finally {
      if (mountedRef.current) setLoading(false);
    }
  }, []);

  useEffect(() => {
    mountedRef.current = true;
    void load();
    return () => {
      mountedRef.current = false;
    };
  }, [load]);

  const handleAdd = useCallback(async () => {
    const text = newText.trim();
    if (!text) return;
    setActionError(null);
    setAdding(true);
    try {
      const list = await goalsApi.add(text);
      if (mountedRef.current) {
        setError(null);
        setGoals(list);
        setNewText('');
      }
    } catch (err) {
      if (mountedRef.current)
        setActionError(err instanceof Error ? err.message : t('brain.goals.actionError'));
    } finally {
      if (mountedRef.current) setAdding(false);
    }
  }, [newText, t]);

  const startEdit = useCallback((goal: GoalItem) => {
    setActionError(null);
    setEditingId(goal.id);
    setEditText(goal.text);
  }, []);

  const cancelEdit = useCallback(() => {
    setEditingId(null);
    setEditText('');
  }, []);

  const saveEdit = useCallback(
    async (id: string) => {
      const text = editText.trim();
      if (!text) return;
      setActionError(null);
      setBusyId(id);
      try {
        const list = await goalsApi.edit(id, text);
        if (mountedRef.current) {
          setError(null);
          setGoals(list);
          setEditingId(null);
          setEditText('');
        }
      } catch (err) {
        if (mountedRef.current)
          setActionError(err instanceof Error ? err.message : t('brain.goals.actionError'));
      } finally {
        if (mountedRef.current) setBusyId(null);
      }
    },
    [editText, t]
  );

  const handleDelete = useCallback(
    async (id: string) => {
      setActionError(null);
      setBusyId(id);
      try {
        const list = await goalsApi.remove(id);
        if (mountedRef.current) {
          setError(null);
          setGoals(list);
        }
      } catch (err) {
        if (mountedRef.current)
          setActionError(err instanceof Error ? err.message : t('brain.goals.actionError'));
      } finally {
        if (mountedRef.current) setBusyId(null);
      }
    },
    [t]
  );

  const handleReflect = useCallback(async () => {
    setActionError(null);
    setReflectSummary(null);
    setReflecting(true);
    try {
      const res = await goalsApi.reflect();
      if (mountedRef.current) {
        setError(null);
        setGoals(res.items);
        setReflectSummary(
          res.ran
            ? res.summary || t('brain.goals.reflectDone')
            : res.summary || t('brain.goals.actionError')
        );
      }
    } catch (err) {
      if (mountedRef.current)
        setActionError(err instanceof Error ? err.message : t('brain.goals.actionError'));
    } finally {
      if (mountedRef.current) setReflecting(false);
    }
  }, [t]);

  return (
    <div className="space-y-3 animate-fade-up">
      <div className={cardClass}>
        {/* Header */}
        <div className="flex items-start justify-between gap-3">
          <div className="min-w-0">
            <h2 className="text-sm font-semibold text-content">{t('brain.goals.title')}</h2>
            <p className="mt-0.5 text-xs text-content-muted">{t('brain.goals.description')}</p>
          </div>
          <Button
            type="button"
            variant="secondary"
            size="sm"
            onClick={handleReflect}
            disabled={reflecting}>
            <LuSparkles className="mr-1.5 h-3.5 w-3.5" />
            {reflecting ? t('brain.goals.reflecting') : t('brain.goals.reflect')}
          </Button>
        </div>

        {/* Add row */}
        <div className="mt-4 flex items-center gap-2">
          <Input
            inputSize="sm"
            value={newText}
            placeholder={t('brain.goals.addPlaceholder')}
            onChange={e => setNewText(e.target.value)}
            onKeyDown={e => {
              if (e.key === 'Enter') void handleAdd();
            }}
          />
          <Button
            type="button"
            variant="primary"
            size="sm"
            onClick={handleAdd}
            disabled={adding || !newText.trim()}>
            <LuPlus className="mr-1 h-3.5 w-3.5" />
            {t('brain.goals.add')}
          </Button>
        </div>

        {/* Errors / reflect summary */}
        {actionError && (
          <div
            className="mt-3 rounded-lg border border-coral-200 bg-coral-50 px-3 py-2 text-xs text-coral-700 dark:border-coral-500/30 dark:bg-coral-500/10 dark:text-coral-300"
            role="alert">
            {actionError}
          </div>
        )}
        {reflectSummary && (
          <div className="mt-3 whitespace-pre-wrap rounded-lg border border-sage-200 bg-sage-50 px-3 py-2 text-xs text-sage-800 dark:border-sage-500/30 dark:bg-sage-500/10 dark:text-sage-200">
            {reflectSummary}
          </div>
        )}

        {/* List */}
        <div className="mt-4">
          {loading ? (
            <div className="flex items-center justify-center py-8 text-content-faint">
              <div className="mr-2 h-4 w-4 animate-spin rounded-full border-2 border-ocean-500 border-t-transparent" />
              <span className="text-sm">{t('common.loading')}</span>
            </div>
          ) : error ? (
            <div className="rounded-lg border border-coral-200 bg-coral-50 px-3 py-2 text-sm text-coral-700 dark:border-coral-500/30 dark:bg-coral-500/10 dark:text-coral-300">
              {error}
            </div>
          ) : goals.length === 0 ? (
            <p className="py-6 text-center text-sm text-content-faint">{t('brain.goals.empty')}</p>
          ) : (
            <ul className="divide-y divide-line overflow-hidden rounded-xl border border-line dark:divide-neutral-800">
              {goals.map(goal => (
                <li key={goal.id} className="bg-surface px-3 py-2.5">
                  {editingId === goal.id ? (
                    <div className="flex items-center gap-2">
                      <Input
                        inputSize="sm"
                        value={editText}
                        onChange={e => setEditText(e.target.value)}
                        onKeyDown={e => {
                          if (e.key === 'Enter') void saveEdit(goal.id);
                          if (e.key === 'Escape') cancelEdit();
                        }}
                        autoFocus
                      />
                      <Button
                        type="button"
                        variant="tertiary"
                        size="xs"
                        onClick={() => void saveEdit(goal.id)}
                        disabled={busyId === goal.id || !editText.trim()}
                        aria-label={t('common.save')}>
                        <LuCheck className="h-3.5 w-3.5" />
                      </Button>
                      <Button
                        type="button"
                        variant="tertiary"
                        size="xs"
                        onClick={cancelEdit}
                        aria-label={t('common.cancel')}>
                        <LuX className="h-3.5 w-3.5" />
                      </Button>
                    </div>
                  ) : (
                    <div className="flex items-center justify-between gap-3">
                      <span className="min-w-0 flex-1 text-sm text-content-secondary">
                        {goal.text}
                      </span>
                      <div className="flex shrink-0 items-center gap-1">
                        <Button
                          type="button"
                          variant="tertiary"
                          size="xs"
                          onClick={() => startEdit(goal)}
                          disabled={busyId === goal.id}
                          aria-label={t('brain.goals.editGoal')}>
                          <LuPencil className="h-3.5 w-3.5" />
                        </Button>
                        <Button
                          type="button"
                          variant="secondary"
                          tone="danger"
                          size="xs"
                          onClick={() => void handleDelete(goal.id)}
                          disabled={busyId === goal.id}
                          aria-label={t('brain.goals.deleteGoal')}>
                          <LuTrash2 className="h-3.5 w-3.5" />
                        </Button>
                      </div>
                    </div>
                  )}
                </li>
              ))}
            </ul>
          )}
        </div>
      </div>
    </div>
  );
}
