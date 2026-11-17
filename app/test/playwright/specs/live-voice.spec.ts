import { expect, type Page, type Route, test, type WebSocketRoute } from '@playwright/test';

import { bootAuthenticatedPage, dismissWalkthroughIfPresent } from '../helpers/core-rpc';

/**
 * Live voice agent, driven in a real browser.
 *
 * The real core boots and signs in as usual; only the live-voice surface is
 * mocked so the spec never dials a voice provider:
 *  - `openhuman.voice_live_*` RPCs are fulfilled here (`page.route('**\/rpc')`,
 *    every other method falls through to the core);
 *  - the core's `/ws/live-voice` socket is replaced by `page.routeWebSocket`,
 *    which plays the core's side of the contract (ready → transcript → tools →
 *    agent audio → closed).
 * Chromium runs with a fake microphone so the AudioWorklet uplink really runs.
 */
test.use({
  launchOptions: { args: ['--use-fake-ui-for-media-stream', '--use-fake-device-for-media-stream'] },
  permissions: ['microphone'],
});

const PROVIDERS = {
  default_provider: 'gemini-hosted',
  providers: [
    {
      id: 'gemini-hosted',
      label: 'Gemini (TinyHumans)',
      kind: 'hosted',
      configured: true,
      key_slug: null,
      voices: ['Puck'],
      languages: ['en-US'],
    },
    {
      id: 'elevenlabs-hosted',
      label: 'ElevenLabs (TinyHumans)',
      kind: 'hosted',
      configured: true,
      key_slug: null,
      voices: [],
      languages: [],
    },
    {
      id: 'gemini',
      label: 'Gemini (Google key)',
      kind: 'byok',
      configured: false,
      key_slug: 'google',
      voices: [],
      languages: [],
    },
    {
      id: 'sarvam',
      label: 'Sarvam AI',
      kind: 'byok',
      configured: false,
      key_slug: 'sarvam',
      voices: ['anushka'],
      languages: ['hi-IN'],
    },
  ],
};

const SETTINGS = {
  default_provider: 'gemini-hosted',
  gemini: { model: null, voice: null, language: null },
  sarvam: { language: null, speaker: null, model: null },
  elevenlabs: { voice_id: null },
};

async function mockLiveVoiceRpc(page: Page): Promise<string[]> {
  const calls: string[] = [];
  await page.route('**/rpc', async (route: Route) => {
    const body = JSON.parse(route.request().postData() || '{}');
    const method: string = body.method ?? '';
    const reply = (result: unknown) =>
      route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({ jsonrpc: '2.0', id: body.id, result }),
      });
    switch (method) {
      case 'openhuman.voice_live_providers':
        calls.push(method);
        return reply(PROVIDERS);
      case 'openhuman.voice_live_settings_get':
        calls.push(method);
        return reply(SETTINGS);
      case 'openhuman.voice_live_settings_set':
        calls.push(method);
        return reply({ ...SETTINGS, ...(body.params ?? {}) });
      case 'openhuman.voice_live_test_provider':
        calls.push(`${method}:${body.params?.provider}`);
        return reply({ ok: true, latency_ms: 123, error: null });
      default:
        return route.fallback();
    }
  });
  return calls;
}

interface FakeCore {
  frames: Array<Record<string, unknown>>;
  audioBytes: () => number;
  socket: () => WebSocketRoute | null;
}

/** Play the core's side of `/ws/live-voice`. */
async function mockLiveVoiceSocket(page: Page): Promise<FakeCore> {
  const frames: Array<Record<string, unknown>> = [];
  let audioBytes = 0;
  let current: WebSocketRoute | null = null;
  await page.routeWebSocket(/\/ws\/live-voice/, ws => {
    current = ws;
    ws.onMessage(message => {
      if (typeof message !== 'string') {
        audioBytes += message.length;
        return;
      }
      const frame = JSON.parse(message) as Record<string, unknown>;
      frames.push(frame);
      if (frame.type === 'start') {
        ws.send(
          JSON.stringify({
            type: 'ready',
            session_id: 'pw-session',
            provider: 'gemini-hosted',
            output_sample_rate: 24000,
            thread_id: frame.thread_id ?? null,
          })
        );
      }
    });
  });
  return { frames, audioBytes: () => audioBytes, socket: () => current };
}

test.describe('Live voice agent', () => {
  test('Connections → Voice agents lists providers and tests one', async ({ page }) => {
    const calls = await mockLiveVoiceRpc(page);
    await bootAuthenticatedPage(page, 'pw-live-voice-panel', '/connections?tab=live-voice');
    await dismissWalkthroughIfPresent(page);

    await expect(page.getByTestId('two-pane-nav-voice-agents')).toHaveAttribute(
      'aria-current',
      'page'
    );
    await expect(page.getByTestId('live-voice-vendor-gemini')).toContainText(
      'Included with TinyHumans'
    );
    await expect(page.getByTestId('live-voice-vendor-sarvam')).toContainText('Needs a key');

    await page.getByTestId('live-voice-settings-gemini').click();
    await expect(page.getByTestId('live-voice-modal')).toBeVisible();
    await page.getByTestId('live-voice-test-button-gemini-hosted').click();
    await expect(page.getByTestId('live-voice-test-gemini-hosted')).toHaveText('Working · 123 ms');
    expect(calls).toContain('openhuman.voice_live_test_provider:gemini-hosted');
  });

  test('clicking the mascot starts a live session in the open thread', async ({ page }) => {
    await mockLiveVoiceRpc(page);
    const core = await mockLiveVoiceSocket(page);
    await bootAuthenticatedPage(page, 'pw-live-voice-session', '/chat');
    await dismissWalkthroughIfPresent(page);

    await page.getByTestId('composer-human-mode').click();
    await expect(page.getByTestId('chat-mascot-stage')).toBeVisible();

    // The start frame comes first and carries the open thread.
    await expect.poll(() => core.frames[0]?.type).toBe('start');
    expect(core.frames[0]).toMatchObject({ input_sample_rate: 16000 });
    const threadId = await page.evaluate(() => window.location.hash.match(/thread-[^/?]+/)?.[0]);
    if (threadId) expect(core.frames[0].thread_id).toBe(threadId);

    // The fake mic streams PCM16 frames once the core is ready.
    await expect.poll(() => core.audioBytes(), { timeout: 15_000 }).toBeGreaterThan(3200);
    await expect(page.getByTestId('live-voice-controls')).toHaveAttribute(
      'data-state',
      'listening'
    );
    await expect(page.getByTestId('live-voice-provider')).toHaveText('Gemini');

    const ws = core.socket()!;
    ws.send(JSON.stringify({ type: 'transcript', role: 'user', text: 'What is on', final: false }));
    await expect(page.getByTestId('live-voice-partial-user')).toHaveText('What is on');
    ws.send(
      JSON.stringify({ type: 'transcript', role: 'user', text: 'What is on today?', final: true })
    );
    await expect(page.getByTestId('live-voice-captions')).toContainText('What is on today?');

    ws.send(JSON.stringify({ type: 'tool_started', call_id: 'c1', name: 'calendar' }));
    await expect(page.getByTestId('live-voice-tool-chip')).toHaveAttribute(
      'data-status',
      'running'
    );
    ws.send(
      JSON.stringify({
        type: 'tool_finished',
        call_id: 'c1',
        name: 'calendar',
        ok: true,
        cancelled: false,
      })
    );
    await expect(page.getByTestId('live-voice-tool-chip')).toHaveAttribute('data-status', 'ok');

    // 200 ms of agent speech flips the stage to speaking.
    ws.send(Buffer.alloc(9600, 0x10));
    await expect(page.getByTestId('live-voice-controls')).toHaveAttribute('data-state', 'speaking');

    await page.getByTestId('live-voice-end').click();
    await expect.poll(() => core.frames[core.frames.length - 1]?.type).toBe('stop');
    await expect(page.getByTestId('live-voice-start')).toBeVisible();
  });

  test('a fatal core error shows a retry', async ({ page }) => {
    await mockLiveVoiceRpc(page);
    const core = await mockLiveVoiceSocket(page);
    await bootAuthenticatedPage(page, 'pw-live-voice-error', '/chat');
    await dismissWalkthroughIfPresent(page);

    await page.getByTestId('composer-human-mode').click();
    await expect.poll(() => core.frames[0]?.type).toBe('start');
    core
      .socket()!
      .send(
        JSON.stringify({
          type: 'error',
          code: 'provider_down',
          message: 'Provider down',
          fatal: true,
        })
      );
    await expect(page.getByTestId('live-voice-error')).toContainText('Provider down');
    await expect(page.getByTestId('live-voice-retry')).toBeVisible();
  });
});
