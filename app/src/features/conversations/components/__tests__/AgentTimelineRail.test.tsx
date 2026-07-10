import { render, screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';

import { agentNameTone, AgentTimelineRail } from '../AgentTimelineRail';

describe('agentNameTone', () => {
  it('pulses + mutes a running agent (in progress)', () => {
    const tone = agentNameTone('running');
    expect(tone).toContain('animate-pulse');
    expect(tone).toContain('text-content-faint');
  });

  it('pulses an awaiting-user agent', () => {
    expect(agentNameTone('awaiting_user')).toContain('animate-pulse');
  });

  it('renders a done agent solid (no pulse)', () => {
    const tone = agentNameTone('success');
    expect(tone).not.toContain('animate-pulse');
    expect(tone).toContain('text-content-secondary');
  });

  it('tints a failed agent with the error token', () => {
    expect(agentNameTone('error')).toContain('coral');
  });

  it('treats an unknown status as in-progress', () => {
    expect(agentNameTone(undefined)).toContain('animate-pulse');
  });

  it('renders a cancelled agent muted and static (terminal, not pulsing)', () => {
    const tone = agentNameTone('cancelled');
    expect(tone).not.toContain('animate-pulse');
    expect(tone).toContain('text-content-faint');
  });
});

describe('AgentTimelineRail', () => {
  it('renders the row content and a spark node', () => {
    render(
      <AgentTimelineRail isFirst isLast>
        <span>Research Agent</span>
      </AgentTimelineRail>
    );
    const row = screen.getByTestId('agent-timeline-row');
    expect(row.textContent).toContain('Research Agent');
    expect(row.querySelector('svg')).not.toBeNull();
  });

  it('omits the upper connector on the first row and the lower connector on the last', () => {
    const { container } = render(
      <AgentTimelineRail isFirst isLast>
        <span>only</span>
      </AgentTimelineRail>
    );
    // first+last single row → no connector segments at all
    expect(container.querySelectorAll('span[aria-hidden]')).toHaveLength(0);
  });

  it('draws both connectors on a middle row', () => {
    const { container } = render(
      <AgentTimelineRail isFirst={false} isLast={false}>
        <span>middle</span>
      </AgentTimelineRail>
    );
    expect(container.querySelectorAll('span[aria-hidden]')).toHaveLength(2);
  });
});
