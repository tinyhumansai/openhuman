import { useCallback, useEffect, useRef, useState } from 'react';

import Conversations from '../../pages/Conversations';
import {
  type MeetAgentEvent,
  meetAgentJoin,
  meetAgentLeave,
  subscribeMeetAgentEvents,
} from '../../services/meetAgent';
import { APP_ENVIRONMENT } from '../../utils/config';
import { Ghosty } from './Mascot';
import { useHumanMascot } from './useHumanMascot';

const SPEAK_REPLIES_KEY = 'human.speakReplies';
const MEET_AGENT_ACCOUNT_KEY = 'human.meetAgent.accountId';
const MEET_AGENT_URL_KEY = 'human.meetAgent.meetingUrl';

/** Staging-only Meet Agent dev panel. Hidden in production. */
function MeetAgentPanel() {
  const [accountId, setAccountId] = useState<string>(
    () => window.localStorage.getItem(MEET_AGENT_ACCOUNT_KEY) ?? ''
  );
  const [meetingUrl, setMeetingUrl] = useState<string>(
    () => window.localStorage.getItem(MEET_AGENT_URL_KEY) ?? ''
  );
  const [statusLine, setStatusLine] = useState<string>('idle');
  const unsubRef = useRef<(() => void) | null>(null);

  // Persist inputs to localStorage as they change.
  useEffect(() => {
    window.localStorage.setItem(MEET_AGENT_ACCOUNT_KEY, accountId);
  }, [accountId]);
  useEffect(() => {
    window.localStorage.setItem(MEET_AGENT_URL_KEY, meetingUrl);
  }, [meetingUrl]);

  // Subscribe to lifecycle events on mount, unsubscribe on unmount.
  useEffect(() => {
    const unsub = subscribeMeetAgentEvents((e: MeetAgentEvent) => {
      if (e.kind === 'meet_agent_joined') {
        const time = new Date(e.joinedAt).toLocaleTimeString();
        setStatusLine(`joined ${e.code} at ${time}`);
      } else if (e.kind === 'meet_agent_left') {
        setStatusLine(`left (${e.reason})`);
      } else if (e.kind === 'meet_agent_failed') {
        setStatusLine(`failed (${e.reason})`);
      }
    });
    unsubRef.current = unsub;
    return () => {
      unsub();
      unsubRef.current = null;
    };
  }, []);

  const handleJoin = useCallback(async () => {
    if (!accountId || !meetingUrl) return;
    setStatusLine('joining...');
    try {
      await meetAgentJoin({ accountId, meetingUrl });
    } catch (err) {
      setStatusLine(`error: ${String(err)}`);
    }
  }, [accountId, meetingUrl]);

  const handleLeave = useCallback(async () => {
    if (!accountId) return;
    try {
      await meetAgentLeave({ accountId });
      setStatusLine('idle');
    } catch (err) {
      setStatusLine(`error: ${String(err)}`);
    }
  }, [accountId]);

  return (
    <div className="absolute bottom-24 left-4 z-20 w-72 rounded-xl border border-amber-300 bg-amber-50/90 backdrop-blur-sm p-3 shadow-soft text-xs text-stone-700 space-y-2">
      <div className="font-semibold text-amber-700">Meet Agent (staging)</div>
      <input
        className="w-full rounded border border-stone-300 px-2 py-1 text-xs bg-white"
        placeholder="account-id"
        value={accountId}
        onChange={e => setAccountId(e.target.value)}
        data-testid="meet-agent-account-id"
      />
      <input
        className="w-full rounded border border-stone-300 px-2 py-1 text-xs bg-white"
        placeholder="https://meet.google.com/xxx-xxxx-xxx"
        value={meetingUrl}
        onChange={e => setMeetingUrl(e.target.value)}
        data-testid="meet-agent-meeting-url"
      />
      <div className="flex gap-2">
        <button
          className="flex-1 rounded bg-ocean-600 text-white py-1 px-2 hover:bg-ocean-700 disabled:opacity-40 text-xs"
          onClick={() => void handleJoin()}
          disabled={!accountId || !meetingUrl}
          data-testid="meet-agent-join">
          Join
        </button>
        <button
          className="flex-1 rounded bg-stone-200 text-stone-700 py-1 px-2 hover:bg-stone-300 disabled:opacity-40 text-xs"
          onClick={() => void handleLeave()}
          disabled={!accountId}
          data-testid="meet-agent-leave">
          Leave
        </button>
      </div>
      <div className="text-stone-500 truncate" data-testid="meet-agent-status">
        {statusLine}
      </div>
    </div>
  );
}

const HumanPage = () => {
  const [speakReplies, setSpeakReplies] = useState<boolean>(() => {
    const raw = window.localStorage.getItem(SPEAK_REPLIES_KEY);
    return raw === null ? true : raw === '1';
  });

  useEffect(() => {
    window.localStorage.setItem(SPEAK_REPLIES_KEY, speakReplies ? '1' : '0');
  }, [speakReplies]);

  const { face, viseme } = useHumanMascot({ speakReplies });

  // Sidebar reserves ~436px (420px panel + 16px gutter) on the right; the
  // mascot stage takes the remaining width so the two never overlap.
  return (
    <div className="absolute inset-0 bg-stone-100 overflow-hidden">
      <div
        className="pointer-events-none absolute inset-0"
        style={{
          background: 'radial-gradient(ellipse at 35% 40%, rgba(74,131,221,0.10), transparent 60%)',
        }}
      />

      {/* Mascot stage — fills the area to the left of the reserved sidebar column. */}
      <div className="absolute inset-y-0 left-0 right-[436px] flex items-center justify-center">
        <div className="relative w-[min(80vh,90%)] aspect-square">
          <Ghosty face={face} viseme={viseme} />
        </div>
      </div>

      <label className="absolute top-4 left-4 z-10 inline-flex items-center gap-2 px-3 py-1.5 rounded-full bg-white/80 backdrop-blur-sm border border-stone-300 text-xs text-stone-700 shadow-soft cursor-pointer select-none">
        <input
          type="checkbox"
          checked={speakReplies}
          onChange={e => setSpeakReplies(e.target.checked)}
          className="cursor-pointer"
        />
        Speak replies
      </label>

      {/* Staging-only: Meet Agent dev panel */}
      {APP_ENVIRONMENT !== 'production' && <MeetAgentPanel />}

      {/* Chat sidebar — vertically centered above the BottomTabBar (~80px). */}
      <div className="absolute right-4 top-0 bottom-20 z-10 flex items-center">
        <aside className="w-[420px] h-[min(720px,calc(100vh-160px))] rounded-2xl border border-stone-300 bg-white shadow-soft flex flex-col overflow-hidden">
          <Conversations variant="sidebar" />
        </aside>
      </div>
    </div>
  );
};

export default HumanPage;
