import { fireEvent, render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { useState } from 'react';
import { describe, expect, it, vi } from 'vitest';

import ChipTabs, { type ChipTabItem } from './ChipTabs';

type TabId = 'one' | 'two' | 'three';

const items: ChipTabItem<TabId>[] = [
  { id: 'one', label: 'One' },
  { id: 'two', label: 'Two' },
  { id: 'three', label: 'Three' },
];

describe('ChipTabs', () => {
  it('renders a tablist with one chip per item by default', () => {
    render(<ChipTabs items={items} value="one" onChange={() => {}} ariaLabel="Sections" />);

    const list = screen.getByRole('tablist', { name: 'Sections' });
    expect(list).toBeInTheDocument();
    expect(screen.getAllByRole('tab')).toHaveLength(3);
  });

  it('marks the active chip with aria-selected', () => {
    render(<ChipTabs items={items} value="two" onChange={() => {}} testIdPrefix="t" />);

    expect(screen.getByTestId('t-two')).toHaveAttribute('aria-selected', 'true');
    expect(screen.getByTestId('t-one')).toHaveAttribute('aria-selected', 'false');
    expect(screen.getByTestId('t-three')).toHaveAttribute('aria-selected', 'false');
  });

  // COVERAGE GAP, deliberate. `uses roving tabIndex for keyboard navigation`
  // asserted the hand-rolled implementation's STATIC roving tabIndex
  // (tabIndex = f(active value), true from first render). Radix `Tabs`'
  // `RovingFocusGroup` genuinely does not work that way, in a browser or in
  // jsdom: `tabIndex` starts at -1 on EVERY item (verified directly — asking
  // for `tabindex="0"` on the active chip immediately after render, with no
  // interaction, returns `-1`) because the roving tab stop is a one-time
  // ENTRY behavior. The group root is the tab stop until something actually
  // focuses into it, at which point `RovingFocusGroup` redirects focus to the
  // active item and only THEN does that item become the roving stop. There is
  // no static "the active item already has tabIndex 0" invariant to assert
  // pre-interaction, so this one cannot be restored — not because jsdom can't
  // traverse focus, but because the behavior it asserted no longer exists.
  //
  // The other three tests below — the arrow-key it.each and the wrap test —
  // are NOT a coverage gap: they're restored using `userEvent.keyboard(...)`
  // rather than raw `fireEvent.keyDown` + manual `.focus()`. The raw-event
  // version failed (`onChange` called 0 times) because Radix's roving-focus
  // internals depend on the real sequential focus/keydown event chain
  // `userEvent` models — the same reason this codebase's `ToggleGroup.test.tsx`
  // only exercises Radix roving focus through `userEvent`, never `fireEvent`.
  // The wrap test additionally needs a small controlled harness: Radix only
  // fires `onValueChange` for a value that differs from the CURRENT `value`
  // prop, so a wrap-then-wrap-back assertion against a component whose parent
  // never re-renders with the intermediate value silently no-ops on the
  // second step (verified) — not a Radix or jsdom quirk, just what "controlled"
  // means.

  it.each([
    ['{ArrowRight}', 'three'],
    ['{ArrowLeft}', 'one'],
    ['{Home}', 'one'],
    ['{End}', 'three'],
  ] as const)('moves focus and selects with %s', async (key, expectedId) => {
    const user = userEvent.setup();
    const onChange = vi.fn();
    render(<ChipTabs items={items} value="two" onChange={onChange} testIdPrefix="t" />);

    screen.getByTestId('t-two').focus();
    await user.keyboard(key);

    expect(onChange).toHaveBeenCalledWith(expectedId);
    expect(screen.getByTestId(`t-${expectedId}`)).toHaveFocus();
  });

  it('wraps arrow-key navigation at either end', async () => {
    // A real controlled harness — see the coverage-gap comment above for why
    // a static `value` prop can't exercise a second wrap in the same test.
    function Controlled({ onSelect }: { onSelect: (id: TabId) => void }) {
      const [value, setValue] = useState<TabId>('one');
      return (
        <ChipTabs
          items={items}
          value={value}
          onChange={id => {
            setValue(id);
            onSelect(id);
          }}
          testIdPrefix="t"
        />
      );
    }

    const user = userEvent.setup();
    const onChange = vi.fn();
    render(<Controlled onSelect={onChange} />);

    screen.getByTestId('t-one').focus();
    await user.keyboard('{ArrowLeft}');
    expect(onChange).toHaveBeenLastCalledWith('three');

    await user.keyboard('{ArrowRight}');
    expect(onChange).toHaveBeenLastCalledWith('one');
  });

  it('connects a tab to its panel through stable IDs', () => {
    render(
      <>
        <ChipTabs
          items={[{ id: 'one', label: 'One', controls: 'panel-one', labelledBy: 'tab-one' }]}
          value="one"
          onChange={() => {}}
        />
        <div id="panel-one" role="tabpanel" aria-labelledby="tab-one">
          First panel
        </div>
      </>
    );

    const tab = screen.getByRole('tab', { name: 'One' });
    const panel = screen.getByRole('tabpanel', { name: 'One' });
    expect(tab).toHaveAttribute('id', 'tab-one');
    expect(tab).toHaveAttribute('aria-controls', 'panel-one');
    expect(panel).toHaveAttribute('aria-labelledby', tab.id);
  });

  it('reduces chip padding in compact mode', () => {
    const { rerender } = render(
      <ChipTabs items={items} value="one" onChange={() => {}} testIdPrefix="t" />
    );
    expect(screen.getByTestId('t-one')).toHaveClass('px-3', 'py-1');

    rerender(<ChipTabs items={items} value="one" onChange={() => {}} testIdPrefix="t" compact />);
    expect(screen.getByTestId('t-one')).toHaveClass('px-2', 'py-0.5');
    expect(screen.getByTestId('t-one')).not.toHaveClass('px-3', 'py-1');
  });

  it('emits onChange with the clicked chip id', () => {
    const onChange = vi.fn();
    render(<ChipTabs items={items} value="one" onChange={onChange} testIdPrefix="t" />);

    fireEvent.click(screen.getByTestId('t-three'));
    expect(onChange).toHaveBeenCalledWith('three');
  });

  it('uses an explicit per-item testId over the prefix', () => {
    render(
      <ChipTabs
        items={[{ id: 'one', label: 'One', testId: 'custom-chip' }]}
        value="one"
        onChange={() => {}}
        testIdPrefix="t"
      />
    );

    expect(screen.getByTestId('custom-chip')).toBeInTheDocument();
    expect(screen.queryByTestId('t-one')).not.toBeInTheDocument();
  });

  it('renders navigation semantics with aria-current when as="nav"', () => {
    render(<ChipTabs items={items} value="two" onChange={() => {}} as="nav" ariaLabel="Sub nav" />);

    expect(screen.getByRole('navigation', { name: 'Sub nav' })).toBeInTheDocument();
    // No tab roles in nav mode.
    expect(screen.queryAllByRole('tab')).toHaveLength(0);
    expect(screen.getByText('Two')).toHaveAttribute('aria-current', 'page');
    expect(screen.getByText('One')).not.toHaveAttribute('aria-current');
    expect(screen.getByText('Two')).not.toHaveAttribute('tabindex');
  });
});
