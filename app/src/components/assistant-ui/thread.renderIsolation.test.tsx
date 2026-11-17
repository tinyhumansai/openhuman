import {
  AssistantRuntimeProvider,
  type ThreadMessageLike,
  useAuiState,
  useExternalStoreRuntime,
} from '@assistant-ui/react';
import { render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import { Thread, type ThreadComponents } from './thread';

const convertMessage = (message: ThreadMessageLike) => message;
const onNew = async () => {};
function Harness({
  messages,
  components,
  poll,
}: {
  messages: ThreadMessageLike[];
  components: ThreadComponents;
  poll: number;
}) {
  const runtime = useExternalStoreRuntime({ messages, convertMessage, onNew });
  return (
    <AssistantRuntimeProvider runtime={runtime}>
      <span>{poll}</span>
      <Thread
        components={{ ...components, ComposerHeader: () => <span>Composer refresh {poll}</span> }}
      />
    </AssistantRuntimeProvider>
  );
}
describe('conversation render isolation', () => {
  it('skips unchanged messages on a host refresh while continuing to render message updates', () => {
    const rendered = vi.fn();
    function Assistant() {
      const text = useAuiState(s =>
        s.message.content
          .filter(part => part.type === 'text')
          .map(part => part.text)
          .join('')
      );
      rendered();
      return <p>{text}</p>;
    }
    const components = { AssistantMessage: Assistant };
    const messages: ThreadMessageLike[] = [
      { id: 'reply', role: 'assistant', content: 'Original response' },
    ];
    const { rerender } = render(<Harness messages={messages} components={components} poll={0} />);
    expect(screen.getByText('Original response')).toBeVisible();
    rendered.mockClear();
    rerender(<Harness messages={messages} components={components} poll={1} />);
    expect(rendered).not.toHaveBeenCalled();
    rerender(
      <Harness
        messages={[{ ...messages[0]!, content: 'Updated response' }]}
        components={components}
        poll={2}
      />
    );
    expect(screen.getByText('Updated response')).toBeVisible();
    expect(rendered).toHaveBeenCalled();
  });
});
