import { Fragment } from 'react';

import { splitAgentMessageIntoBubbles } from '../../../../utils/agentMessageBubbles';
import { AgentMessageBubble, AgentMessageText } from '../../components/AgentMessageBubble';
import type { AgentBubblePosition } from '../../utils/format';

/**
 * Renders one assistant message. In `text` view mode the whole message is one
 * markdown block; otherwise it is split into positioned bubbles via
 * `splitAgentMessageIntoBubbles` (matching the current render loop).
 */
export function AssistantMessageItem({
  content,
  viewMode = 'bubbles',
}: {
  content: string;
  viewMode?: 'text' | 'bubbles';
}) {
  if (viewMode === 'text') {
    return <AgentMessageText content={content} />;
  }
  const parts = splitAgentMessageIntoBubbles(content);
  return (
    <>
      {parts.map((segment, index) => {
        const position: AgentBubblePosition =
          parts.length === 1
            ? 'single'
            : index === 0
              ? 'first'
              : index === parts.length - 1
                ? 'last'
                : 'middle';
        return (
          <Fragment key={index}>
            <AgentMessageBubble content={segment} position={position} />
          </Fragment>
        );
      })}
    </>
  );
}
