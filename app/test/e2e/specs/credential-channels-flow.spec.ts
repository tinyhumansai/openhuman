/**
 * Credential channels — Yuanbao (10.1.5) and Email/IMAP-SMTP (10.1.6).
 *
 * Both are `api_key` channels: the user pastes credentials into the generic
 * `ChannelSetupModal` and the core validates them against the channel's
 * `ChannelDefinition`. Neither had a spec at any UI layer —
 * `git grep -l 'yuanbao\|imap' app/test/e2e/specs app/test/playwright/specs`
 * returned nothing — which is what matrix rows 10.1.5 ("No WDIO spec yet") and
 * 10.1.6 ("WDIO connect-flow [is a] follow-up") record.
 *
 * ## What this asserts, and what it deliberately leaves to the Rust unit tests
 *
 * The Rust side already covers the *ops layer* for both channels:
 * `channels/controllers/ops_yuanbao_email_tests.rs` and
 * `ops/connect_email_config_tests_tests.rs` cover credential→config mapping,
 * defaults, port/sender parsing, persist + disconnect and pre-network
 * invalid-port rejection. Re-asserting any of that here would duplicate it.
 *
 * What no layer covered is the **RPC surface these channels are reached
 * through**: that each one is in the definition table `channels_list` serves,
 * that `channels_describe` returns the field set the setup modal renders its
 * inputs from, and that connect→status→disconnect round-trips over the wire.
 * A channel can be fully implemented in `ops` and still be unreachable if it
 * drops out of `all_channel_definitions()` — `channels_connect` resolves the
 * definition first (`tinychannels/src/backend.rs:208-210`) and fails with
 * "unknown channel" before any of the covered code runs. Slack and WhatsApp are
 * both in that state today; nothing caught it.
 *
 * Mirrors the C.1/C.2/C.3/C.4/C.8/C.9 shape of `telegram-channel-flow.spec.ts`,
 * which is the house pattern for a channel lifecycle.
 *
 * No external network: Yuanbao credential verification is directed to the
 * local mock backend. Email's required IMAP login is tested against a
 * test-owned loopback socket that drops the TLS handshake before credentials
 * can be sent.
 */
import fs from 'node:fs';
import { createServer } from 'node:net';
import path from 'node:path';

import { waitForApp } from '../helpers/app-helpers';
import { callOpenhumanRpc } from '../helpers/core-rpc';
import { resetApp } from '../helpers/reset-app';
import { getMockServerPort, startMockServer, stopMockServer } from '../mock-server';

const LOG_PREFIX = '[CredentialChannels]';

/** Accept one local connection and drop the TLS handshake before IMAP credentials can be sent. */
async function startImapHandshakeRejector(): Promise<{
  port: number;
  connectionCount: () => number;
  close: () => Promise<void>;
}> {
  let connections = 0;
  const server = createServer(socket => {
    connections += 1;
    socket.destroy();
  });
  await new Promise<void>((resolve, reject) => {
    server.once('error', reject);
    server.listen(0, '127.0.0.1', resolve);
  });
  const address = server.address();
  if (!address || typeof address === 'string') throw new Error('IMAP fixture did not bind');
  return {
    port: address.port,
    connectionCount: () => connections,
    close: () =>
      new Promise<void>((resolve, reject) =>
        server.close(error => (error ? reject(error) : resolve()))
      ),
  };
}

function seedEmailConfigForDisconnect(contents: string): string {
  const workspace = process.env.OPENHUMAN_WORKSPACE?.trim();
  if (!workspace) throw new Error('OPENHUMAN_WORKSPACE is required to seed the email config');
  const file = path.join(workspace, 'config.toml');
  fs.writeFileSync(
    file,
    `${contents.trimEnd()}\n\n[channels_config.email]\nimap_host = "127.0.0.1"\nsmtp_host = "127.0.0.1"\nusername = "e2e@example.invalid"\npassword = ""\nfrom_address = "e2e@example.invalid"\nallowed_senders = ["*"]\n`
  );
  return file;
}

interface AuthModeSpec {
  mode?: string;
  fields?: { key?: string; required?: boolean }[];
}

interface ChannelDefinition {
  id?: string;
  display_name?: string;
  auth_modes?: AuthModeSpec[];
  authModes?: AuthModeSpec[];
}

interface ChannelStatusEntry {
  channelId?: string;
  channel_id?: string;
  connected?: boolean;
  hasCredentials?: boolean;
  has_credentials?: boolean;
}

/** `Outcome` wraps payloads inconsistently across controllers; drill down. */
function unwrap(result: unknown): unknown {
  const outer = (result as Record<string, unknown> | null) ?? {};
  if (Array.isArray(outer)) return outer;
  return (outer as Record<string, unknown>).result ?? outer;
}

function asArray(value: unknown): unknown[] {
  return Array.isArray(value) ? value : [];
}

function authModesOf(def: ChannelDefinition | undefined): AuthModeSpec[] {
  return asArray(def?.auth_modes ?? def?.authModes) as AuthModeSpec[];
}

/** Field keys declared for one auth mode, as the setup modal would read them. */
function fieldKeysFor(def: ChannelDefinition | undefined, mode: string): string[] {
  const spec = authModesOf(def).find(m => m.mode === mode);
  return asArray(spec?.fields)
    .map(f => (f as { key?: string }).key)
    .filter((k): k is string => typeof k === 'string');
}

async function describeChannel(channel: string): Promise<ChannelDefinition | undefined> {
  const out = await callOpenhumanRpc('openhuman.channels_describe', { channel });
  expect(out.ok).toBe(true);
  return unwrap(out.result) as ChannelDefinition | undefined;
}

async function statusFor(channel: string): Promise<ChannelStatusEntry | undefined> {
  const out = await callOpenhumanRpc('openhuman.channels_status', { channel });
  expect(out.ok).toBe(true);
  const payload = unwrap(out.result);
  const entries = asArray(
    Array.isArray(payload) ? payload : (payload as Record<string, unknown>)?.entries
  ) as ChannelStatusEntry[];
  return entries.find(e => (e.channelId ?? e.channel_id) === channel);
}

function isConnected(entry: ChannelStatusEntry | undefined): boolean {
  return entry?.connected === true;
}

/**
 * The two credential channels under test, with the field keys their
 * definitions declare (`tinychannels-bus/src/controllers/definitions.rs`:
 * `yuanbao_definition` :634, `email_definition` :533).
 */
const CREDENTIAL_CHANNELS = [
  {
    channel: 'yuanbao',
    label: 'Yuanbao',
    requiredFields: ['app_key', 'app_secret'],
    validCredentials: { app_key: 'e2e-yuanbao-app-key', app_secret: 'e2e-yuanbao-app-secret' },
    /** Omits `app_secret`, which the definition marks required. */
    incompleteCredentials: { app_key: 'e2e-yuanbao-app-key' },
    missingFieldHint: 'app_secret',
  },
  {
    channel: 'email',
    label: 'Email (IMAP/SMTP)',
    requiredFields: ['imap_host', 'smtp_host', 'username', 'password'],
    validCredentials: {
      imap_host: 'imap.e2e.invalid',
      imap_port: '993',
      smtp_host: 'smtp.e2e.invalid',
      smtp_port: '465',
      username: 'e2e@example.invalid',
      password: 'e2e-app-password',
    },
    /** Omits `username`, which the definition marks required. */
    incompleteCredentials: {
      imap_host: 'imap.e2e.invalid',
      smtp_host: 'smtp.e2e.invalid',
      password: 'e2e-app-password',
    },
    missingFieldHint: 'username',
  },
] as const;

describe('Credential channels — Yuanbao and Email (IMAP/SMTP)', () => {
  before(async function beforeSuite() {
    this.timeout(90_000);
    await startMockServer();
    await waitForApp();
    await resetApp('e2e-credential-channels');
  });

  after(async () => {
    await stopMockServer();
  });

  it('D.1 channels_list includes both credential channels with an api_key auth mode', async function () {
    this.timeout(30_000);
    const out = await callOpenhumanRpc('openhuman.channels_list', {});
    expect(out.ok).toBe(true);

    const payload = unwrap(out.result);
    const channels = asArray(
      Array.isArray(payload) ? payload : (payload as Record<string, unknown>)?.channels
    ) as ChannelDefinition[];
    expect(channels.length).toBeGreaterThan(0);

    for (const { channel, label } of CREDENTIAL_CHANNELS) {
      const def = channels.find(c => c.id === channel);
      if (!def) {
        throw new Error(
          `${label} is missing from channels_list, so the setup UI cannot offer it and ` +
            `channels_connect would fail with "unknown channel: ${channel}"`
        );
      }
      if (!authModesOf(def).some(m => m.mode === 'api_key')) {
        throw new Error(`${label} should advertise the api_key auth mode`);
      }
    }
  });

  for (const spec of CREDENTIAL_CHANNELS) {
    const { channel, label, requiredFields, validCredentials, incompleteCredentials } = spec;

    it(`D.2 channels_describe for ${channel} returns the fields the setup modal renders`, async function () {
      this.timeout(30_000);
      const def = await describeChannel(channel);
      expect(def?.id).toBe(channel);

      const keys = fieldKeysFor(def, 'api_key');
      for (const field of requiredFields) {
        if (!keys.includes(field)) {
          throw new Error(
            `${label}'s api_key auth mode should declare the "${field}" field — the generic ` +
              `ChannelSetupModal builds its inputs from this list, so a field that drops out ` +
              `of the definition silently disappears from the connect form. Got: ${keys.join(', ')}`
          );
        }
      }
    });

    it(
      channel === 'email'
        ? 'D.3 email refuses local IMAP verification without storing credentials'
        : `D.3 ${channel} connect with complete credentials reports connected`,
      async function () {
        this.timeout(60_000);
        // Start from a known-disconnected state so a leftover connection from an
        // earlier run cannot make the assertion below pass without a connect.
        const disconnected = await callOpenhumanRpc('openhuman.channels_disconnect', {
          channel,
          authMode: 'api_key',
        });
        expect(disconnected.ok).toBe(true);
        if (isConnected(await statusFor(channel))) {
          throw new Error(
            `precondition: ${label} should be disconnected before the connect under test`
          );
        }

        const imapFixture = channel === 'email' ? await startImapHandshakeRejector() : undefined;
        try {
          const credentials = imapFixture
            ? { ...validCredentials, imap_host: '127.0.0.1', imap_port: String(imapFixture.port) }
            : { ...validCredentials, api_domain: `http://127.0.0.1:${getMockServerPort()}` };
          const out = await callOpenhumanRpc('openhuman.channels_connect', {
            channel,
            authMode: 'api_key',
            credentials,
          });
          if (channel === 'email') {
            expect(imapFixture?.connectionCount()).toBeGreaterThan(0);
            expect(out.ok).toBe(false);
            // Remote fault text is intentionally hidden by ModuleClient.
            // The socket count above proves this is the live verifier's
            // refusal, rather than an unavailable or incompatible module.
            expect(out.error).toBe('MODULE_CALL_REPORTED: module execution failed');
            for (const value of Object.values(credentials)) {
              expect(out.error).not.toContain(value);
            }
            const status = await statusFor(channel);
            expect(isConnected(status)).toBe(false);
            expect(status?.hasCredentials ?? status?.has_credentials).toBe(false);
            console.log(`${LOG_PREFIX} D.3 email: local IMAP verification rejected`);
            return;
          }

          if (!out.ok) {
            throw new Error(`${label} connect should be accepted: ${JSON.stringify(out)}`);
          }

          if (!isConnected(await statusFor(channel))) {
            throw new Error(
              `${label} reported a successful connect but channels_status does not show it ` +
                `connected — the credentials were accepted and then not persisted`
            );
          }
          console.log(`${LOG_PREFIX} D.3 ${channel}: connected`);
        } finally {
          await imapFixture?.close();
        }
      }
    );

    it(`D.4 ${channel} connect missing a required credential is rejected`, async function () {
      this.timeout(60_000);
      await callOpenhumanRpc('openhuman.channels_disconnect', { channel, authMode: 'api_key' });

      const out = await callOpenhumanRpc('openhuman.channels_connect', {
        channel,
        authMode: 'api_key',
        credentials: incompleteCredentials,
      });

      if (out.ok) {
        throw new Error(
          `${label} accepted a connect that omits the required "${spec.missingFieldHint}" field; ` +
            `validate_credentials is not being applied, so an incomplete setup would be stored ` +
            `and fail later at send time instead of in the form`
        );
      }

      if (isConnected(await statusFor(channel))) {
        throw new Error(`${label} rejected the incomplete credentials but still reports connected`);
      }
      console.log(`${LOG_PREFIX} D.4 ${channel}: incomplete credentials rejected`);
    });

    it(`D.5 ${channel} disconnect clears the stored connection`, async function () {
      this.timeout(60_000);
      if (channel === 'email') {
        const workspace = process.env.OPENHUMAN_WORKSPACE?.trim();
        if (!workspace) throw new Error('OPENHUMAN_WORKSPACE is required to test email disconnect');
        const configFile = path.join(workspace, 'config.toml');
        const originalConfig = fs.readFileSync(configFile, 'utf8');
        try {
          const disconnected = await callOpenhumanRpc('openhuman.channels_disconnect', {
            channel,
            authMode: 'api_key',
          });
          expect(disconnected.ok).toBe(true);
          expect(originalConfig).not.toContain('[channels_config.email]');
          seedEmailConfigForDisconnect(originalConfig);
          expect(fs.readFileSync(configFile, 'utf8')).toContain('[channels_config.email]');

          const out = await callOpenhumanRpc('openhuman.channels_disconnect', {
            channel,
            authMode: 'api_key',
          });
          expect(out.ok).toBe(true);
          expect(fs.readFileSync(configFile, 'utf8')).not.toContain('[channels_config.email]');
          const status = await statusFor(channel);
          expect(isConnected(status)).toBe(false);
          expect(status?.hasCredentials ?? status?.has_credentials).toBe(false);
          console.log(`${LOG_PREFIX} D.5 email: persisted config removed by disconnect RPC`);
        } finally {
          fs.writeFileSync(configFile, originalConfig);
        }
        return;
      }
      await callOpenhumanRpc('openhuman.channels_connect', {
        channel,
        authMode: 'api_key',
        credentials: {
          ...validCredentials,
          ...(channel === 'yuanbao'
            ? { api_domain: `http://127.0.0.1:${getMockServerPort()}` }
            : {}),
        },
      });
      if (!isConnected(await statusFor(channel))) {
        throw new Error(
          `precondition: ${label} should be connected before the disconnect under test`
        );
      }

      const out = await callOpenhumanRpc('openhuman.channels_disconnect', {
        channel,
        authMode: 'api_key',
      });
      expect(out.ok).toBe(true);

      if (isConnected(await statusFor(channel))) {
        throw new Error(
          `${label} disconnect returned ok but channels_status still reports it connected`
        );
      }
      console.log(`${LOG_PREFIX} D.5 ${channel}: disconnected`);
    });
  }
});
