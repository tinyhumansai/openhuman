import {
  AssistantRuntimeProvider,
  type ThreadMessageLike,
  useExternalStoreRuntime,
} from '@assistant-ui/react';
import { render, screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';

const messages: ThreadMessageLike[] = [
  { role: 'user', content: [{ type: 'text', text: 'hello' }] },
];

function Harness() {
  const runtime = useExternalStoreRuntime({
    messages,
    isRunning: false,
    convertMessage: (m: ThreadMessageLike) => m,
    onNew: async () => {},
  });
  return (
    <AssistantRuntimeProvider runtime={runtime}>
      <div data-testid="seam-ok">ok</div>
    </AssistantRuntimeProvider>
  );
}

describe('assistant-ui seam', () => {
  it('imports and renders under jsdom', () => {
    render(<Harness />);
    expect(screen.getByTestId('seam-ok')).toBeInTheDocument();
  });
});
