import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { createRef } from 'react';
import { describe, expect, it, vi } from 'vitest';

import InputGroupRoot, { InputGroupAddon, InputGroupButton, InputGroupInput } from './InputGroup';

const RAW_PALETTE = /\b(bg|text|border|ring)-(neutral|stone|slate|canvas|white|black)\b/;

describe('InputGroup', () => {
  it('renders an input with a leading addon and a trailing button', () => {
    render(
      <InputGroupRoot data-testid="group">
        <InputGroupAddon data-testid="addon">https://</InputGroupAddon>
        <InputGroupInput aria-label="Site URL" data-testid="input" />
        <InputGroupButton data-testid="button">Check</InputGroupButton>
      </InputGroupRoot>
    );

    expect(screen.getByTestId('group')).toHaveAttribute('data-slot', 'input-group');
    expect(screen.getByTestId('addon')).toHaveAttribute('data-slot', 'input-group-addon');
    expect(screen.getByTestId('input')).toHaveAttribute('data-slot', 'input-group-input');
    expect(screen.getByTestId('button')).toHaveAttribute('data-slot', 'input-group-button');
    expect(screen.getByRole('textbox', { name: 'Site URL' })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Check' })).toBeInTheDocument();
  });

  it('propagates the group size to the input, the addon and the button', () => {
    render(
      <InputGroupRoot data-testid="group" size="lg">
        <InputGroupInput aria-label="Amount" data-testid="input" />
        <InputGroupAddon data-testid="addon">kg</InputGroupAddon>
        <InputGroupButton data-testid="button">Go</InputGroupButton>
      </InputGroupRoot>
    );

    expect(screen.getByTestId('group')).toHaveAttribute('data-size', 'lg');
    expect(screen.getByTestId('addon')).toHaveAttribute('data-size', 'lg');
    // `lg` on Input and on Button are both h-11, which is the point of the map.
    expect(screen.getByTestId('input').className).toMatch(/h-11/);
    expect(screen.getByTestId('button')).toHaveAttribute('data-size', 'lg');
    expect(screen.getByTestId('button').className).toMatch(/h-11/);
  });

  it('lets a part override the inherited size', () => {
    render(
      <InputGroupRoot size="lg">
        <InputGroupInput aria-label="Amount" inputSize="sm" data-testid="input" />
        <InputGroupAddon size="sm" data-testid="addon">
          kg
        </InputGroupAddon>
      </InputGroupRoot>
    );

    expect(screen.getByTestId('input').className).toMatch(/h-8/);
    expect(screen.getByTestId('addon')).toHaveAttribute('data-size', 'sm');
    expect(screen.getByTestId('addon').className).toMatch(/h-8/);
  });

  it('joins the parts so only the outer corners round', () => {
    render(
      <InputGroupRoot data-testid="group">
        <InputGroupInput aria-label="Query" />
      </InputGroupRoot>
    );

    const group = screen.getByTestId('group').className;
    expect(group).toMatch(/\[&>\*:not\(:first-child\)\]:rounded-l-none/);
    expect(group).toMatch(/\[&>\*:not\(:last-child\)\]:rounded-r-none/);
    expect(group).toMatch(/\[&>\*:not\(:first-child\)\]:-ml-px/);
  });

  it('forwards refs, className and rest props on every part', () => {
    const rootRef = createRef<HTMLDivElement>();
    const inputRef = createRef<HTMLInputElement>();

    render(
      <InputGroupRoot ref={rootRef} data-testid="group" id="url-group" className="mt-2">
        <InputGroupInput
          ref={inputRef}
          data-testid="input"
          name="site"
          id="site"
          aria-label="Site URL"
          className="tracking-tight"
        />
      </InputGroupRoot>
    );

    expect(rootRef.current).toBe(screen.getByTestId('group'));
    expect(screen.getByTestId('group')).toHaveAttribute('id', 'url-group');
    expect(screen.getByTestId('group').className).toMatch(/mt-2/);

    const input = screen.getByTestId('input');
    expect(inputRef.current).toBe(input);
    expect(input).toHaveAttribute('name', 'site');
    expect(input).toHaveAttribute('id', 'site');
    expect(input.className).toMatch(/tracking-tight/);
  });

  it('types into the input and activates the trailing button from the keyboard', async () => {
    const user = userEvent.setup();
    const onSubmit = vi.fn();

    render(
      <InputGroupRoot>
        <InputGroupInput aria-label="Query" data-testid="input" />
        <InputGroupButton onClick={onSubmit} analyticsId="search-run">
          Search
        </InputGroupButton>
      </InputGroupRoot>
    );

    await user.tab();
    const input = screen.getByTestId('input');
    expect(input).toHaveFocus();
    await user.keyboard('openhuman');
    expect(input).toHaveValue('openhuman');

    await user.tab();
    const button = screen.getByRole('button', { name: 'Search' });
    expect(button).toHaveFocus();
    expect(button).toHaveAttribute('data-analytics-id', 'search-run');
    await user.keyboard('{Enter}');
    expect(onSubmit).toHaveBeenCalledTimes(1);
  });

  it('uses only themeable tokens — no raw palette utilities', () => {
    render(
      <InputGroupRoot data-testid="group">
        <InputGroupAddon data-testid="addon">@</InputGroupAddon>
        <InputGroupInput aria-label="Handle" data-testid="input" />
        <InputGroupButton data-testid="button">Save</InputGroupButton>
      </InputGroupRoot>
    );

    for (const id of ['group', 'addon', 'input', 'button']) {
      expect(screen.getByTestId(id).className).not.toMatch(RAW_PALETTE);
    }
  });
});
