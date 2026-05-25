import { render, screen, waitFor } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

vi.mock('../../../../lib/i18n/I18nContext', () => ({ useT: () => ({ t: (key: string) => key }) }));

vi.mock('../../hooks/useSettingsNavigation', () => ({
  useSettingsNavigation: () => ({ navigateBack: vi.fn(), breadcrumbs: [] }),
}));

vi.mock('../components/SettingsHeader', () => ({ default: () => null }));

vi.mock('../../../intelligence/MemoryTextWithEntities', () => ({
  MemoryTextWithEntities: ({ text }: { text: string }) => <span>{text}</span>,
}));

vi.mock('../../../../utils/tauriCommands', () => ({
  memoryClearNamespace: vi.fn().mockResolvedValue({}),
  memoryDeleteDocument: vi.fn().mockResolvedValue({}),
  memoryListDocuments: vi.fn().mockResolvedValue({ data: { documents: [] } }),
  memoryListNamespaces: vi.fn().mockResolvedValue([]),
  memoryQueryNamespace: vi.fn().mockResolvedValue({ matches: [] }),
  memoryRecallNamespace: vi.fn().mockResolvedValue({ matches: [] }),
}));

describe('MemoryDebugPanel stable test hooks', () => {
  it('renders the panel-level test id', async () => {
    const { default: MemoryDebugPanel } = await import('../MemoryDebugPanel');

    render(<MemoryDebugPanel />);

    expect(screen.getByTestId('memory-debug-panel')).toBeInTheDocument();
    await waitFor(() => expect(screen.getByText('memory.documents')).toBeInTheDocument());
  });
});
