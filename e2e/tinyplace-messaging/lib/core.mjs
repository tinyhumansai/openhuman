// Boot a real `openhuman-core` process and drive its tiny.place messaging RPCs.
//
// openhuman is a Rust core exposing JSON-RPC over `POST /rpc`. All tiny.place
// messaging lives in the core's `tinyplace` domain (methods
// `openhuman.tinyplace_*`), backed by the vendored Rust tiny.place SDK. The
// core derives its tiny.place identity (a base58 Solana cryptoId) from the
// wallet mnemonic at m/44'/501'/0'/0'. So: one core process = one identity, and
// two cores with two mnemonics = two agents that can message each other over a
// real tiny.place backend — exactly what these tests exercise.
import { spawn } from "node:child_process";
import { mkdtempSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

import { generateMnemonic } from "./mnemonic.mjs";

const HERE = resolve(fileURLToPath(import.meta.url), "..");
const REPO_ROOT = resolve(HERE, "..", "..", "..");

export const DEFAULT_CORE_BIN =
  process.env.OPENHUMAN_CORE_BIN || join(REPO_ROOT, "target", "debug", "openhuman-core");
export const DEFAULT_BACKEND =
  process.env.TINYPLACE_API_BASE_URL || "http://localhost:18080";

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

// The tiny.place identity is re-derived from the mnemonic, so these per-chain
// account addresses are cosmetic metadata; wallet_setup only validates that
// each supported chain appears once with a non-empty address + derivation path.
// The Solana derivation path must be the canonical one the signer uses.
const PLACEHOLDER_ACCOUNTS = [
  { chain: "evm", address: "0x0000000000000000000000000000000000000001", derivationPath: "m/44'/60'/0'/0/0" },
  { chain: "btc", address: "bc1qplaceholderplaceholderplaceholderplac0000", derivationPath: "m/84'/0'/0'/0/0" },
  { chain: "solana", address: "11111111111111111111111111111111", derivationPath: "m/44'/501'/0'/0'" },
  { chain: "tron", address: "T0000000000000000000000000000000001", derivationPath: "m/44'/195'/0'/0/0" },
];

function makeRpc(port, token, label) {
  return async function rpc(method, params = {}) {
    let res;
    try {
      res = await fetch(`http://127.0.0.1:${port}/rpc`, {
        method: "POST",
        headers: { "content-type": "application/json", authorization: `Bearer ${token}` },
        body: JSON.stringify({ jsonrpc: "2.0", id: Date.now(), method, params }),
      });
    } catch (e) {
      throw new Error(`[${label}] ${method} transport error: ${e.message}`);
    }
    const body = await res.json();
    if (body.error) {
      throw new Error(`[${label}] ${method} -> ${JSON.stringify(body.error)}`);
    }
    // The core wraps every handler payload as { logs, result }; unwrap it.
    let result = body.result;
    if (result && typeof result === "object" && "result" in result && "logs" in result) {
      result = result.result;
    }
    return result;
  };
}

/**
 * Launch an openhuman core, import a fresh wallet identity, publish Signal
 * pre-keys, and advertise the encryption key on the directory card — i.e. a
 * fully message-ready tiny.place agent.
 *
 * Returns a handle: { name, cryptoId, rpc, port, workspace, stop() }.
 */
export async function launchAgent(name, opts = {}) {
  const {
    port,
    backend = DEFAULT_BACKEND,
    coreBin = DEFAULT_CORE_BIN,
    mnemonic = generateMnemonic(),
    preKeyCount = 10,
    verbose = !!process.env.VERBOSE,
    readyTimeoutMs = 45_000,
  } = opts;
  if (!port) throw new Error("launchAgent requires an explicit port");

  const token = `oh-${name}-${process.pid}-token`;
  const workspace = mkdtempSync(join(tmpdir(), `oh-${name}-`));
  const child = spawn(
    coreBin,
    ["run", "--host", "127.0.0.1", "--port", String(port), "--jsonrpc-only"],
    {
      env: {
        ...process.env,
        OPENHUMAN_WORKSPACE: workspace,
        OPENHUMAN_KEYRING_BACKEND: "file",
        OPENHUMAN_CORE_TOKEN: token,
        TINYPLACE_API_BASE_URL: backend,
      },
      stdio: ["ignore", "pipe", "pipe"],
    },
  );
  const logs = [];
  const capture = (stream, tag) =>
    stream.on("data", (d) => {
      const line = d.toString();
      logs.push(line);
      if (verbose) process.stderr.write(`[${name}${tag}] ${line}`);
    });
  capture(child.stdout, "");
  capture(child.stderr, "!");

  let stopped = false;
  const stop = () => {
    if (!stopped) {
      stopped = true;
      child.kill("SIGKILL");
    }
  };
  const fail = (msg) => {
    stop();
    return new Error(`${msg}\n--- last core logs (${name}) ---\n${logs.slice(-15).join("")}`);
  };

  try {
    // 1) Wait for the HTTP server to accept connections.
    let up = false;
    const deadline = Date.now() + readyTimeoutMs;
    while (Date.now() < deadline) {
      if (child.exitCode !== null) throw fail(`core '${name}' exited early (code ${child.exitCode})`);
      try {
        const r = await fetch(`http://127.0.0.1:${port}/health`);
        if (r.ok) { up = true; break; }
      } catch { /* not up yet */ }
      await sleep(400);
    }
    if (!up) throw fail(`core '${name}' /health never came up on :${port}`);

    const rpc = makeRpc(port, token, name);

    // 2) Import a fresh wallet identity (encrypt the mnemonic, then persist it).
    const encryptedMnemonic = await rpc("openhuman.encrypt_secret", { plaintext: mnemonic });
    await rpc("openhuman.wallet_setup", {
      consentGranted: true,
      source: "imported",
      mnemonicWordCount: mnemonic.split(" ").length,
      encryptedMnemonic,
      accounts: PLACEHOLDER_ACCOUNTS,
      force: true,
    });

    // 3) Publish Signal pre-keys so peers can start an encrypted session, and
    //    advertise the encryption key on the directory card so it's routable.
    await rpc("openhuman.tinyplace_signal_provision", { preKeyCount });
    await rpc("openhuman.tinyplace_signal_register_encryption_key", {});

    const status = await rpc("openhuman.tinyplace_signal_key_status", {});
    const cryptoId = status.agentId;
    if (!cryptoId) throw fail(`core '${name}' produced an empty cryptoId`);

    return { name, cryptoId, rpc, port, workspace, stop, mnemonic };
  } catch (e) {
    stop();
    throw e;
  }
}

/**
 * Poll a core's mailbox, decrypting + acknowledging each envelope, until a
 * message from `fromCryptoId` (or any, if omitted) is decrypted — or timeout.
 * Returns the decrypted plaintext string, or null on timeout.
 */
export async function receiveMessage(core, { fromCryptoId, timeoutMs = 8_000 } = {}) {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    const list = await core.rpc("openhuman.tinyplace_messages_list", { limit: 50 });
    const envelopes = Array.isArray(list) ? list : list?.messages ?? [];
    for (const envelope of envelopes) {
      let decoded;
      try {
        decoded = await core.rpc("openhuman.tinyplace_signal_decrypt_message", { envelope });
      } finally {
        // Acknowledge (delete) regardless: the ratchet already advanced.
        await core.rpc("openhuman.tinyplace_messages_acknowledge", { messageId: envelope.id });
      }
      if (decoded && (!fromCryptoId || decoded.from === fromCryptoId)) {
        return decoded.plaintext;
      }
    }
    await sleep(400);
  }
  return null;
}
