#!/usr/bin/env node
/**
 * OpenHuman Tools Discovery Script
 *
 * Discovers all available tools from the V8 skills runtime and generates
 * a comprehensive TOOLS.md file following OpenClaw framework standards.
 *
 * Usage: node scripts/tools-generator/discover-tools.js
 */
import { existsSync, mkdirSync, writeFileSync } from 'fs';
import { dirname, join } from 'path';
import { fileURLToPath } from 'url';

import { generateOpenClawMarkdown } from './openClaw-formatter.js';
import {
  executeTauriDiscovery,
  getTauriEnvironmentInfo,
  prepareTauriEnvironment,
  validateTauriEnvironment,
} from './tauri-integration.js';

const __dirname = dirname(fileURLToPath(import.meta.url));
const PROJECT_ROOT = join(__dirname, '../..');
const AI_DIR = join(PROJECT_ROOT, 'ai');
const TOOLS_OUTPUT = join(AI_DIR, 'TOOLS.md');

// Environment categories for OpenClaw compatibility
const ENVIRONMENTS = {
  development: {
    name: 'Development',
    description: 'Local development environment with full access',
  },
  production: {
    name: 'Production',
    description: 'Production environment with security restrictions',
  },
  testing: { name: 'Testing', description: 'Testing environment for automated validation' },
};

/**
 * Discovers available tools from V8 skills runtime or fallback sources
 * @returns {Promise<Array>} Array of discovered tools with skill metadata
 */
async function discoverTools() {
  console.log('🔍 Discovering tools from V8 skills runtime...');

  // Check if Tauri environment is available
  const tauriAvailable = await validateTauriEnvironment();

  if (tauriAvailable) {
    try {
      console.log('🔧 Preparing Tauri environment...');
      await prepareTauriEnvironment();

      console.log('🚀 Executing Tauri tools discovery...');
      const realTools = await executeTauriDiscovery({
        timeout: 60000, // 60 seconds
        retries: 2,
        verbose: process.env.VERBOSE === 'true',
      });

      if (realTools && realTools.length > 0) {
        console.log(
          `✅ Discovered ${realTools.length} tools from ${new Set(realTools.map(t => t.skillId)).size} skills via Tauri`
        );
        return realTools;
      }
    } catch (error) {
      console.warn('⚠️  Could not discover tools from Tauri runtime:', error.message);
      console.log('📋 Using development mock data instead');
    }
  } else {
    console.warn('⚠️  Tauri environment not available');
    console.log('📋 Using development mock data instead');
  }

  // Fallback to mock data for development
  const mockTools = generateMockToolsForDevelopment();
  console.log(
    `✅ Using mock data: ${mockTools.length} tools from ${new Set(mockTools.map(t => t.skillId)).size} skills`
  );
  return mockTools;
}

/**
 * Generates mock tools data for development (until Tauri integration is complete)
 * This simulates the structure returned by runtime_all_tools()
 */
function generateMockToolsForDevelopment() {
  return [
    {
      skillId: 'telegram',
      name: 'send_message',
      description: 'Send a message to a Telegram chat or user',
      inputSchema: {
        type: 'object',
        properties: {
          chat_id: { type: 'string', description: 'Telegram chat ID or username' },
          message: { type: 'string', description: 'Message text to send' },
          parse_mode: {
            type: 'string',
            enum: ['HTML', 'Markdown'],
            description: 'Message formatting mode',
          },
        },
        required: ['chat_id', 'message'],
      },
    },
    {
      skillId: 'telegram',
      name: 'get_chat_history',
      description: 'Retrieve message history from a Telegram chat',
      inputSchema: {
        type: 'object',
        properties: {
          chat_id: { type: 'string', description: 'Telegram chat ID or username' },
          limit: { type: 'number', description: 'Number of messages to retrieve (max 100)' },
          offset: { type: 'number', description: 'Offset for pagination' },
        },
        required: ['chat_id'],
      },
    },
    {
      skillId: 'notion',
      name: 'create_page',
      description: 'Create a new page in Notion workspace',
      inputSchema: {
        type: 'object',
        properties: {
          parent_id: { type: 'string', description: 'Parent database or page ID' },
          title: { type: 'string', description: 'Page title' },
          content: { type: 'array', description: 'Page content blocks' },
          properties: { type: 'object', description: 'Page properties for database pages' },
        },
        required: ['parent_id', 'title'],
      },
    },
    {
      skillId: 'gmail',
      name: 'send_email',
      description: 'Send an email via Gmail',
      inputSchema: {
        type: 'object',
        properties: {
          to: { type: 'string', description: 'Recipient email address' },
          subject: { type: 'string', description: 'Email subject line' },
          body: { type: 'string', description: 'Email body content' },
          attachments: { type: 'array', description: 'File attachments' },
        },
        required: ['to', 'subject', 'body'],
      },
    },
  ];
}

// Removed duplicate functions - now using openClaw-formatter.js

/**
 * Main execution function
 */
async function main() {
  try {
    console.log('🚀 Starting OpenHuman tools discovery...');

    // Discover all available tools
    const tools = await discoverTools();

    if (tools.length === 0) {
      console.warn(
        '⚠️  No tools discovered. This might indicate an issue with the skills runtime.'
      );
    }

    // Ensure AI directory exists
    if (!existsSync(AI_DIR)) {
      console.log(`📁 Creating AI directory: ${AI_DIR}`);
      mkdirSync(AI_DIR, { recursive: true });
    }

    // Generate OpenClaw-compliant markdown
    console.log('📝 Generating OpenClaw-compliant TOOLS.md content...');
    const markdownContent = generateOpenClawMarkdown(tools);

    // Write to output file
    console.log(`💾 Writing TOOLS.md to: ${TOOLS_OUTPUT}`);
    writeFileSync(TOOLS_OUTPUT, markdownContent, 'utf8');

    console.log('✅ TOOLS.md generated successfully!');
    console.log(
      `📊 Generated documentation for ${tools.length} tools across ${new Set(tools.map(t => t.skillId)).size} skills`
    );
  } catch (error) {
    console.error('❌ Error generating TOOLS.md:', error.message);
    process.exit(1);
  }
}

/**
 * Attempts to discover tools from a running Tauri process
 * @returns {Promise<Array>} Array of tools from Tauri runtime
 */
async function discoverToolsFromTauri() {
  return new Promise((resolve, reject) => {
    // Try to spawn a minimal Tauri process for tool discovery
    const isWindows = process.platform === 'win32';
    const tauriCommand = isWindows ? 'cargo.exe' : 'cargo';

    const args = [
      'run',
      '--manifest-path',
      join(PROJECT_ROOT, 'src-tauri', 'Cargo.toml'),
      '--bin',
      'alphahuman-tools-discovery',
    ];

    console.log('🔧 Attempting to run tools discovery via Cargo...');

    const child = spawn(tauriCommand, args, {
      stdio: ['pipe', 'pipe', 'pipe'],
      cwd: PROJECT_ROOT,
      env: { ...process.env, TAURI_TOOLS_DISCOVERY: 'true' },
    });

    let output = '';
    let errorOutput = '';

    child.stdout.on('data', data => {
      output += data.toString();
    });

    child.stderr.on('data', data => {
      errorOutput += data.toString();
    });

    child.on('close', code => {
      if (code === 0 && output.trim()) {
        try {
          const result = JSON.parse(output.trim());
          if (result.success && result.tools) {
            resolve(result.tools);
          } else {
            reject(new Error(result.error || 'Unknown error from Tauri'));
          }
        } catch (parseError) {
          reject(new Error(`Failed to parse Tauri output: ${parseError.message}`));
        }
      } else {
        reject(new Error(`Tauri process failed (code ${code}): ${errorOutput}`));
      }
    });

    child.on('error', error => {
      reject(new Error(`Failed to spawn Tauri process: ${error.message}`));
    });

    // Timeout after 30 seconds
    setTimeout(() => {
      child.kill();
      reject(new Error('Tauri discovery process timed out'));
    }, 30000);
  });
}

// Run if called directly
if (import.meta.url === `file://${process.argv[1]}`) {
  main();
}

export { discoverTools, discoverToolsFromTauri, generateMockToolsForDevelopment };
