/// <reference types="vite/client" />

// Node.js polyfills for browser
declare global {
  interface Window {
    Buffer: typeof Buffer;
    process: typeof process;
    util: typeof import('util');
  }
  var Buffer: typeof import('buffer').Buffer;
  var process: typeof import('process');
  var util: typeof import('util');
}
