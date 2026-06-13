import { fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import SettingsListItem from '../SettingsListItem';
import SettingsNumberField from '../SettingsNumberField';
import SettingsSection from '../SettingsSection';
import SettingsSwitch from '../SettingsSwitch';

// ──────────────────────────────────────────────────────────────────────────────
// SettingsSwitch
// ──────────────────────────────────────────────────────────────────────────────
describe('SettingsSwitch', () => {
  it('renders with role="switch" and correct aria-checked', () => {
    render(<SettingsSwitch id="test-switch" checked={false} onCheckedChange={() => undefined} />);
    const btn = screen.getByRole('switch');
    expect(btn).toBeInTheDocument();
    expect(btn).toHaveAttribute('aria-checked', 'false');
  });

  it('reflects checked=true in aria-checked', () => {
    render(<SettingsSwitch id="test-switch" checked={true} onCheckedChange={() => undefined} />);
    expect(screen.getByRole('switch')).toHaveAttribute('aria-checked', 'true');
  });

  it('calls onCheckedChange with toggled value on click', () => {
    const handler = vi.fn();
    render(<SettingsSwitch id="test-switch" checked={false} onCheckedChange={handler} />);
    fireEvent.click(screen.getByRole('switch'));
    expect(handler).toHaveBeenCalledWith(true);
  });

  it('does not call onCheckedChange when disabled', () => {
    const handler = vi.fn();
    render(<SettingsSwitch id="test-switch" checked={false} onCheckedChange={handler} disabled />);
    fireEvent.click(screen.getByRole('switch'));
    expect(handler).not.toHaveBeenCalled();
  });

  it('accepts aria-label and data-testid', () => {
    render(
      <SettingsSwitch
        id="test-switch"
        checked={false}
        onCheckedChange={() => undefined}
        aria-label="Enable feature"
        data-testid="my-switch"
      />
    );
    expect(screen.getByTestId('my-switch')).toBeInTheDocument();
    expect(screen.getByRole('switch', { name: 'Enable feature' })).toBeInTheDocument();
  });
});

// ──────────────────────────────────────────────────────────────────────────────
// SettingsListItem
// ──────────────────────────────────────────────────────────────────────────────
describe('SettingsListItem', () => {
  it('renders the label', () => {
    render(
      <ul>
        <SettingsListItem label="example-tool" removeLabel="Remove" />
      </ul>
    );
    expect(screen.getByText('example-tool')).toBeInTheDocument();
  });

  it('renders badge when provided', () => {
    render(
      <ul>
        <SettingsListItem
          label="example-tool"
          badge={<span data-testid="badge">Read-write</span>}
          removeLabel="Remove"
        />
      </ul>
    );
    expect(screen.getByTestId('badge')).toBeInTheDocument();
  });

  it('calls onRemove when remove button is clicked', () => {
    const onRemove = vi.fn();
    render(
      <ul>
        <SettingsListItem label="example-tool" onRemove={onRemove} removeLabel="Remove" />
      </ul>
    );
    fireEvent.click(screen.getByText('Remove'));
    expect(onRemove).toHaveBeenCalledTimes(1);
  });

  it('does not render a remove button when onRemove is absent', () => {
    render(
      <ul>
        <SettingsListItem label="example-tool" removeLabel="Remove" />
      </ul>
    );
    expect(screen.queryByText('Remove')).not.toBeInTheDocument();
  });
});

// ──────────────────────────────────────────────────────────────────────────────
// SettingsSection
// ──────────────────────────────────────────────────────────────────────────────
describe('SettingsSection', () => {
  it('renders title and children', () => {
    render(
      <SettingsSection title="Access controls">
        <div>child content</div>
      </SettingsSection>
    );
    expect(screen.getByText('Access controls')).toBeInTheDocument();
    expect(screen.getByText('child content')).toBeInTheDocument();
  });

  it('renders description when provided', () => {
    render(
      <SettingsSection title="Access controls" description="Configure access here.">
        <div />
      </SettingsSection>
    );
    expect(screen.getByText('Configure access here.')).toBeInTheDocument();
  });

  it('renders children without a title', () => {
    render(
      <SettingsSection>
        <div data-testid="child" />
      </SettingsSection>
    );
    expect(screen.getByTestId('child')).toBeInTheDocument();
  });
});

// ──────────────────────────────────────────────────────────────────────────────
// SettingsNumberField
// ──────────────────────────────────────────────────────────────────────────────
describe('SettingsNumberField', () => {
  const defaultProps = {
    id: 'timeout-field',
    value: '120',
    onChange: vi.fn(),
    onCommit: vi.fn(),
    unit: 'seconds',
    min: 1,
    max: 3600,
    'aria-label': 'Action timeout',
  };

  it('renders the input and unit label', () => {
    render(<SettingsNumberField {...defaultProps} />);
    expect(screen.getByLabelText('Action timeout')).toBeInTheDocument();
    expect(screen.getByText('seconds')).toBeInTheDocument();
  });

  it('renders the min–max range hint', () => {
    render(<SettingsNumberField {...defaultProps} />);
    expect(screen.getByText('1–3600')).toBeInTheDocument();
  });

  it('calls onCommit on blur', () => {
    const onCommit = vi.fn();
    render(<SettingsNumberField {...defaultProps} onCommit={onCommit} />);
    fireEvent.blur(screen.getByLabelText('Action timeout'));
    expect(onCommit).toHaveBeenCalledTimes(1);
  });

  it('calls onCommit on Enter keydown', () => {
    const onCommit = vi.fn();
    render(<SettingsNumberField {...defaultProps} onCommit={onCommit} />);
    fireEvent.keyDown(screen.getByLabelText('Action timeout'), { key: 'Enter' });
    expect(onCommit).toHaveBeenCalledTimes(1);
  });

  it('calls onChange when value changes', () => {
    const onChange = vi.fn();
    render(<SettingsNumberField {...defaultProps} onChange={onChange} />);
    fireEvent.change(screen.getByLabelText('Action timeout'), { target: { value: '300' } });
    expect(onChange).toHaveBeenCalledWith('300');
  });
});
