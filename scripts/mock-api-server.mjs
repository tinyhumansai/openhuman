#!/usr/bin/env node
import process from "node:process";

import { DEFAULT_PORT, startMockServer, stopMockServer } from "./mock-api-core.mjs";

function usage() {
  return "Usage: node scripts/mock-api-server.mjs [--port <port>]";
}

function parsePortValue(value, label) {
  const port = Number(value);
  if (Number.isInteger(port) && port > 0 && port <= 65535) {
    return port;
  }
  throw new Error(`${label} must be an integer between 1 and 65535`);
}

function readPortArg() {
  const idx = process.argv.findIndex((arg) => arg === "--port" || arg === "-p");
  if (idx >= 0) {
    const value = process.argv[idx + 1];
    if (!value || value.startsWith("-")) {
      console.error("[mock-api-server] --port requires an integer between 1 and 65535");
      process.exit(2);
    }
    try {
      return parsePortValue(value, "--port");
    } catch (err) {
      console.error(`[mock-api-server] ${err.message}`);
      process.exit(2);
    }
  }
  if (process.argv.includes("--help") || process.argv.includes("-h")) {
    console.log(usage());
    process.exit(0);
  }
  const envPort = Number(process.env.MOCK_API_PORT || process.env.E2E_MOCK_PORT || DEFAULT_PORT);
  return Number.isInteger(envPort) && envPort > 0 ? envPort : DEFAULT_PORT;
}

async function main() {
  const port = readPortArg();
  await startMockServer(port);
  const shutdown = async () => {
    await stopMockServer();
    process.exit(0);
  };
  process.on("SIGINT", shutdown);
  process.on("SIGTERM", shutdown);
}

main().catch((err) => {
  console.error("[mock-api-server] Failed to start:", err);
  process.exit(1);
});
