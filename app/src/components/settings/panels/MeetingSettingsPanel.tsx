import debug from 'debug';
import { useEffect, useRef, useState } from 'react';

import { useT } from '../../../lib/i18n/I18nContext';
import {
  isTauri,
  type MeetAutoJoinPolicy,
  type MeetAutoSummarizePolicy,
  openhumanGetMeetSettings,
  openhumanUpdateMeetSettings,
} from '../../../utils/tauriCommands';
import PanelPage from '../../layout/PanelPage';
import SettingsBackButton from '../components/SettingsBackButton';
import {
  SettingsRow,
  SettingsSection,
  SettingsSelect,
  SettingsStatusLine,
  SettingsSwitch,
} from '../controls';
import { useSettingsNavigation } from '../hooks/useSettingsNavigation';

const log = debug('settings:meetings');

const AUTO_JOIN_OPTIONS: MeetAutoJoinPolicy[] = ['ask_each_time', 'always', 'never'];
const AUTO_SUMMARIZE_OPTIONS: MeetAutoSummarizePolicy[] = ['ask', 'always', 'never'];

const AUTO_JOIN_LABEL_KEY: Record<MeetAutoJoinPolicy, string> = {
  ask_each_time: 'settings.meetings.autoJoin.askEachTime',
  always: 'settings.meetings.autoJoin.always',
  never: 'settings.meetings.autoJoin.never',
};

const AUTO_SUMMARIZE_LABEL_KEY: Record<MeetAutoSummarizePolicy, string> = {
  ask: 'settings.meetings.autoSummarize.ask',
  always: 'settings.meetings.autoSummarize.always',
  never: 'settings.meetings.autoSummarize.never',
};

/**
 * Meeting Assistant settings (issue #3511 / epic #3505 PR-5).
 *
 * Surfaces four `MeetConfig` fields via `openhuman.config_{get,update}_meet_settings`:
 * auto-join policy, post-call summary policy, listen-only default, and backend
 * transcript ingestion. The orchestrator-handoff privacy gate stays in the
 * Privacy panel and is intentionally not duplicated here.
 */
const MeetingSettingsPanel = () => {
  const { t } = useT();
  const { navigateBack } = useSettingsNavigation();

  const [isLoading, setIsLoading] = useState(isTauri());
  const [isSaving, setIsSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [savedNote, setSavedNote] = useState<string | null>(null);

  const [autoJoin, setAutoJoin] = useState<MeetAutoJoinPolicy>('ask_each_time');
  const [autoSummarize, setAutoSummarize] = useState<MeetAutoSummarizePolicy>('ask');
  const [listenOnly, setListenOnly] = useState(true);
  const [ingestTranscripts, setIngestTranscripts] = useState(false);

  const persistSeqRef = useRef(0);

  useEffect(() => {
    let cancelled = false;
    const load = async () => {
      if (!isTauri()) return;
      log('load start');
      try {
        const resp = await openhumanGetMeetSettings();
        if (cancelled) return;
        const s = resp.result;
        log(
          'load ok auto_join=%s auto_summarize=%s listen_only=%s',
          s.auto_join_policy,
          s.auto_summarize_policy,
          s.listen_only_default
        );
        setAutoJoin(s.auto_join_policy);
        setAutoSummarize(s.auto_summarize_policy);
        setListenOnly(s.listen_only_default);
        setIngestTranscripts(s.ingest_backend_transcripts);
      } catch (e) {
        log('load failed err=%o', e);
        if (!cancelled) setError(e instanceof Error ? e.message : t('settings.meetings.loadError'));
      } finally {
        if (!cancelled) setIsLoading(false);
      }
    };
    void load();
    return () => {
      cancelled = true;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const persist = async (
    patch: Parameters<typeof openhumanUpdateMeetSettings>[0],
    onFailure?: () => void
  ) => {
    const seq = ++persistSeqRef.current;
    if (!isTauri()) return;
    log('persist patch=%o seq=%d', patch, seq);
    setError(null);
    setSavedNote(null);
    setIsSaving(true);
    try {
      await openhumanUpdateMeetSettings(patch);
      if (seq !== persistSeqRef.current) return;
      log('persist ok seq=%d', seq);
      setSavedNote(t('settings.meetings.saved'));
    } catch (e) {
      if (seq !== persistSeqRef.current) return;
      log('persist failed seq=%d err=%o', seq, e);
      onFailure?.();
      setError(e instanceof Error ? e.message : t('settings.meetings.saveError'));
    } finally {
      if (seq === persistSeqRef.current) setIsSaving(false);
    }
  };

  const handleAutoJoinChange = (next: MeetAutoJoinPolicy) => {
    const prev = autoJoin;
    setAutoJoin(next);
    void persist({ auto_join_policy: next }, () => setAutoJoin(prev));
  };

  const handleAutoSummarizeChange = (next: MeetAutoSummarizePolicy) => {
    const prev = autoSummarize;
    setAutoSummarize(next);
    void persist({ auto_summarize_policy: next }, () => setAutoSummarize(prev));
  };

  const handleListenOnlyChange = (next: boolean) => {
    const prev = listenOnly;
    setListenOnly(next);
    void persist({ listen_only_default: next }, () => setListenOnly(prev));
  };

  const handleIngestChange = (next: boolean) => {
    const prev = ingestTranscripts;
    setIngestTranscripts(next);
    void persist({ ingest_backend_transcripts: next }, () => setIngestTranscripts(prev));
  };

  if (!isTauri()) {
    return (
      <PanelPage
        className="z-10"
        contentClassName=""
        description={t('settings.meetings.menuDesc')}
        leading={<SettingsBackButton onBack={navigateBack} />}>
        <div className="p-4 pt-2">
          <p className="text-sm text-neutral-500 dark:text-neutral-400">
            {t('settings.meetings.desktopOnly')}
          </p>
        </div>
      </PanelPage>
    );
  }

  if (isLoading) {
    return (
      <PanelPage
        className="z-10"
        contentClassName=""
        description={t('settings.meetings.menuDesc')}
        leading={<SettingsBackButton onBack={navigateBack} />}>
        <div className="p-4 pt-2">
          <p className="text-sm text-neutral-500 dark:text-neutral-400">
            {t('settings.meetings.loading')}
          </p>
        </div>
      </PanelPage>
    );
  }

  return (
    <PanelPage
      className="z-10"
      contentClassName=""
      description={t('settings.meetings.menuDesc')}
      leading={<SettingsBackButton onBack={navigateBack} />}>
      <div className="p-4 pt-2 space-y-5" data-testid="meeting-settings-panel">
        {/* Auto-join policy */}
        <SettingsSection
          title={t('settings.meetings.autoJoin.title')}
          description={t('settings.meetings.autoJoin.desc')}>
          <SettingsRow
            stacked
            control={
              <SettingsSelect
                value={autoJoin}
                onChange={e => handleAutoJoinChange(e.target.value as MeetAutoJoinPolicy)}
                aria-label={t('settings.meetings.autoJoin.title')}>
                {AUTO_JOIN_OPTIONS.map(opt => (
                  <option key={opt} value={opt}>
                    {t(AUTO_JOIN_LABEL_KEY[opt])}
                  </option>
                ))}
              </SettingsSelect>
            }
          />
        </SettingsSection>

        {/* Auto-summarize policy */}
        <SettingsSection
          title={t('settings.meetings.autoSummarize.title')}
          description={t('settings.meetings.autoSummarize.desc')}>
          <SettingsRow
            stacked
            control={
              <SettingsSelect
                value={autoSummarize}
                onChange={e => handleAutoSummarizeChange(e.target.value as MeetAutoSummarizePolicy)}
                aria-label={t('settings.meetings.autoSummarize.title')}>
                {AUTO_SUMMARIZE_OPTIONS.map(opt => (
                  <option key={opt} value={opt}>
                    {t(AUTO_SUMMARIZE_LABEL_KEY[opt])}
                  </option>
                ))}
              </SettingsSelect>
            }
          />
        </SettingsSection>

        {/* Toggles */}
        <SettingsSection>
          <SettingsRow
            htmlFor="switch-meet-listen-only"
            label={t('settings.meetings.listenOnly')}
            description={t('settings.meetings.listenOnlyDesc')}
            control={
              <SettingsSwitch
                id="switch-meet-listen-only"
                checked={listenOnly}
                onCheckedChange={handleListenOnlyChange}
                aria-label={t('settings.meetings.listenOnly')}
              />
            }
          />
          <SettingsRow
            htmlFor="switch-meet-ingest-transcripts"
            label={t('settings.meetings.ingestTranscripts')}
            description={t('settings.meetings.ingestTranscriptsDesc')}
            control={
              <SettingsSwitch
                id="switch-meet-ingest-transcripts"
                checked={ingestTranscripts}
                onCheckedChange={handleIngestChange}
                aria-label={t('settings.meetings.ingestTranscripts')}
              />
            }
          />
        </SettingsSection>

        {/* Status line */}
        <SettingsStatusLine
          saving={isSaving}
          savedNote={savedNote}
          error={error}
          savingLabel={t('settings.meetings.saving')}
        />
      </div>
    </PanelPage>
  );
};

export default MeetingSettingsPanel;
