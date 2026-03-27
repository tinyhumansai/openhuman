/**
 * Auto-update TOOLS.md with all available tools from the runtime
 *
 * This module provides functionality to automatically update the TOOLS.md file
 * with all tools discovered from the running skills system.
 */
import { invoke } from '@tauri-apps/api/core';

import { forceToolsCacheRefresh } from './file-watcher';

// Prevent excessive updates - limit to once per 10 seconds
let lastUpdateTime = 0;
const UPDATE_THROTTLE_MS = 10000;

interface RuntimeToolResponse {
  skillId: string;
  tool: {
    name: string;
    description: string;
    input_schema: { type: string; properties: Record<string, unknown>; required?: string[] };
  };
}

/**
 * Get all tools from the runtime and update TOOLS.md
 */
export async function updateToolsDocumentation(): Promise<void> {
  const now = Date.now();
  if (now - lastUpdateTime < UPDATE_THROTTLE_MS) {
    console.log('⏭️ TOOLS.md update skipped - throttled (last update was recent)');
    return;
  }
  lastUpdateTime = now;

  console.log('=== AUTO-UPDATE TOOLS.md START ===');

  try {
    console.log('🔍 Step 1: Calling runtime_all_tools command...');

    // Call the existing runtime command to get all tools
    const toolsResponse = await invoke<RuntimeToolResponse[]>('runtime_all_tools');

    console.log('🔧 Step 2: Analyzing response...');
    console.log('🔧 Raw response from runtime_all_tools:', toolsResponse);
    console.log('🔧 Response type:', typeof toolsResponse);
    console.log('🔧 Is array:', Array.isArray(toolsResponse));

    if (Array.isArray(toolsResponse)) {
      console.log(`🔧 Array length: ${toolsResponse.length}`);
      if (toolsResponse.length > 0) {
        console.log('🔧 First tool sample:', JSON.stringify(toolsResponse[0], null, 2));
      }
    }

    if (!toolsResponse || toolsResponse.length === 0) {
      console.warn('⚠️ Step 3: No tools discovered from runtime - STOPPING');
      console.warn('⚠️ This means runtime_all_tools returned empty/null');
      return;
    }

    console.log(`✅ Step 3: Discovered ${toolsResponse.length} tools from runtime`);

    // Transform the tools data into grouped format
    console.log('🔍 Step 4: Grouping tools by skill...');
    const toolsBySkill = groupToolsBySkill(toolsResponse);
    console.log(
      '🔧 Tools by skill:',
      Object.keys(toolsBySkill).map(skillId => `${skillId}: ${toolsBySkill[skillId].length} tools`)
    );

    // Generate the TOOLS.md content
    console.log('🔍 Step 5: Generating TOOLS.md markdown content...');
    const markdownContent = generateToolsMarkdown(toolsBySkill);
    console.log(`🔧 Generated markdown length: ${markdownContent.length} characters`);

    // Write to src-tauri/ai/TOOLS.md using Tauri command
    console.log('🔍 Step 6: Writing to src-tauri/ai/TOOLS.md file...');
    console.log('🔧 About to call write_ai_config_file with filename: TOOLS.md');
    console.log('🔧 Content length:', markdownContent.length);

    try {
      const writeResult = await invoke('write_ai_config_file', {
        filename: 'TOOLS.md',
        content: markdownContent,
      });
      console.log('🔧 Write command result:', writeResult);
    } catch (writeError) {
      console.error('❌ Write command failed:', writeError);
      console.error('❌ Error type:', typeof writeError);
      console.error('❌ Error toString:', writeError?.toString());
      throw new Error(`File write failed: ${writeError}`);
    }

    console.log('✅ SUCCESS: TOOLS.md updated successfully!');

    // Clear tools cache and force immediate refresh with new data
    console.log('🗑️ Clearing tools cache and refreshing with new data...');
    await forceToolsCacheRefresh();

    // Manually dispatch tools-updated event to ensure UI updates
    console.log('📡 Dispatching tools-updated event for UI components...');
    window.dispatchEvent(new CustomEvent('tools-updated', { detail: { timestamp: Date.now() } }));

    console.log(
      `📄 Final result: ${toolsResponse.length} tools from ${Object.keys(toolsBySkill).length} skills`
    );
    console.log('=== AUTO-UPDATE TOOLS.md COMPLETE ===');
  } catch (error) {
    console.error('❌ FATAL ERROR in updateToolsDocumentation:', error);
    console.error('❌ Error stack:', error instanceof Error ? error.stack : 'No stack trace');
    console.error('=== AUTO-UPDATE TOOLS.md FAILED ===');
  }
}

/**
 * Group tools by skill for organized documentation
 */
function groupToolsBySkill(
  toolsResponse: RuntimeToolResponse[]
): Record<string, RuntimeToolResponse[]> {
  const grouped: Record<string, RuntimeToolResponse[]> = {};

  for (const toolResponse of toolsResponse) {
    const skillId = toolResponse.skillId;
    if (!grouped[skillId]) {
      grouped[skillId] = [];
    }
    grouped[skillId].push(toolResponse);
  }

  return grouped;
}

/**
 * Generate markdown content for TOOLS.md
 */
function generateToolsMarkdown(toolsBySkill: Record<string, RuntimeToolResponse[]>): string {
  const totalTools = Object.values(toolsBySkill).flat().length;
  const skillCount = Object.keys(toolsBySkill).length;

  const content = [
    `# OpenHuman Tools

This document lists all available tools that OpenHuman can use to interact with external services and perform actions. Tools are organized by integration and automatically updated when the app loads.

## Overview

OpenHuman has access to **${totalTools} tools** across **${skillCount} integrations**.

**Quick Statistics:**
${Object.entries(toolsBySkill)
  .map(([skillId, tools]) => `- **${formatSkillName(skillId)}**: ${tools.length} tools`)
  .join('\n')}

## Available Tools
`,
  ];

  // Generate tool documentation for each skill
  for (const [skillId, tools] of Object.entries(toolsBySkill)) {
    content.push(`\n### ${formatSkillName(skillId)} Tools\n`);
    content.push(`This skill provides ${tools.length} tools for ${skillId} integration.\n`);

    for (const toolResponse of tools) {
      const tool = toolResponse.tool;
      content.push(`#### ${tool.name}\n`);
      content.push(`**Description**: ${tool.description}\n`);

      // Generate parameters documentation
      const properties = tool.input_schema?.properties || {};
      const required = tool.input_schema?.required || [];

      if (Object.keys(properties).length > 0) {
        content.push('**Parameters**:');
        for (const [paramName, paramDef] of Object.entries(properties)) {
          const isRequired = required.includes(paramName);
          const requiredText = isRequired ? ' **(required)**' : '';
          const def = paramDef as { type?: string; description?: string };
          const type = def.type || 'any';
          const description = def.description || 'No description';
          content.push(`- **${paramName}** (${type})${requiredText}: ${description}`);
        }
      } else {
        content.push('**Parameters**: *None*');
      }

      content.push('\n**Usage Context**: Available in all environments\n');

      // Generate example usage
      const exampleParams: Record<string, unknown> = {};
      for (const [paramName, paramDef] of Object.entries(properties)) {
        const type = (paramDef as { type?: string }).type;
        exampleParams[paramName] = getExampleValue(paramName, type);
      }

      content.push('**Example**:');
      content.push('```json');
      content.push(JSON.stringify({ tool: tool.name, parameters: exampleParams }, null, 2));
      content.push('```\n');

      content.push('---\n');
    }
  }

  // Add footer
  content.push(`
## Tool Usage Guidelines

### Authentication
- All tools require proper authentication setup through the Skills system
- OAuth credentials are managed securely and refreshed automatically
- API keys are stored encrypted in the application keychain

### Rate Limiting
- Tools automatically respect API rate limits of external services
- Intelligent retry logic handles temporary failures with exponential backoff

### Error Handling
- All tools return structured error responses with detailed information
- Network failures trigger automatic retry with configurable attempts

---

**Tool Statistics**
- Total Tools: ${totalTools}
- Active Skills: ${skillCount}
- Last Updated: ${new Date().toISOString()}

*This file was automatically generated when the app loaded.*
*Tools are discovered from the running V8 skills runtime.*`);

  return content.join('\n');
}

/**
 * Format skill ID into readable name
 */
function formatSkillName(skillId: string): string {
  return skillId
    .split(/[-_]/)
    .map(word => word.charAt(0).toUpperCase() + word.slice(1))
    .join(' ');
}

/**
 * Generate example value for a parameter
 */
function getExampleValue(paramName: string, type: string | undefined): unknown {
  switch (type) {
    case 'string':
      return `example_${paramName}`;
    case 'number':
      return 10;
    case 'boolean':
      return true;
    case 'array':
      return [];
    case 'object':
      return {};
    default:
      return `example_${paramName}`;
  }
}
