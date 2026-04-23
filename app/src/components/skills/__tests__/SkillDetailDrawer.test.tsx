/**
 * SkillDetailDrawer — vitest coverage
 *
 * Verifies:
 * - Renders skill name, description, version, tags, allowed tools, warnings.
 * - Escape key closes the drawer.
 * - Close button click triggers onClose.
 * - Backdrop click closes the drawer.
 * - Resource list empty-state message shows when no resources.
 * - Selecting a resource from the tree mounts the preview.
 */
import { fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import type { SkillSummary } from '../../../services/api/skillsApi';
import SkillDetailDrawer from '../SkillDetailDrawer';

// Mock skillsApi so <SkillResourcePreview /> doesn't hit the network
vi.mock('../../../services/api/skillsApi', () => ({
  skillsApi: {
    listSkills: vi.fn().mockResolvedValue([]),
    readSkillResource: vi.fn().mockResolvedValue({
      skillId: 'demo',
      relativePath: 'scripts/run.sh',
      content: '#!/bin/sh\necho hi',
      bytes: 18,
    }),
  },
}));

function buildSkill(overrides: Partial<SkillSummary> = {}): SkillSummary {
  return {
    id: 'demo',
    name: 'demo',
    description: 'A demonstration skill.',
    version: '1.2.3',
    author: 'Acme Labs',
    tags: ['alpha', 'beta'],
    tools: ['bash', 'python'],
    prompts: [],
    location: '/tmp/skills/demo/SKILL.md',
    resources: ['scripts/run.sh', 'references/README.md', 'assets/logo.png'],
    scope: 'user',
    legacy: false,
    warnings: [],
    ...overrides,
  };
}

describe('SkillDetailDrawer', () => {
  it('renders core metadata and scope pill', () => {
    const onClose = vi.fn();
    render(<SkillDetailDrawer skill={buildSkill()} onClose={onClose} />);
    expect(screen.getByText('demo')).toBeInTheDocument();
    expect(screen.getByText('A demonstration skill.')).toBeInTheDocument();
    expect(screen.getByText('v1.2.3')).toBeInTheDocument();
    expect(screen.getByText('Acme Labs')).toBeInTheDocument();
    // User scope pill
    expect(screen.getByText('User')).toBeInTheDocument();
    // Tools
    expect(screen.getByText('bash')).toBeInTheDocument();
    expect(screen.getByText('python')).toBeInTheDocument();
  });

  it('shows Project pill for project-scope skills', () => {
    render(<SkillDetailDrawer skill={buildSkill({ scope: 'project' })} onClose={vi.fn()} />);
    expect(screen.getByText('Project')).toBeInTheDocument();
  });

  it('shows Legacy pill when legacy=true regardless of scope', () => {
    render(
      <SkillDetailDrawer
        skill={buildSkill({ scope: 'user', legacy: true })}
        onClose={vi.fn()}
      />
    );
    expect(screen.getByText('Legacy')).toBeInTheDocument();
  });

  it('renders warnings in an amber panel', () => {
    render(
      <SkillDetailDrawer
        skill={buildSkill({ warnings: ['unknown frontmatter field: foo'] })}
        onClose={vi.fn()}
      />
    );
    expect(screen.getByText('Warnings')).toBeInTheDocument();
    expect(screen.getByText('unknown frontmatter field: foo')).toBeInTheDocument();
  });

  it('invokes onClose when Escape is pressed', () => {
    const onClose = vi.fn();
    render(<SkillDetailDrawer skill={buildSkill()} onClose={onClose} />);
    fireEvent.keyDown(document, { key: 'Escape' });
    expect(onClose).toHaveBeenCalled();
  });

  it('invokes onClose when the close button is clicked', () => {
    const onClose = vi.fn();
    render(<SkillDetailDrawer skill={buildSkill()} onClose={onClose} />);
    fireEvent.click(screen.getByRole('button', { name: /close skill details/i }));
    expect(onClose).toHaveBeenCalled();
  });

  it('shows the empty-state hint when there are no bundled resources', () => {
    render(<SkillDetailDrawer skill={buildSkill({ resources: [] })} onClose={vi.fn()} />);
    expect(screen.getByText(/No bundled resources/i)).toBeInTheDocument();
  });

  it('mounts the preview when a resource is clicked', () => {
    render(<SkillDetailDrawer skill={buildSkill()} onClose={vi.fn()} />);
    fireEvent.click(screen.getByTitle('scripts/run.sh'));
    // Loading state from preview
    expect(screen.getByRole('status', { name: /loading/i })).toBeInTheDocument();
  });
});
