/**
 * Shared `<openhuman-link>` rendering for notification surfaces
 * (`NotificationCard` and the `Notifications` page).
 *
 * Notification bodies emitted by Rust error helpers can contain
 * `<openhuman-link path="…">…</openhuman-link>` tags (e.g. morning-briefing
 * failures pointing users at Discord / Settings). Without parsing, the raw
 * tag leaks as literal text. This module mirrors the chat-side pill so the
 * tag renders as a clickable pill instead.
 *
 * **Why this lives here and not in a global shared spot:** the chat-side
 * `OpenhumanLinkPill` is a non-exported function inside `AgentMessageBubble.tsx`
 * (`app/src/pages/conversations/`). Extracting from chat would change the chat
 * render path — out of scope for this fix. Instead, we keep the grammar / parsing
 * shared (reuses `parseBubbleSegments` from conversations) but reimplement the
 * pill locally. Both notification surfaces share *this* file so the diff stays
 * testable with one Vitest suite.
 *
 * Safety: this component renders **only** text + button elements. It never
 * uses `dangerouslySetInnerHTML`, never sets an `href`, and the dispatched
 * `OPENHUMAN_LINK_EVENT` is consumed by `OpenhumanLinkModal`, which hard-
 * allowlists `path` values before routing. See `OpenhumanLinkModal.tsx`
 * `ALLOWED_PATHS_SET`.
 */
import { parseBubbleSegments } from '../../pages/conversations/utils/format';
import { OPENHUMAN_LINK_EVENT } from '../OpenhumanLinkModal';

function NotificationLinkPill({ path, label }: { path: string; label: string }) {
  return (
    <button
      type="button"
      onClick={e => {
        // Don't trigger the surrounding card / row click — the pill is its
        // own action.
        e.stopPropagation();
        window.dispatchEvent(new CustomEvent(OPENHUMAN_LINK_EVENT, { detail: { path } }));
      }}
      className="inline-flex items-center gap-1 rounded-full border border-primary-200 bg-primary-50 px-2 py-0.5 text-[11px] font-medium text-primary-700 transition-colors hover:bg-primary-100">
      {label}
      <svg className="h-2.5 w-2.5" viewBox="0 0 24 24" fill="none" stroke="currentColor">
        <path
          strokeLinecap="round"
          strokeLinejoin="round"
          strokeWidth={2}
          d="M5 12h14M13 6l6 6-6 6"
        />
      </svg>
    </button>
  );
}

export default function NotificationBody({ body }: { body: string }) {
  const segments = parseBubbleSegments(body);
  return (
    <>
      {segments.map((seg, i) =>
        seg.kind === 'link' ? (
          <NotificationLinkPill key={i} path={seg.path} label={seg.label} />
        ) : (
          // React auto-escapes text content, so any other markup in the body
          // (e.g. `<script>…</script>`) renders as literal text.
          <span key={i}>{seg.text}</span>
        )
      )}
    </>
  );
}
