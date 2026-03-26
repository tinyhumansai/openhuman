#!/usr/bin/env node
/**
 * Tauri Integration for Tools Discovery
 *
 * Provides integration utilities for discovering tools via Tauri runtime.
 * Handles cross-platform execution, error handling, and fallbacks.
 */
import { spawn } from 'child_process';
import { join } from 'path';
import { dirname } from 'path';
import { fileURLToPath } from 'url';

const __dirname = dirname(fileURLToPath(import.meta.url));
const PROJECT_ROOT = join(__dirname, '../..');

/**
 * Platform-specific command detection
 * @returns {Object} Command and arguments for spawning Tauri process
 */
export function getTauriCommand() {
  const isWindows = process.platform === 'win32';

  return {
    command: isWindows ? 'cargo.exe' : 'cargo',
    args: [
      'run',
      '--manifest-path',
      join(PROJECT_ROOT, 'src-tauri', 'Cargo.toml'),
      '--bin',
      'openhuman-tools-discovery',
    ],
  };
}

/**
 * Validates if Tauri development environment is available
 * @returns {Promise<boolean>} True if Tauri can be used
 */
export async function validateTauriEnvironment() {
  return new Promise(resolve => {
    const { command } = getTauriCommand();

    const child = spawn(command, ['--version'], { stdio: ['pipe', 'pipe', 'pipe'] });

    child.on('close', code => {
      resolve(code === 0);
    });

    child.on('error', () => {
      resolve(false);
    });

    // Timeout after 10 seconds
    setTimeout(() => {
      child.kill();
      resolve(false);
    }, 10000);
  });
}

/**
 * Executes tools discovery via Tauri runtime
 * @param {Object} options - Configuration options
 * @returns {Promise<Array>} Discovered tools
 */
export async function executeTauriDiscovery(options = {}) {
  const {
    timeout = 45000, // 45 seconds
    retries = 2,
    verbose = false,
  } = options;

  for (let attempt = 1; attempt <= retries; attempt++) {
    try {
      if (verbose) {
        console.log(`🔄 Tauri discovery attempt ${attempt}/${retries}...`);
      }

      const result = await runTauriDiscovery(timeout, verbose);
      return result;
    } catch (error) {
      if (attempt === retries) {
        throw error;
      }

      if (verbose) {
        console.warn(`⚠️  Attempt ${attempt} failed:`, error.message);
        console.log('🔄 Retrying...');
      }
    }
  }
}

/**
 * Internal function to run Tauri discovery process
 * @param {number} timeout - Timeout in milliseconds
 * @param {boolean} verbose - Enable verbose logging
 * @returns {Promise<Array>} Discovered tools
 */
async function runTauriDiscovery(timeout, verbose) {
  return new Promise((resolve, reject) => {
    const { command, args } = getTauriCommand();

    if (verbose) {
      console.log(`🔧 Executing: ${command} ${args.join(' ')}`);
    }

    const child = spawn(command, args, {
      stdio: ['pipe', 'pipe', 'pipe'],
      cwd: PROJECT_ROOT,
      env: {
        ...process.env,
        TAURI_TOOLS_DISCOVERY: 'true',
        RUST_LOG: verbose ? 'debug' : 'warn',
        RUST_BACKTRACE: '1',
      },
    });

    let output = '';
    let errorOutput = '';

    child.stdout.on('data', data => {
      const text = data.toString();
      output += text;

      if (verbose && text.trim()) {
        console.log('📤 Tauri output:', text.trim());
      }
    });

    child.stderr.on('data', data => {
      const text = data.toString();
      errorOutput += text;

      if (verbose && text.trim()) {
        console.log('📤 Tauri stderr:', text.trim());
      }
    });

    child.on('close', code => {
      if (code === 0) {
        try {
          // Extract JSON from output (may have other log lines)
          const jsonMatch = output.match(/\{.*"success".*\}/s);
          if (!jsonMatch) {
            reject(new Error('No valid JSON found in Tauri output'));
            return;
          }

          const result = JSON.parse(jsonMatch[0]);

          if (result.success) {
            resolve(result.tools || []);
          } else {
            reject(new Error(result.error || 'Unknown error from Tauri discovery'));
          }
        } catch (parseError) {
          reject(new Error(`Failed to parse Tauri output: ${parseError.message}`));
        }
      } else {
        const errorMsg = errorOutput.trim() || `Process exited with code ${code}`;
        reject(new Error(`Tauri discovery failed: ${errorMsg}`));
      }
    });

    child.on('error', error => {
      reject(new Error(`Failed to spawn Tauri process: ${error.message}`));
    });

    // Timeout handling
    const timeoutId = setTimeout(() => {
      child.kill('SIGTERM');

      // Force kill after 5 more seconds
      setTimeout(() => {
        if (!child.killed) {
          child.kill('SIGKILL');
        }
      }, 5000);

      reject(new Error(`Tauri discovery timed out after ${timeout}ms`));
    }, timeout);

    child.on('close', () => {
      clearTimeout(timeoutId);
    });
  });
}

/**
 * Prepares the environment for tools discovery
 * Ensures build dependencies and environment are ready
 * @returns {Promise<void>}
 */
export async function prepareTauriEnvironment() {
  console.log('🔧 Preparing Tauri environment for tools discovery...');

  // Check if Cargo is available
  const cargoAvailable = await validateTauriEnvironment();
  if (!cargoAvailable) {
    throw new Error('Cargo/Rust toolchain not found. Please install Rust and Cargo.');
  }

  console.log('✅ Tauri environment ready');
}

/**
 * Gets information about the current Tauri setup
 * @returns {Promise<Object>} Environment information
 */
export async function getTauriEnvironmentInfo() {
  const cargoAvailable = await validateTauriEnvironment();

  return {
    cargoAvailable,
    platform: process.platform,
    architecture: process.arch,
    projectRoot: PROJECT_ROOT,
    command: getTauriCommand(),
  };
}
