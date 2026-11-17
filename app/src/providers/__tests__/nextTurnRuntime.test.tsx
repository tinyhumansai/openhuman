import {
  AssistantRuntimeProvider,
  ThreadPrimitive,
  useAuiState,
  useExternalStoreRuntime,
} from '@assistant-ui/react';
import { render, screen } from '@testing-library/react';
import { useMemo } from 'react';
import { describe, expect, it } from 'vitest';

import type { ThreadMessage } from '../../types/thread';
import { buildRuntimeMessages } from '../assistantUiMessages';

const convertMessage = (message: ReturnType<typeof buildRuntimeMessages>[number]) => message;
const onNew = async () => {};
const row = (
  id: string,
  sender: 'user' | 'agent',
  content: string,
  requestId?: string
): ThreadMessage => ({
  id,
  sender,
  content,
  type: 'text',
  createdAt: '2026-10-10T00:00:00Z',
  extraMetadata: requestId ? { requestId } : {},
});
function Message() {
  const id = useAuiState(s => s.message.id);
  const branches = useAuiState(s => s.message.branchCount);
  return <span data-testid={id}>{branches}</span>;
}
function State() {
  const ids = useAuiState(s => s.thread.messages.map(message => message.id).join(','));
  return (
    <>
      <output data-testid="order">{ids}</output>
      <ThreadPrimitive.Messages>{() => <Message />}</ThreadPrimitive.Messages>
    </>
  );
}
function Harness({ history, requestId }: { history: ThreadMessage[]; requestId?: string }) {
  const messages = useMemo(
    () =>
      buildRuntimeMessages(
        history,
        requestId ? { requestId, content: 'Streaming reply', thinking: '' } : null,
        { isRunning: Boolean(requestId), liveRequestId: requestId }
      ),
    [history, requestId]
  );
  const runtime = useExternalStoreRuntime({
    messages,
    convertMessage,
    onNew,
    isRunning: Boolean(requestId),
  });
  return (
    <AssistantRuntimeProvider runtime={runtime}>
      <State />
    </AssistantRuntimeProvider>
  );
}
describe('assistant-ui next-turn repository', () => {
  it('keeps the reply identity on settlement and cannot reparent a previous tail into the next turn', () => {
    const history = [row('user-1', 'user', 'first')];
    const { rerender } = render(<Harness history={history} requestId="r1" />);
    expect(screen.getByTestId('order')).toHaveTextContent('user-1,agent:r1');
    const settled = [...history, row('agent:r1', 'agent', 'First final reply', 'r1')];
    rerender(<Harness history={settled} />);
    expect(screen.getByTestId('agent:r1')).toHaveTextContent('1');
    rerender(<Harness history={[...settled, row('user-2', 'user', 'try now')]} requestId="r2" />);
    expect(screen.getByTestId('order')).toHaveTextContent('user-1,agent:r1,user-2,agent:r2');
    expect(screen.getByTestId('agent:r1')).toHaveTextContent('1');
    expect(screen.getByTestId('agent:r2')).toHaveTextContent('1');
  });
});
