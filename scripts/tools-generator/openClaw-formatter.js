#!/usr/bin/env node
/**
 * OpenClaw Framework Formatter
 *
 * Formats discovered tools into OpenClaw-compliant documentation
 * with professional presentation, examples, and usage guidelines.
 */

/**
 * Environment configurations for OpenClaw compliance
 */
export const ENVIRONMENTS = {
  development: {
    name: 'Development',
    description: 'Local development environment with full access',
    accessLevel: 'Full access to all tools',
    rateLimits: 'Relaxed for testing',
    authentication: 'Development credentials',
    logging: 'Verbose logging enabled',
  },
  production: {
    name: 'Production',
    description: 'Production environment with security restrictions',
    accessLevel: 'Production-safe tools only',
    rateLimits: 'Standard API limits enforced',
    authentication: 'Production credentials required',
    logging: 'Essential logs only',
  },
  testing: {
    name: 'Testing',
    description: 'Testing environment for automated validation',
    accessLevel: 'Safe tools for automated testing',
    rateLimits: 'Testing-specific limits',
    authentication: 'Test credentials',
    logging: 'Test execution logs',
  },
};

/**
 * Tool categories for better organization
 */
export const TOOL_CATEGORIES = {
  communication: {
    name: 'Communication',
    description: 'Tools for messaging, email, and social interaction',
    skills: ['telegram', 'gmail', 'discord', 'slack'],
  },
  productivity: {
    name: 'Productivity',
    description: 'Tools for task management, note-taking, and organization',
    skills: ['notion', 'todoist', 'calendar', 'trello'],
  },
  automation: {
    name: 'Automation',
    description: 'Tools for workflow automation and task scheduling',
    skills: ['zapier', 'ifttt', 'scheduler', 'webhook'],
  },
  data: {
    name: 'Data & Analytics',
    description: 'Tools for data processing, analysis, and storage',
    skills: ['database', 'csv', 'json', 'analytics'],
  },
  media: {
    name: 'Media & Content',
    description: 'Tools for image, video, and content processing',
    skills: ['image', 'video', 'audio', 'pdf'],
  },
  utility: {
    name: 'Utilities',
    description: 'General-purpose utility tools and helpers',
    skills: ['file', 'text', 'crypto', 'converter'],
  },
};

/**
 * Converts JSON Schema to markdown parameter documentation
 * @param {Object} schema - JSON Schema object
 * @returns {string} Formatted markdown for parameters
 */
export function formatParameters(schema) {
  if (!schema || !schema.properties) {
    return '- *None*';
  }

  const params = [];
  const required = schema.required || [];

  for (const [name, prop] of Object.entries(schema.properties)) {
    const isRequired = required.includes(name);
    const requiredMark = isRequired ? ' **(required)**' : '';
    const type = prop.type || 'any';
    const description = prop.description || 'No description available';

    let paramLine = `- **${name}** (${type})${requiredMark}: ${description}`;

    // Add enum values if present
    if (prop.enum) {
      paramLine += ` Options: ${prop.enum.map(v => `\`${v}\``).join(', ')}`;
    }

    // Add format information
    if (prop.format) {
      paramLine += ` (Format: ${prop.format})`;
    }

    // Add constraints
    if (prop.minLength || prop.maxLength) {
      const constraints = [];
      if (prop.minLength) constraints.push(`min: ${prop.minLength}`);
      if (prop.maxLength) constraints.push(`max: ${prop.maxLength}`);
      paramLine += ` [${constraints.join(', ')}]`;
    }

    params.push(paramLine);
  }

  return params.join('\n');
}

/**
 * Generates example usage for a tool
 * @param {Object} tool - Tool definition
 * @returns {string} Formatted example
 */
export function generateToolExample(tool) {
  const params = {};
  const schema = tool.inputSchema;

  if (schema && schema.properties) {
    // Generate example values for the first few parameters
    for (const [name, prop] of Object.entries(schema.properties)) {
      if (Object.keys(params).length >= 3) break; // Limit to 3 params for brevity

      let exampleValue;
      switch (prop.type) {
        case 'string':
          exampleValue = prop.enum ? prop.enum[0] : `example_${name}`;
          break;
        case 'number':
          exampleValue = prop.default || 10;
          break;
        case 'boolean':
          exampleValue = prop.default || true;
          break;
        case 'array':
          exampleValue = [];
          break;
        case 'object':
          exampleValue = {};
          break;
        default:
          exampleValue = `example_${name}`;
      }

      params[name] = exampleValue;
    }
  }

  return JSON.stringify({ tool: tool.name, parameters: params }, null, 2);
}

/**
 * Groups tools by skill for better organization
 * @param {Array} tools - Array of tool objects
 * @returns {Object} Grouped tools by skill
 */
export function groupToolsBySkill(tools) {
  const grouped = {};

  for (const tool of tools) {
    const skillId = tool.skillId;
    if (!grouped[skillId]) {
      grouped[skillId] = {
        skillId,
        name: formatSkillName(skillId),
        category: categorizeSkill(skillId),
        tools: [],
      };
    }
    grouped[skillId].tools.push(tool);
  }

  return grouped;
}

/**
 * Formats skill names for display
 * @param {string} skillId - Skill identifier
 * @returns {string} Formatted name
 */
function formatSkillName(skillId) {
  return skillId
    .split(/[-_]/)
    .map(word => word.charAt(0).toUpperCase() + word.slice(1))
    .join(' ');
}

/**
 * Categorizes a skill based on its ID
 * @param {string} skillId - Skill identifier
 * @returns {string} Category name
 */
function categorizeSkill(skillId) {
  for (const [category, config] of Object.entries(TOOL_CATEGORIES)) {
    if (config.skills.some(skill => skillId.includes(skill))) {
      return category;
    }
  }
  return 'utility';
}

/**
 * Generates environment configuration section
 * @returns {string} Environment documentation
 */
export function generateEnvironmentSection() {
  let section = '## Environment Configuration\n\n';
  section += 'Tools are available in different environments with varying capabilities:\n\n';

  for (const [envId, env] of Object.entries(ENVIRONMENTS)) {
    section += `### ${env.name} Environment\n\n`;
    section += `${env.description}\n\n`;
    section += `- **Access Level**: ${env.accessLevel}\n`;
    section += `- **Rate Limits**: ${env.rateLimits}\n`;
    section += `- **Authentication**: ${env.authentication}\n`;
    section += `- **Logging**: ${env.logging}\n\n`;
  }

  return section;
}

/**
 * Generates tool categories section
 * @param {Object} groupedTools - Tools grouped by skill
 * @returns {string} Categories documentation
 */
export function generateCategoriesSection(groupedTools) {
  const categoryCounts = {};

  // Count tools by category
  for (const skill of Object.values(groupedTools)) {
    const category = skill.category;
    if (!categoryCounts[category]) {
      categoryCounts[category] = { skills: [], toolCount: 0 };
    }
    categoryCounts[category].skills.push(skill.skillId);
    categoryCounts[category].toolCount += skill.tools.length;
  }

  let section = '## Tool Categories\n\n';

  for (const [categoryId, categoryConfig] of Object.entries(TOOL_CATEGORIES)) {
    const counts = categoryCounts[categoryId];
    if (!counts) continue;

    section += `### ${categoryConfig.name}\n\n`;
    section += `${categoryConfig.description}\n\n`;
    section += `- **Skills**: ${counts.skills.length}\n`;
    section += `- **Tools**: ${counts.toolCount}\n`;
    section += `- **Available Skills**: ${counts.skills.map(formatSkillName).join(', ')}\n\n`;
  }

  return section;
}

/**
 * Generates complete tools section with skills and tools
 * @param {Object} groupedTools - Tools grouped by skill
 * @returns {string} Tools documentation
 */
export function generateToolsSection(groupedTools) {
  const skillNames = Object.keys(groupedTools).sort();

  let section = '## Available Tools\n\n';

  for (const skillId of skillNames) {
    const skill = groupedTools[skillId];
    const categoryConfig = TOOL_CATEGORIES[skill.category];

    section += `### ${skill.name} Tools\n\n`;

    if (categoryConfig) {
      section += `**Category**: ${categoryConfig.name}\n\n`;
    }

    section += `This skill provides ${skill.tools.length} tool${skill.tools.length === 1 ? '' : 's'} for ${skillId} integration.\n\n`;

    for (const tool of skill.tools) {
      section += `#### ${tool.name}\n\n`;
      section += `**Description**: ${tool.description}\n\n`;
      section += `**Parameters**:\n${formatParameters(tool.inputSchema)}\n\n`;
      section += `**Usage Context**: Available in all environments\n\n`;
      section += `**Example**:\n\`\`\`json\n${generateToolExample(tool)}\n\`\`\`\n\n`;
      section += '---\n\n';
    }
  }

  return section;
}

/**
 * Generates usage guidelines section
 * @returns {string} Guidelines documentation
 */
export function generateGuidelinesSection() {
  return `## Tool Usage Guidelines

### Authentication
- All tools require proper authentication setup through the Skills system
- OAuth credentials are managed securely and refreshed automatically
- API keys are stored encrypted in the application keychain
- Test credentials are available for development and testing environments

### Rate Limiting
- Tools automatically respect API rate limits of external services
- Intelligent retry logic handles temporary failures with exponential backoff
- Bulk operations are automatically chunked to avoid hitting limits
- Rate limit status is monitored and reported in real-time

### Error Handling
- All tools return structured error responses with detailed information
- Network failures trigger automatic retry with configurable attempts
- Invalid parameters return clear validation messages with examples
- Tool execution timeouts are handled gracefully with partial results

### Security & Privacy
- Input validation is performed on all parameters using JSON Schema
- Output sanitization prevents injection attacks and data leakage
- Sensitive data is never logged or exposed in error messages
- All API communications use secure protocols (HTTPS/TLS)

### Performance Optimization
- Tool results are cached when appropriate to reduce API calls
- Parallel execution is used for independent operations
- Connection pooling minimizes overhead for repeated API calls
- Background sync keeps data fresh without blocking operations

### Monitoring & Observability
- Tool execution metrics are collected for performance analysis
- Error rates and response times are monitored continuously
- Debug logging is available in development environments
- Tool usage analytics help optimize integration performance

## Skill Management

Tools are provided by Skills, which are JavaScript modules running in a secure V8 runtime:

- **Discovery**: Tools are automatically discovered at build time from running skills
- **Lifecycle**: Skills can be enabled/disabled independently without affecting others
- **Configuration**: Each skill has its own configuration panel with setup wizards
- **Updates**: Skills are updated through Git submodules and the Skills management system
- **Security**: Skills run in sandboxed environments with limited system access

## Integration Architecture

### V8 Runtime
- Skills execute in isolated V8 JavaScript contexts on desktop platforms
- Mobile platforms use lightweight alternatives with server-side execution
- Memory limits and execution timeouts prevent resource exhaustion
- Inter-skill communication is managed through secure message passing

### API Bridge
- Tools communicate with external services through standardized API bridges
- Rate limiting, retry logic, and error handling are implemented at the bridge level
- Authentication tokens are managed centrally and shared across tools
- Response caching and optimization are handled transparently

### Data Flow
1. Tool request received from AI agent
2. Input validation and parameter processing
3. Skill execution in secure V8 context
4. API calls through standardized bridges
5. Response processing and formatting
6. Result delivery to AI agent

`;
}

/**
 * Generates footer section with metadata
 * @param {Array} tools - Array of all tools
 * @returns {string} Footer content
 */
export function generateFooter(tools) {
  const skillCount = new Set(tools.map(t => t.skillId)).size;

  return `## Support & Troubleshooting

### Common Issues
1. **Tool Not Available**: Check if the associated skill is enabled in Settings → Skills
2. **Authentication Errors**: Verify credentials in the skill's configuration panel
3. **Rate Limit Exceeded**: Wait for the limit to reset or upgrade your API plan
4. **Invalid Parameters**: Review the parameter documentation and examples above

### Getting Help
- **Skill Documentation**: Each skill has detailed setup and usage instructions
- **Debug Logs**: Enable verbose logging in development mode for detailed error information
- **Community Support**: Join our Discord community for help from other users
- **Technical Support**: Contact our support team for critical issues

### Contributing
- **New Tools**: Submit tool requests through our GitHub repository
- **Bug Reports**: Report issues with specific tools and include error logs
- **Improvements**: Suggest enhancements to existing tools and their documentation

---

**Tool Statistics**
- Total Tools: ${tools.length}
- Active Skills: ${skillCount}
- Categories: ${Object.keys(TOOL_CATEGORIES).length}
- Last Updated: ${new Date().toISOString()}

*This file was automatically generated at build time from the V8 skills runtime.*
*For the most up-to-date information, regenerate this file by running \`yarn tools:generate\`.*
`;
}

/**
 * Generates complete OpenClaw-compliant TOOLS.md content
 * @param {Array} tools - Array of discovered tools
 * @returns {string} Complete TOOLS.md content
 */
export function generateOpenClawMarkdown(tools) {
  const grouped = groupToolsBySkill(tools);
  const skillNames = Object.keys(grouped);

  let content = `# OpenHuman Tools

This document lists all available tools that OpenHuman can use to interact with external services and perform actions. Tools are organized by integration and automatically updated during build time.

## Overview

OpenHuman has access to **${tools.length} tools** across **${skillNames.length} integrations** organized into **${Object.keys(TOOL_CATEGORIES).length} categories**.

**Quick Statistics:**
${skillNames.map(skill => `- **${grouped[skill].name}**: ${grouped[skill].tools.length} tools`).join('\n')}

`;

  content += generateEnvironmentSection();
  content += generateCategoriesSection(grouped);
  content += generateToolsSection(grouped);
  content += generateGuidelinesSection();
  content += generateFooter(tools);

  return content;
}
