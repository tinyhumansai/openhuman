import { describe, expect, it, vi } from 'vitest';

import { registry } from './registry';
import type { RegisteredAction } from './types';
import { slashCommandsFromActions } from './useSlashCommands';

describe('slashCommandsFromActions', () => {
  it('exposes only composer-enabled registry actions and executes the registry action', () => {
    const frame = Symbol('chat');
    const clear = vi.fn();
    registry.reset();
    registry.setActiveStack([frame]);
    registry.registerAction(
      {
        id: 'chat.new',
        label: 'New chat',
        handler: clear,
        slashCommand: { id: 'clear', descriptionKey: 'command.clear' },
      },
      frame
    );
    const actions = registry.getActiveActions([frame]) as RegisteredAction[];
    const commands = slashCommandsFromActions(actions, (key, fallback) => `${key}:${fallback}`);

    expect(commands).toHaveLength(1);
    expect(commands[0]).toMatchObject({ id: 'clear', description: 'command.clear:New chat' });
    commands[0]?.execute?.();
    expect(clear).toHaveBeenCalledOnce();
  });
});
