import { fireEvent, render, screen } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { ExternalSchemeLink, LinkActions } from './LinkActions';

const openUrl = vi.fn(async (_url: string) => undefined);
vi.mock('../../../../utils/openUrl', () => ({ openUrl: (url: string) => openUrl(url) }));

describe('LinkActions', () => {
  beforeEach(() => openUrl.mockClear());

  it('opens https links in the browser and offers a QR', () => {
    render(<LinkActions links={[{ url: 'https://pay.example.com/o/1', kind: 'external' }]} />);
    fireEvent.click(screen.getByText('Open in browser'));
    expect(openUrl).toHaveBeenCalledWith('https://pay.example.com/o/1');
    fireEvent.click(screen.getByText('Show QR'));
    expect(screen.getByTestId('mcp-ui-qr')).toBeInTheDocument();
  });

  it('never opens app links locally; shows a QR instead', () => {
    render(<LinkActions links={[{ url: 'upi://pay?pa=shop@bank', kind: 'handoff' }]} />);
    expect(screen.queryByText('Open in browser')).toBeNull();
    fireEvent.click(screen.getByText('Open on your phone'));
    expect(screen.getByTestId('mcp-ui-qr')).toBeInTheDocument();
    expect(openUrl).not.toHaveBeenCalled();
  });

  it('drops blocked links even if the payload calls them external', () => {
    const { container } = render(
      <LinkActions links={[{ url: 'javascript:alert(1)', kind: 'external' }]} />
    );
    expect(container).toBeEmptyDOMElement();
  });
});

describe('ExternalSchemeLink', () => {
  it('shows a QR on click', () => {
    render(<ExternalSchemeLink href="phonepe://pay?id=1">Pay with PhonePe</ExternalSchemeLink>);
    fireEvent.click(screen.getByText('Pay with PhonePe'));
    expect(screen.getByTestId('mcp-ui-qr')).toBeInTheDocument();
    expect(openUrl).not.toHaveBeenCalled();
  });
});
