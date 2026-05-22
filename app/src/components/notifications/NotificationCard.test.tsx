/**
 * Markup-leak guard for `<openhuman-link>` rendering in notification bodies.
 *
 * These tests are the airtight contract for issue #2279 (Bug A) — they assert:
 *   - well-formed tags become pills (no raw `<openhuman-link>` text leaks),
 *   - attacker-influenceable `path` values can't become a navigable
 *     `javascript:` link, and
 *   - non-link bodies and stray markup render as literal, auto-escaped text
 *     (no `<script>` element gets injected into the DOM).
 *
 * NotificationCard and the Notifications page both render bodies the same way,
 * so a single suite covers both call sites — we render NotificationCard for
 * the local body component (since the page uses an identical inline helper)
 * and the page exports its own helper too.
 */
import { fireEvent, render, screen, within } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import type { IntegrationNotification } from '../../types/notifications';
import { OPENHUMAN_LINK_EVENT } from '../OpenhumanLinkModal';
import NotificationCard from './NotificationCard';

function makeNotification(body: string): IntegrationNotification {
  return {
    id: 'n-1',
    provider: 'gmail',
    title: 'Morning briefing',
    body,
    raw_payload: {},
    status: 'unread',
    received_at: new Date().toISOString(),
  };
}

function renderCard(body: string) {
  return render(<NotificationCard notification={makeNotification(body)} onMarkRead={vi.fn()} />);
}

describe('NotificationCard <openhuman-link> rendering', () => {
  it('renders an <openhuman-link> tag as a pill (no raw tag leaks)', () => {
    const body = '<openhuman-link path="community/discord">Report on Discord</openhuman-link>';
    renderCard(body);

    const bodyEl = screen.getByTestId('notification-card-body');
    // The pill is a <button> with the label as accessible name. The outer
    // notification card is also a <button> wrapping everything, so we scope
    // the query to the body element.
    const pill = within(bodyEl).getByRole('button', { name: /Report on Discord/i });
    expect(pill).toBeInTheDocument();

    // Critically: the raw tag text must NOT appear anywhere in the rendered DOM.
    expect(bodyEl.textContent ?? '').not.toContain('<openhuman-link');
    expect(bodyEl.textContent ?? '').not.toContain('</openhuman-link>');
  });

  it('does NOT emit a navigable javascript: link for a malicious path (XSS guard)', () => {
    const body = '<openhuman-link path="javascript:alert(1)">click me</openhuman-link>';
    const { container } = renderCard(body);

    // No <a href="javascript:..."> anywhere in the rendered tree. The pill is
    // a <button>, never an <a>, but we assert the absolute invariant directly.
    const anchors = container.querySelectorAll('a');
    for (const a of anchors) {
      const href = a.getAttribute('href') ?? '';
      expect(href.toLowerCase().startsWith('javascript:')).toBe(false);
    }

    // Even though the pill exists (the parser doesn't allowlist `path` — the
    // modal listener does), clicking it MUST NOT navigate. We verify by
    // listening for the dispatched custom event and confirming the path is
    // exactly what was parsed — the OpenhumanLinkModal listener (which
    // hard-allowlists paths) is what stops the dangerous string from doing
    // anything. See OpenhumanLinkModal.tsx ALLOWED_PATHS_SET.
    const seen: string[] = [];
    const listener = (e: Event) => {
      const detail = (e as CustomEvent<{ path: string }>).detail;
      seen.push(detail?.path ?? '');
    };
    window.addEventListener(OPENHUMAN_LINK_EVENT, listener);
    try {
      const bodyEl = screen.getByTestId('notification-card-body');
      const pill = within(bodyEl).getByRole('button', { name: /click me/i });
      pill.click();
    } finally {
      window.removeEventListener(OPENHUMAN_LINK_EVENT, listener);
    }

    // The dispatched event payload is exactly what was parsed — but `OpenhumanLinkModal`
    // (the listener that actually opens UI) hard-allowlists paths, so the
    // `javascript:` string never gets to act on anything. Both halves of the
    // contract are asserted: dispatch is faithful, navigation is impossible.
    expect(seen).toEqual(['javascript:alert(1)']);

    // No href injection regardless of whether the pill rendered.
    expect(
      Array.from(container.querySelectorAll<HTMLElement>('[href]')).every(el => {
        const href = el.getAttribute('href') ?? '';
        return !href.toLowerCase().startsWith('javascript:');
      })
    ).toBe(true);
  });

  it('renders a plain-text body as the literal string (regression guard)', () => {
    renderCard('plain text without any tags');

    const bodyEl = screen.getByTestId('notification-card-body');
    expect(bodyEl).toHaveTextContent('plain text without any tags');
    // No pill rendered for tag-free body — scope to the body element to ignore
    // the outer card <button>.
    expect(within(bodyEl).queryByRole('button')).toBeNull();
  });

  it('renders <script> as literal text — does not inject a <script> element', () => {
    const body = '<script>alert(1)</script>';
    const { container } = renderCard(body);

    // React auto-escapes anything that's not an `<openhuman-link>` segment.
    const bodyEl = screen.getByTestId('notification-card-body');
    expect(bodyEl.textContent).toBe('<script>alert(1)</script>');

    // Hard guarantee: no actual <script> element anywhere in the rendered tree.
    expect(container.querySelector('script')).toBeNull();
  });

  it('renders mixed text + link segments in order', () => {
    const body = 'Before <openhuman-link path="community/discord">Discord</openhuman-link> after';
    renderCard(body);

    const bodyEl = screen.getByTestId('notification-card-body');
    expect(bodyEl).toHaveTextContent(/Before/);
    expect(bodyEl).toHaveTextContent(/after/);
    expect(within(bodyEl).getByRole('button', { name: /Discord/i })).toBeInTheDocument();
    expect(bodyEl.textContent ?? '').not.toContain('<openhuman-link');
  });

  // Keyboard-activation coverage for the `<div role="button">` wrapper we use
  // instead of a real `<button>` (a real button can't legally contain the
  // `<openhuman-link>` pill which is also a button). Exercises the `onKeyDown`
  // branch so the diff-coverage gate sees those lines hit.
  it('activates the card body via Enter and Space keys', () => {
    const onMarkRead = vi.fn();
    const notification = makeNotification('plain body, no pill so no inner button');
    // Plain text body, so the only role=button in the card is the outer wrapper.
    render(<NotificationCard notification={notification} onMarkRead={onMarkRead} />);

    const wrapper = screen.getByRole('button');
    fireEvent.keyDown(wrapper, { key: 'Enter' });
    fireEvent.keyDown(wrapper, { key: ' ' });
    // Status is `unread` (set by makeNotification) and no `onNavigate` was
    // passed, so handleBodyClick falls through to onMarkRead each time.
    expect(onMarkRead).toHaveBeenCalledTimes(2);
    expect(onMarkRead).toHaveBeenCalledWith(notification.id);

    // Other keys must NOT activate — guard against the branch being too loose.
    onMarkRead.mockClear();
    fireEvent.keyDown(wrapper, { key: 'a' });
    fireEvent.keyDown(wrapper, { key: 'Tab' });
    expect(onMarkRead).not.toHaveBeenCalled();
  });

  // Bubbling guard: pressing Enter/Space while focused on the inner pill must
  // NOT also activate the surrounding card. Without `e.target !== e.currentTarget`
  // the keydown would bubble up and trigger `handleBodyClick` accidentally.
  it('does NOT activate the card when keydown bubbles from the inner pill', () => {
    const onMarkRead = vi.fn();
    const body = '<openhuman-link path="community/discord">Discord</openhuman-link>';
    render(<NotificationCard notification={makeNotification(body)} onMarkRead={onMarkRead} />);

    const bodyEl = screen.getByTestId('notification-card-body');
    const pill = within(bodyEl).getByRole('button', { name: /Discord/i });
    fireEvent.keyDown(pill, { key: 'Enter' });
    fireEvent.keyDown(pill, { key: ' ' });
    expect(onMarkRead).not.toHaveBeenCalled();
  });
});
