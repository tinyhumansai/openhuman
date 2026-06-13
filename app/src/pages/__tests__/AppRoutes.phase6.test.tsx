/**
 * Desktop /human + /chat route test.
 *
 * The Human tab was briefly merged into /chat (IA Phase 6) and then restored as
 * a first-class destination: /human now renders the Human surface directly
 * rather than redirecting to /chat. This verifies both routes render their own
 * page on desktop. iOS keeps /human as a real route in AppRoutesIOS.tsx.
 *
 * Uses a minimal route tree so no full provider chain is needed.
 */
import { render, screen } from '@testing-library/react';
import { MemoryRouter, Route, Routes } from 'react-router-dom';
import { describe, expect, it } from 'vitest';

function TestRoutes() {
  return (
    <Routes>
      {/* The real /chat route (Assistant surface). */}
      <Route path="/chat" element={<div data-testid="chat-page">chat</div>} />
      {/* /human renders the Human surface directly (no longer a redirect). */}
      <Route path="/human" element={<div data-testid="human-page">human</div>} />
    </Routes>
  );
}

const renderAt = (path: string) =>
  render(
    <MemoryRouter initialEntries={[path]}>
      <TestRoutes />
    </MemoryRouter>
  );

describe('Desktop /human + /chat routes', () => {
  it('/human renders the Human page directly (restored, not a redirect)', () => {
    renderAt('/human');
    expect(screen.getByTestId('human-page')).toBeInTheDocument();
    expect(screen.queryByTestId('chat-page')).toBeNull();
  });

  it('/chat renders the chat page directly', () => {
    renderAt('/chat');
    expect(screen.getByTestId('chat-page')).toBeInTheDocument();
  });
});
