/**
 * Accessibility smoke lane (plan.md §7 — the frontend had zero a11y assertions).
 * Renders a few high-traffic, self-contained components and runs axe-core over
 * the output, asserting no violations. jsdom can't compute layout, so
 * color-contrast is auto-skipped; this catches the structural a11y bugs that
 * matter — missing roles/labels, invalid ARIA, unlabelled controls.
 *
 * Kept deliberately small and provider-light so it stays fast and stable; grow
 * it screen-by-screen rather than pulling in the full app shell.
 */
import { configureStore } from '@reduxjs/toolkit';
import { render } from '@testing-library/react';
import { axe } from 'jest-axe';
import { Provider } from 'react-redux';
import { describe, expect, it, vi } from 'vitest';

import chatRuntimeReducer, {
  type ArtifactSnapshot,
  type PendingApproval,
  setPendingApprovalForThread,
} from '../../store/chatRuntimeSlice';
import ApprovalRequestCard from '../chat/ApprovalRequestCard';
import ArtifactCard from '../chat/ArtifactCard';

vi.mock('../../services/artifactDownloadService', () => ({
  saveArtifactViaDialog: vi.fn(),
  revealArtifactInFileManager: vi.fn(),
}));
vi.mock('../../services/coreRpcClient', () => ({ callCoreRpc: vi.fn() }));

async function expectNoViolations(container: HTMLElement) {
  const results = await axe(container);
  expect(results.violations).toEqual([]);
}

describe('accessibility smoke', () => {
  it('ArtifactCard (ready) has no axe violations', async () => {
    const artifact: ArtifactSnapshot = {
      artifactId: 'a-1',
      kind: 'presentation',
      title: 'Quarterly Deck',
      status: 'ready',
      sizeBytes: 4096,
      path: 'a-1/deck.pptx',
      updatedAt: 0,
    };
    const { container } = render(<ArtifactCard artifact={artifact} />);
    await expectNoViolations(container);
  });

  it('ArtifactCard (failed with error) has no axe violations', async () => {
    const artifact: ArtifactSnapshot = {
      artifactId: 'a-2',
      kind: 'document',
      title: 'Report',
      status: 'failed',
      error: 'producer crashed',
      updatedAt: 0,
    };
    const { container } = render(<ArtifactCard artifact={artifact} onRetry={vi.fn()} />);
    await expectNoViolations(container);
  });

  it('ApprovalRequestCard has no axe violations', async () => {
    const approval: PendingApproval = {
      requestId: 'req-1',
      toolName: 'shell',
      message: 'Run `shell` — shell (18 bytes of arguments)',
      command: 'pip show yfinance',
    };
    const store = configureStore({ reducer: { chatRuntime: chatRuntimeReducer } });
    store.dispatch(setPendingApprovalForThread({ threadId: 't1', approval }));
    const { container } = render(
      <Provider store={store}>
        <ApprovalRequestCard threadId="t1" approval={approval} />
      </Provider>
    );
    await expectNoViolations(container);
  });
});
