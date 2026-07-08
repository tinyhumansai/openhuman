// End-to-end: openhuman ↔ tiny.place messaging, driven through the real
// openhuman core JSON-RPC surface against a real tiny.place backend.
//
// Two independent `openhuman-core` processes (Alice, Bob), each with its own
// freshly-imported wallet identity, exercise the complete DM lifecycle exactly
// as the desktop app would drive it over `core_rpc_relay`:
//
//   • send a contact request            (openhuman.tinyplace_contacts_request)
//   • see it arrive on the other side    (openhuman.tinyplace_contacts_requests)
//   • accept the request                 (openhuman.tinyplace_contacts_accept)
//   • confirm the mutual contact         (openhuman.tinyplace_contacts_list)
//   • send Signal-encrypted DMs          (openhuman.tinyplace_signal_send_message)
//   • receive + decrypt + acknowledge    (messages_list / signal_decrypt_message / messages_acknowledge)
//
// It also proves the two security invariants that make this a real messaging
// system: DMs are refused between non-contacts, and the relay only ever stores
// opaque ciphertext.
//
// Requires: a reachable tiny.place backend (TINYPLACE_API_BASE_URL, default
// http://localhost:18080) and a built `openhuman-core` binary. `run.sh` wires
// both up; see README.md.
import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import net from "node:net";

import { launchAgent, receiveMessage, DEFAULT_BACKEND, DEFAULT_CORE_BIN } from "./lib/core.mjs";
import { existsSync } from "node:fs";

function freePort() {
  return new Promise((res, rej) => {
    const srv = net.createServer();
    srv.on("error", rej);
    srv.listen(0, "127.0.0.1", () => {
      const { port } = srv.address();
      srv.close(() => res(port));
    });
  });
}

async function backendReachable() {
  try {
    const r = await fetch(`${DEFAULT_BACKEND}/healthz`);
    return r.ok;
  } catch {
    return false;
  }
}

let alice;
let bob;

before(async () => {
  assert.ok(
    existsSync(DEFAULT_CORE_BIN),
    `openhuman-core binary not found at ${DEFAULT_CORE_BIN}. Build it: cargo build --bin openhuman-core (or run run.sh).`,
  );
  assert.ok(
    await backendReachable(),
    `tiny.place backend not reachable at ${DEFAULT_BACKEND}/healthz. Start it first (see run.sh / README.md).`,
  );
  const [pa, pb] = await Promise.all([freePort(), freePort()]);
  [alice, bob] = await Promise.all([
    launchAgent("alice", { port: pa }),
    launchAgent("bob", { port: pb }),
  ]);
});

after(() => {
  alice?.stop();
  bob?.stop();
});

test("each core boots a distinct, message-ready tiny.place identity", async () => {
  assert.match(alice.cryptoId, /^[1-9A-HJ-NP-Za-km-z]{32,44}$/, "alice cryptoId is base58");
  assert.match(bob.cryptoId, /^[1-9A-HJ-NP-Za-km-z]{32,44}$/, "bob cryptoId is base58");
  assert.notEqual(alice.cryptoId, bob.cryptoId, "the two cores have different identities");

  for (const core of [alice, bob]) {
    const status = await core.rpc("openhuman.tinyplace_signal_key_status", {});
    assert.equal(status.agentId, core.cryptoId);
    assert.equal(status.hasActiveSignedPreKey, true, `${core.name} published a signed pre-key`);
    assert.ok(status.localPreKeyCount > 0, `${core.name} holds local one-time pre-keys`);
    assert.equal(status.encryptionKeyPublished, true, `${core.name} advertised its encryption key`);
  }
});

test("a DM is refused between agents who are not contacts", async () => {
  // Fresh, unrelated identity → not a contact of Bob → the relay must refuse.
  const stranger = await launchAgent("stranger", { port: await freePort() });
  try {
    await assert.rejects(
      () =>
        stranger.rpc("openhuman.tinyplace_signal_send_message", {
          recipient: bob.cryptoId,
          plaintext: "you don't know me",
        }),
      /not_a_contact|403|contact/i,
      "sending a DM without an accepted contact relationship is rejected",
    );
  } finally {
    stranger.stop();
  }
});

test("Alice sends a contact request and Bob sees it pending", async () => {
  await alice.rpc("openhuman.tinyplace_contacts_request", { agentId: bob.cryptoId });

  const incoming = await bob.rpc("openhuman.tinyplace_contacts_requests", {});
  const ids = (incoming.incoming ?? []).map((r) => r.agentId ?? r.cryptoId);
  assert.ok(ids.includes(alice.cryptoId), "Alice's request is in Bob's incoming requests");

  const status = await bob.rpc("openhuman.tinyplace_contacts_status", { agentId: alice.cryptoId });
  assert.equal(status.status, "pending", "relationship is pending before acceptance");
});

test("Bob accepts and both sides see a mutual accepted contact", async () => {
  await bob.rpc("openhuman.tinyplace_contacts_accept", { agentId: alice.cryptoId });

  const aliceContacts = await alice.rpc("openhuman.tinyplace_contacts_list", {});
  const aliceIds = (aliceContacts.contacts ?? []).map((c) => c.agentId ?? c.cryptoId);
  assert.ok(aliceIds.includes(bob.cryptoId), "Bob is in Alice's contact list");

  const bobStatus = await bob.rpc("openhuman.tinyplace_contacts_status", { agentId: alice.cryptoId });
  assert.equal(bobStatus.status, "accepted", "relationship is accepted");
});

test("Alice → Bob: first DM (X3DH) is delivered as ciphertext and decrypts", async () => {
  const plaintext = "hello bob — first encrypted DM from alice";
  const sent = await alice.rpc("openhuman.tinyplace_signal_send_message", {
    recipient: bob.cryptoId,
    plaintext,
  });
  assert.equal(sent.encrypted, true, "send reports the message was encrypted");
  assert.ok(sent.messageId, "send returns a messageId");

  // Inspect the raw relayed envelope before decrypting: the relay must only
  // ever hold opaque ciphertext, never the plaintext.
  let sawCiphertext = false;
  for (let i = 0; i < 15 && !sawCiphertext; i++) {
    const list = await bob.rpc("openhuman.tinyplace_messages_list", { limit: 50 });
    const envelopes = Array.isArray(list) ? list : list?.messages ?? [];
    const env = envelopes.find((e) => e.id === sent.messageId);
    if (env) {
      assert.notEqual(env.body, plaintext, "relay envelope body is not the plaintext");
      assert.ok(!String(env.body ?? "").includes(plaintext), "ciphertext does not contain the plaintext");
      assert.match(env.type, /PREKEY_BUNDLE|CIPHERTEXT/, "envelope carries a Signal message type");
      sawCiphertext = true;
    } else {
      await new Promise((r) => setTimeout(r, 400));
    }
  }
  assert.ok(sawCiphertext, "Bob's relay mailbox received the ciphertext envelope");

  const received = await receiveMessage(bob, { fromCryptoId: alice.cryptoId });
  assert.equal(received, plaintext, "Bob decrypts Alice's first message");
});

test("Bob → Alice: ratchet reply decrypts, proving a bidirectional session", async () => {
  const plaintext = "hi alice — ratchet reply from bob";
  const sent = await bob.rpc("openhuman.tinyplace_signal_send_message", {
    recipient: alice.cryptoId,
    plaintext,
  });
  assert.equal(sent.encrypted, true);

  const received = await receiveMessage(alice, { fromCryptoId: bob.cryptoId });
  assert.equal(received, plaintext, "Alice decrypts Bob's reply");
});

test("a follow-up DM in the established session still decrypts", async () => {
  const plaintext = "and one more, to prove the session persists";
  await alice.rpc("openhuman.tinyplace_signal_send_message", {
    recipient: bob.cryptoId,
    plaintext,
  });
  const received = await receiveMessage(bob, { fromCryptoId: alice.cryptoId });
  assert.equal(received, plaintext, "Bob decrypts a subsequent in-session message");
});
