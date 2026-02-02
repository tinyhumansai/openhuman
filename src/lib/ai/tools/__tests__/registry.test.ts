import { describe, expect, it } from 'vitest';

import { type AITool, ToolRegistry } from '../registry';

function createMockTool(name: string, response = 'ok'): AITool {
  return {
    definition: {
      name,
      description: `Mock ${name} tool`,
      parameters: { type: 'object', properties: {} },
    },
    execute: async () => ({ content: response }),
  };
}

describe('ToolRegistry', () => {
  it('should start empty', () => {
    const registry = new ToolRegistry();
    expect(registry.size).toBe(0);
    expect(registry.getDefinitions()).toHaveLength(0);
  });

  it('should register a tool', () => {
    const registry = new ToolRegistry();
    registry.register(createMockTool('test_tool'));
    expect(registry.size).toBe(1);
  });

  it('should get a tool by name', () => {
    const registry = new ToolRegistry();
    const tool = createMockTool('my_tool');
    registry.register(tool);
    expect(registry.get('my_tool')).toBe(tool);
  });

  it('should return undefined for unknown tool', () => {
    const registry = new ToolRegistry();
    expect(registry.get('nonexistent')).toBeUndefined();
  });

  it('should unregister a tool', () => {
    const registry = new ToolRegistry();
    registry.register(createMockTool('to_remove'));
    expect(registry.size).toBe(1);
    registry.unregister('to_remove');
    expect(registry.size).toBe(0);
    expect(registry.get('to_remove')).toBeUndefined();
  });

  it('should return all tool definitions', () => {
    const registry = new ToolRegistry();
    registry.register(createMockTool('tool_a'));
    registry.register(createMockTool('tool_b'));
    const defs = registry.getDefinitions();
    expect(defs).toHaveLength(2);
    expect(defs.map(d => d.name)).toContain('tool_a');
    expect(defs.map(d => d.name)).toContain('tool_b');
  });

  it('should execute a registered tool', async () => {
    const registry = new ToolRegistry();
    registry.register(createMockTool('exec_tool', 'result_value'));
    const result = await registry.execute('exec_tool', {});
    expect(result.content).toBe('result_value');
    expect(result.isError).toBeUndefined();
  });

  it('should return error for executing unknown tool', async () => {
    const registry = new ToolRegistry();
    const result = await registry.execute('unknown', {});
    expect(result.isError).toBe(true);
    expect(result.content).toContain('Unknown tool');
  });

  it('should catch tool execution errors', async () => {
    const registry = new ToolRegistry();
    const failingTool: AITool = {
      definition: { name: 'failing', description: 'A tool that fails', parameters: {} },
      execute: async () => {
        throw new Error('Intentional failure');
      },
    };
    registry.register(failingTool);
    const result = await registry.execute('failing', {});
    expect(result.isError).toBe(true);
    expect(result.content).toContain('Intentional failure');
  });

  it('should replace tool with same name on re-register', () => {
    const registry = new ToolRegistry();
    registry.register(createMockTool('tool', 'v1'));
    registry.register(createMockTool('tool', 'v2'));
    expect(registry.size).toBe(1);
  });
});
