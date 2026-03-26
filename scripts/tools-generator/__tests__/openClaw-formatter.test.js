/**
 * Unit tests for the OpenClaw formatter.
 * Tests markdown generation, tool formatting, and categorization.
 */
import { describe, expect, it } from 'vitest';

import {
  ENVIRONMENTS,
  formatParameters,
  generateOpenClawMarkdown,
  generateToolExample,
  groupToolsBySkill,
  TOOL_CATEGORIES,
} from '../openClaw-formatter.js';

describe('OpenClaw Formatter', () => {
  describe('formatParameters', () => {
    it('should format parameters correctly', () => {
      const schema = {
        type: 'object',
        properties: {
          required_param: { type: 'string', description: 'A required parameter' },
          optional_param: { type: 'number', description: 'An optional parameter' },
        },
        required: ['required_param'],
      };

      const result = formatParameters(schema);

      expect(result).toContain('**required_param** (string) **(required)**: A required parameter');
      expect(result).toContain('**optional_param** (number): An optional parameter');
    });

    it('should handle enum parameters', () => {
      const schema = {
        type: 'object',
        properties: {
          mode: { type: 'string', description: 'Selection mode', enum: ['auto', 'manual'] },
        },
      };

      const result = formatParameters(schema);

      expect(result).toContain('Options: `auto`, `manual`');
    });

    it('should handle empty parameters', () => {
      const result = formatParameters({});
      expect(result).toBe('- *None*');

      const result2 = formatParameters({ type: 'object', properties: {} });
      expect(result2).toBe('- *None*');
    });
  });

  describe('generateToolExample', () => {
    it('should generate example JSON for a tool', () => {
      const tool = {
        name: 'send_message',
        description: 'Send a message',
        skillId: 'telegram',
        inputSchema: {
          type: 'object',
          properties: {
            chat_id: { type: 'string', description: 'Chat ID' },
            message: { type: 'string', description: 'Message text' },
            count: { type: 'number', default: 5 },
          },
        },
      };

      const result = generateToolExample(tool);

      expect(result).toContain('"tool": "send_message"');
      expect(result).toContain('"chat_id": "example_chat_id"');
      expect(result).toContain('"message": "example_message"');
      expect(result).toContain('"count": 5');
    });

    it('should handle boolean and array types', () => {
      const tool = {
        name: 'test_tool',
        skillId: 'test',
        inputSchema: {
          type: 'object',
          properties: {
            enabled: { type: 'boolean', default: true },
            tags: { type: 'array' },
            config: { type: 'object' },
          },
        },
      };

      const result = generateToolExample(tool);

      expect(result).toContain('"enabled": true');
      expect(result).toContain('"tags": []');
      expect(result).toContain('"config": {}');
    });
  });

  describe('groupToolsBySkill', () => {
    it('should group tools by skill correctly', () => {
      const tools = [
        {
          skillId: 'telegram',
          name: 'send_message',
          description: 'Send message',
          inputSchema: { type: 'object', properties: {} },
        },
        {
          skillId: 'telegram',
          name: 'get_history',
          description: 'Get history',
          inputSchema: { type: 'object', properties: {} },
        },
        {
          skillId: 'gmail',
          name: 'send_email',
          description: 'Send email',
          inputSchema: { type: 'object', properties: {} },
        },
      ];

      const grouped = groupToolsBySkill(tools);

      expect(grouped).toHaveProperty('telegram');
      expect(grouped).toHaveProperty('gmail');
      expect(grouped.telegram.tools).toHaveLength(2);
      expect(grouped.gmail.tools).toHaveLength(1);
      expect(grouped.telegram.name).toBe('Telegram');
      expect(grouped.gmail.name).toBe('Gmail');
    });

    it('should categorize skills correctly', () => {
      const tools = [
        {
          skillId: 'telegram',
          name: 'send_message',
          description: 'Send message',
          inputSchema: { type: 'object', properties: {} },
        },
        {
          skillId: 'notion',
          name: 'create_page',
          description: 'Create page',
          inputSchema: { type: 'object', properties: {} },
        },
        {
          skillId: 'unknown_skill',
          name: 'unknown_tool',
          description: 'Unknown tool',
          inputSchema: { type: 'object', properties: {} },
        },
      ];

      const grouped = groupToolsBySkill(tools);

      expect(grouped.telegram.category).toBe('communication');
      expect(grouped.notion.category).toBe('productivity');
      expect(grouped.unknown_skill.category).toBe('utility');
    });
  });

  describe('generateOpenClawMarkdown', () => {
    it('should generate complete markdown documentation', () => {
      const tools = [
        {
          skillId: 'telegram',
          name: 'send_message',
          description: 'Send a message to a Telegram chat',
          inputSchema: {
            type: 'object',
            properties: {
              chat_id: { type: 'string', description: 'Chat ID' },
              message: { type: 'string', description: 'Message text' },
            },
            required: ['chat_id', 'message'],
          },
        },
      ];

      const result = generateOpenClawMarkdown(tools);

      // Check main sections
      expect(result).toContain('# OpenHuman Tools');
      expect(result).toContain('## Overview');
      expect(result).toContain('## Environment Configuration');
      expect(result).toContain('## Tool Categories');
      expect(result).toContain('## Available Tools');
      expect(result).toContain('## Tool Usage Guidelines');

      // Check content
      expect(result).toContain('**1 tools** across **1 integrations**');
      expect(result).toContain('### Telegram Tools');
      expect(result).toContain('#### send_message');
      expect(result).toContain('Send a message to a Telegram chat');

      // Check environments
      expect(result).toContain('### Development Environment');
      expect(result).toContain('### Production Environment');
      expect(result).toContain('### Testing Environment');

      // Check guidelines
      expect(result).toContain('### Authentication');
      expect(result).toContain('### Rate Limiting');
      expect(result).toContain('### Error Handling');
    });

    it('should handle empty tools list', () => {
      const result = generateOpenClawMarkdown([]);

      expect(result).toContain('**0 tools** across **0 integrations**');
      expect(result).toContain('## Available Tools');
      // Should still contain all standard sections
      expect(result).toContain('## Environment Configuration');
      expect(result).toContain('## Tool Usage Guidelines');
    });

    it('should include tool statistics', () => {
      const tools = [
        {
          skillId: 'telegram',
          name: 'send_message',
          description: 'Send message',
          inputSchema: { type: 'object', properties: {} },
        },
        {
          skillId: 'gmail',
          name: 'send_email',
          description: 'Send email',
          inputSchema: { type: 'object', properties: {} },
        },
      ];

      const result = generateOpenClawMarkdown(tools);

      expect(result).toContain('- Total Tools: 2');
      expect(result).toContain('- Active Skills: 2');
    });
  });

  describe('ENVIRONMENTS constant', () => {
    it('should have correct environment definitions', () => {
      expect(ENVIRONMENTS).toHaveProperty('development');
      expect(ENVIRONMENTS).toHaveProperty('production');
      expect(ENVIRONMENTS).toHaveProperty('testing');

      expect(ENVIRONMENTS.development.name).toBe('Development');
      expect(ENVIRONMENTS.production.accessLevel).toBe('Production-safe tools only');
      expect(ENVIRONMENTS.testing.rateLimits).toBe('Testing-specific limits');
    });
  });

  describe('TOOL_CATEGORIES constant', () => {
    it('should have correct category definitions', () => {
      expect(TOOL_CATEGORIES).toHaveProperty('communication');
      expect(TOOL_CATEGORIES).toHaveProperty('productivity');
      expect(TOOL_CATEGORIES).toHaveProperty('automation');
      expect(TOOL_CATEGORIES).toHaveProperty('data');
      expect(TOOL_CATEGORIES).toHaveProperty('media');
      expect(TOOL_CATEGORIES).toHaveProperty('utility');

      expect(TOOL_CATEGORIES.communication.skills).toContain('telegram');
      expect(TOOL_CATEGORIES.productivity.skills).toContain('notion');
      expect(TOOL_CATEGORIES.automation.skills).toContain('zapier');
    });
  });
});
