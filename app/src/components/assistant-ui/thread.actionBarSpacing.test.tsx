import {
  AssistantRuntimeProvider,
  type ThreadMessageLike,
  useExternalStoreRuntime,
} from '@assistant-ui/react';
import { render, screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';

import { Thread } from './thread';

/**
 * The assistant action bar reserves its own height (`min-h-*`) so a bar revealed
 * on hover does not shift the transcript, then hands most of that reservation
 * back with a negative bottom margin (`-mb-*`) so it is drawn *inside* the gap
 * the message group already provides rather than stacking on top of it.
 *
 * The amount handed back has to be exactly the message group's `gap-y-*`, and
 * both failure directions have shipped or been caught in review:
 *
 * - Give back nothing (the `-mb` had drifted onto the message root, where it
 *   only cancelled that element's own paint-box `pb`) and the full reservation
 *   becomes dead space: a 30px band under every assistant turn, on top of the
 *   gap — consecutive replies ~54px apart instead of 30px.
 * - Give back the whole reservation and the bar is pulled deeper than the gap
 *   is tall, so its tail paints over the following message's first line, at the
 *   same left inset (`ms-2` on the bar, `px-2` on the content).
 *
 * jsdom performs no layout, so the utilities are the only observable here. They
 * are read as an invariant relating two elements — pulled === available gap —
 * rather than asserted as literals, so retuning either stays free while
 * breaking their correspondence does not.
 */

const messages: ThreadMessageLike[] = [
  { role: 'user', content: [{ type: 'text', text: 'hello' }] },
  { role: 'assistant', content: [{ type: 'text', text: 'first reply' }] },
  { role: 'assistant', content: [{ type: 'text', text: 'second reply' }] },
];

function Harness() {
  const runtime = useExternalStoreRuntime({
    messages,
    isRunning: false,
    convertMessage: (m: ThreadMessageLike) => m,
    onNew: async () => {},
  });
  return (
    <AssistantRuntimeProvider runtime={runtime}>
      <Thread />
    </AssistantRuntimeProvider>
  );
}

/**
 * The numeric step of a Tailwind spacing utility — the `6` in `gap-y-6`, the
 * `7.5` in `min-h-7.5` — or `null` when the element does not carry one.
 */
function spacingStep(el: HTMLElement, prefix: string): number | null {
  const token = el.className.split(/\s+/).find(cls => cls.startsWith(`${prefix}-`));
  if (!token) return null;
  const step = Number(token.slice(prefix.length + 1));
  return Number.isFinite(step) ? step : null;
}

describe('assistant message action bar spacing', () => {
  it('draws the action bar inside the inter-message gap, filling it exactly', () => {
    const { container } = render(<Harness />);

    const group = container.querySelector<HTMLElement>('[data-slot="aui_message-group"]');
    expect(group).not.toBeNull();
    const gap = spacingStep(group as HTMLElement, 'gap-y');
    expect(gap).not.toBeNull();

    const footers = screen.getAllByTestId('agent-message').map(root => {
      const footer = root.querySelector<HTMLElement>('[data-slot="aui_assistant-message-footer"]');
      expect(footer).not.toBeNull();
      return footer as HTMLElement;
    });
    expect(footers.length).toBeGreaterThan(0);

    for (const footer of footers) {
      const reserved = spacingStep(footer, 'min-h');
      const pulled = spacingStep(footer, '-mb');

      // Reserving the bar's height is what keeps a hover-revealed bar from
      // shifting the transcript, so the reservation itself must be present.
      expect(reserved).not.toBeNull();

      // Pulling back less than the gap leaves dead space under every turn;
      // pulling back more paints the bar over the next message's first line.
      expect(pulled).toBe(gap);

      // The bar is drawn into the gap, so it must not be taller than the space
      // the pull actually buys it.
      expect(reserved as number).toBeLessThanOrEqual((pulled as number) + (gap as number));
    }
  });
});
