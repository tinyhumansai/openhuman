import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import type { SelfIdentity } from '../../lib/orchestration/orchestrationClient';
import SelfIdentityCard from './SelfIdentityCard';

vi.mock('../../lib/i18n/I18nContext', () => ({ useT: () => ({ t: (k: string) => k }) }));

const discoverable: SelfIdentity = {
  agentId: '6wNaBJkatir4B86cw5ykHZWQ3xoNaKygX5vAU9MQbHSh',
  handles: [{ username: 'openhuman', primary: true }],
  primaryHandle: 'openhuman',
  cardPublished: true,
  keyPublished: true,
  discoverable: true,
};

describe('SelfIdentityCard', () => {
  beforeEach(() => {
    Object.assign(navigator, { clipboard: { writeText: vi.fn().mockResolvedValue(undefined) } });
  });

  it('shows a loading state before the identity resolves', () => {
    render(<SelfIdentityCard identity={null} loading />);
    expect(screen.getByTestId('tinyplace-self-identity')).toHaveTextContent(
      'tinyplaceOrchestration.identity.loading'
    );
  });

  it('renders nothing once loaded with no identity', () => {
    const { container } = render(<SelfIdentityCard identity={null} loading={false} />);
    expect(container.firstChild).toBeNull();
  });

  it('renders the primary handle, shortened address and discoverable status', () => {
    render(<SelfIdentityCard identity={discoverable} loading={false} />);
    expect(screen.getByText('@openhuman')).toBeInTheDocument();
    // Address is shortened but the full value is preserved in the title.
    expect(screen.getByText('6wNaBJ…QbHSh')).toHaveAttribute('title', discoverable.agentId);
    const status = screen.getByTestId('tinyplace-self-identity-status');
    expect(status).toHaveAttribute('data-discoverable', 'true');
  });

  it('flags an un-messageable identity and shows the register hint', () => {
    const undiscoverable: SelfIdentity = {
      agentId: 'addrWithNoCardPublishedYet',
      handles: [],
      cardPublished: false,
      keyPublished: false,
      discoverable: false,
    };
    render(<SelfIdentityCard identity={undiscoverable} loading={false} />);
    expect(screen.getByText('tinyplaceOrchestration.identity.noHandle')).toBeInTheDocument();
    expect(screen.getByTestId('tinyplace-self-identity-status')).toHaveAttribute(
      'data-discoverable',
      'false'
    );
    expect(
      screen.getByText('tinyplaceOrchestration.identity.undiscoverableHint')
    ).toBeInTheDocument();
  });

  const undiscoverable: SelfIdentity = {
    agentId: 'addrWithNoCardPublishedYet',
    handles: [{ username: 'openhuman', primary: true }],
    primaryHandle: 'openhuman',
    cardPublished: false,
    keyPublished: false,
    discoverable: false,
  };

  it('renders a "Make discoverable" button that fires onPublish while undiscoverable', () => {
    const onPublish = vi.fn();
    render(<SelfIdentityCard identity={undiscoverable} loading={false} onPublish={onPublish} />);
    const btn = screen.getByTestId('tinyplace-self-identity-publish');
    expect(btn).toHaveTextContent('tinyplaceOrchestration.identity.makeDiscoverable');
    fireEvent.click(btn);
    expect(onPublish).toHaveBeenCalledTimes(1);
  });

  it('disables the button and shows the publishing label while a publish is in flight', () => {
    render(
      <SelfIdentityCard identity={undiscoverable} loading={false} onPublish={vi.fn()} publishing />
    );
    const btn = screen.getByTestId('tinyplace-self-identity-publish');
    expect(btn).toBeDisabled();
    expect(btn).toHaveTextContent('tinyplaceOrchestration.identity.publishing');
  });

  it('surfaces a publish error under the button', () => {
    render(
      <SelfIdentityCard
        identity={undiscoverable}
        loading={false}
        onPublish={vi.fn()}
        publishError="boom"
      />
    );
    expect(screen.getByTestId('tinyplace-self-identity-publish-error')).toHaveTextContent(
      'tinyplaceOrchestration.identity.publishFailed'
    );
  });

  it('shows no "make discoverable" button once discoverable', () => {
    render(<SelfIdentityCard identity={discoverable} loading={false} onPublish={vi.fn()} />);
    expect(screen.queryByTestId('tinyplace-self-identity-publish')).toBeNull();
  });

  it('offers a "Republish keys" action while discoverable that fires onPublish', () => {
    const onPublish = vi.fn();
    render(<SelfIdentityCard identity={discoverable} loading={false} onPublish={onPublish} />);
    const btn = screen.getByTestId('tinyplace-self-identity-republish');
    expect(btn).toHaveTextContent('tinyplaceOrchestration.identity.republish');
    fireEvent.click(btn);
    expect(onPublish).toHaveBeenCalledTimes(1);
  });

  it('omits the publish button when no onPublish handler is provided', () => {
    render(<SelfIdentityCard identity={undiscoverable} loading={false} />);
    expect(screen.queryByTestId('tinyplace-self-identity-publish')).toBeNull();
  });

  it('copies the address to the clipboard on click', async () => {
    render(<SelfIdentityCard identity={discoverable} loading={false} />);
    fireEvent.click(screen.getByTestId('tinyplace-self-identity-copy'));
    expect(navigator.clipboard.writeText).toHaveBeenCalledWith(discoverable.agentId);
    await waitFor(() =>
      expect(screen.getByTestId('tinyplace-self-identity-copy')).toHaveTextContent(
        'tinyplaceOrchestration.identity.copied'
      )
    );
  });
});
