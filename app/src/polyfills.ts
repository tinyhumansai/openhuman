// biome-ignore lint/suspicious/noExplicitAny: intentional global polyfill assignments
// Polyfill Node.js globals for browser dependencies
// This must be imported FIRST before any other imports that use Node.js APIs

import { Buffer } from 'buffer';
import process from 'process';
import * as util from 'util';

// Immediately set Buffer on all global objects synchronously
// This must happen before any other code runs
(function setupNodePolyfills() {
  const buffer = Buffer;

  // Set Buffer on all global objects
  if (typeof globalThis !== 'undefined') {
    (globalThis as any).Buffer = buffer;
  }

  if (typeof window !== 'undefined') {
    (window as any).Buffer = buffer;
  }

  if (typeof global !== 'undefined') {
    (global as any).Buffer = buffer;
    (global as any).global = globalThis;
  }

  if (typeof self !== 'undefined') {
    (self as any).Buffer = buffer;
  }

  // Set process on global objects
  if (typeof globalThis !== 'undefined') {
    (globalThis as any).process = process;
    (globalThis as any).util = util;
  }

  if (typeof window !== 'undefined') {
    (window as any).process = process;
    (window as any).util = util;
  }

  if (typeof global !== 'undefined') {
    (global as any).process = process;
    (global as any).util = util;
  }
})();

// Export for use in modules
export { Buffer, process, util };
export default Buffer;
