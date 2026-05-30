import { fireEvent, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import { renderWithProviders } from '../../test/test-utils';
import DiagramViewerTab, { buildDiagramImageUrl } from './DiagramViewerTab';

vi.mock('../../utils/tauriCommands/config', () => ({
  openhumanGetDashboardSettings: vi
    .fn()
    .mockResolvedValue({
      result: {
        diagram_viewer: {
          enabled: true,
          source_url: 'http://localhost:8787/workspace/diagrams/latest.png',
          refresh_interval_seconds: 10,
        },
      },
      logs: [],
    }),
}));

describe('buildDiagramImageUrl', () => {
  it('adds a cache-busting refresh parameter to absolute URLs', () => {
    expect(buildDiagramImageUrl('http://localhost:8787/latest.png?format=png', 4)).toBe(
      'http://localhost:8787/latest.png?format=png&openhuman_refresh=4'
    );
  });

  it('adds a cache-busting refresh parameter to relative URLs', () => {
    expect(buildDiagramImageUrl('/workspace/diagrams/latest.png', 2)).toBe(
      '/workspace/diagrams/latest.png?openhuman_refresh=2'
    );
  });
});

describe('DiagramViewerTab', () => {
  it('refreshes the diagram image URL on demand', async () => {
    renderWithProviders(<DiagramViewerTab />);

    const image = await screen.findByRole('img', {
      name: 'Latest generated OpenHuman architecture diagram',
    });
    expect(image).toHaveAttribute('src', expect.stringContaining('openhuman_refresh=0'));

    fireEvent.click(screen.getByRole('button', { name: 'Refresh diagram' }));

    expect(
      screen.getByRole('img', { name: 'Latest generated OpenHuman architecture diagram' })
    ).toHaveAttribute('src', expect.stringContaining('openhuman_refresh=1'));
  });

  it('shows an empty state instead of a broken image after load failure', async () => {
    renderWithProviders(<DiagramViewerTab />);

    const image = await screen.findByRole('img', {
      name: 'Latest generated OpenHuman architecture diagram',
    });
    fireEvent.error(image);

    expect(screen.getByText('No diagram available yet')).toBeInTheDocument();
    expect(
      screen.getByText('npx skills add yizhiyanhua-ai/fireworks-tech-graph')
    ).toBeInTheDocument();
    expect(
      screen.getByText(
        'Generate an architecture diagram of the current swarm in dark terminal style'
      )
    ).toBeInTheDocument();
    expect(
      screen.queryByRole('img', { name: 'Latest generated OpenHuman architecture diagram' })
    ).not.toBeInTheDocument();
  });
});
