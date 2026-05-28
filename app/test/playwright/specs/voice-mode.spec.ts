import { expect, type Page, test } from '@playwright/test';

import {
  bootAuthenticatedPage,
  callCoreRpc,
  dismissWalkthroughIfPresent,
} from '../helpers/core-rpc';

async function openChat(page: Page): Promise<void> {
  await bootAuthenticatedPage(page, 'pw-voice-mode', '/chat');
  await page.goto('/#/chat');
  await page.evaluate(() => {
    localStorage.setItem('openhuman:walkthrough_completed', 'true');
    localStorage.removeItem('openhuman:walkthrough_pending');
  });
  await dismissWalkthroughIfPresent(page);
  const skipButton = page.getByRole('button', { name: /Skip|Skip tour/i });
  if (
    await skipButton
      .first()
      .isVisible()
      .catch(() => false)
  ) {
    await skipButton.first().click({ force: true });
    await expect(skipButton.first()).toBeHidden();
  }
  await expect(page.getByPlaceholder('Type a message...')).toBeVisible();
}

async function installGetUserMediaError(page: Page, name: string): Promise<void> {
  await page.evaluate(errorName => {
    const mediaDevices = navigator.mediaDevices as MediaDevices & {
      __e2e_original_getUserMedia?: MediaDevices['getUserMedia'];
    };
    if (!mediaDevices.__e2e_original_getUserMedia) {
      mediaDevices.__e2e_original_getUserMedia = mediaDevices.getUserMedia.bind(mediaDevices);
    }
    Object.defineProperty(mediaDevices, 'getUserMedia', {
      configurable: true,
      value: () =>
        Promise.reject(new DOMException(`[Playwright voice mock] ${errorName}`, errorName)),
    });
  }, name);
}

async function restoreGetUserMedia(page: Page): Promise<void> {
  if (page.isClosed()) return;
  await page.evaluate(() => {
    const mediaDevices = navigator.mediaDevices as MediaDevices & {
      __e2e_original_getUserMedia?: MediaDevices['getUserMedia'];
    };
    if (mediaDevices.__e2e_original_getUserMedia) {
      Object.defineProperty(mediaDevices, 'getUserMedia', {
        configurable: true,
        value: mediaDevices.__e2e_original_getUserMedia,
      });
      delete mediaDevices.__e2e_original_getUserMedia;
    }
  });
}

async function switchChatIntoMicComposer(page: Page): Promise<void> {
  await dismissWalkthroughIfPresent(page);
  await page.getByRole('button', { name: 'Start recording' }).click({ force: true });
  await expect(page.getByText(/Tap and speak|Waiting for agent/i)).toBeVisible();
  await expect(page.getByRole('button', { name: 'Switch to text' })).toBeVisible();
}

test.describe('Voice mode integration', () => {
  test.beforeEach(async ({ page }) => {
    await openChat(page);
  });

  test('chat mic button switches into MicComposer and can return to text mode', async ({
    page,
  }) => {
    await switchChatIntoMicComposer(page);

    await page.getByRole('button', { name: 'Switch to text' }).click();
    await expect(page.getByPlaceholder('Type a message...')).toBeVisible();
    await expect(page.getByTestId('send-message-button')).toBeVisible();
  });

  test('permission-denied getUserMedia shows a specific voice-transcription error', async ({
    page,
  }) => {
    try {
      await switchChatIntoMicComposer(page);
      await installGetUserMediaError(page, 'NotAllowedError');
      await page.getByRole('button', { name: 'Start recording' }).click();

      const errorBanner = page.locator('[data-chat-send-error-code="voice_transcription"]');
      await expect(errorBanner).toBeVisible();
      await expect(errorBanner).toContainText(/permission|denied|microphone/i);
      await expect(errorBanner).not.toContainText(/something went wrong/i);
    } finally {
      await restoreGetUserMedia(page);
    }
  });

  test('missing-device getUserMedia shows a specific unavailable-device error', async ({
    page,
  }) => {
    try {
      await switchChatIntoMicComposer(page);
      await installGetUserMediaError(page, 'NotFoundError');
      await page.getByRole('button', { name: 'Start recording' }).click();

      const errorBanner = page.locator('[data-chat-send-error-code="voice_transcription"]');
      await expect(errorBanner).toBeVisible();
      await expect(errorBanner).toContainText(/unavailable|device|microphone|not found/i);
      await expect(errorBanner).not.toContainText(/something went wrong/i);
    } finally {
      await restoreGetUserMedia(page);
    }
  });
});

test.describe('Voice mode - offline STT contract (voice_status RPC)', () => {
  test.beforeEach(async ({ page }) => {
    await bootAuthenticatedPage(page, 'pw-voice-mode-status', '/home');
  });

  test('voice_status RPC returns a well-formed response', async () => {
    const status = await callCoreRpc<unknown>('openhuman.voice_status', {});
    const root = (status ?? {}) as Record<string, unknown>;
    const payload =
      root && typeof root === 'object' && 'result' in root
        ? (root.result as Record<string, unknown>)
        : root;

    expect(typeof payload.stt_available).toBe('boolean');
    expect(typeof payload.tts_available).toBe('boolean');
    expect(typeof payload.stt_provider).toBe('string');
  });

  test('voice_status reports a declared provider even when local assets are unavailable', async () => {
    const status = await callCoreRpc<unknown>('openhuman.voice_status', {});
    const root = (status ?? {}) as Record<string, unknown>;
    const payload =
      root && typeof root === 'object' && 'result' in root
        ? (root.result as Record<string, unknown>)
        : root;

    const sttProvider = String(payload.stt_provider ?? '');
    expect(sttProvider.length).toBeGreaterThan(0);

    const whisperBinary = payload.whisper_binary;
    const sttModelPath = payload.stt_model_path;
    if ((sttProvider === 'whisper' || sttProvider === 'local') && !whisperBinary && !sttModelPath) {
      expect(payload.stt_available).toBe(false);
    }
  });
});
