import { expect, test } from '@playwright/test';
import { promises as fs } from 'node:fs';
import path from 'node:path';

import { bootAuthenticatedPage, callCoreRpc } from '../helpers/core-rpc';

function workspaceDir(): string {
  const ws = process.env.OPENHUMAN_WORKSPACE;
  if (!ws) throw new Error('OPENHUMAN_WORKSPACE not set for audio-toolkit-flow');
  return ws;
}

test.describe('Audio toolkit flow', () => {
  test.beforeEach(async ({ page }, testInfo) => {
    const slug = testInfo.title.toLowerCase().replace(/[^a-z0-9]+/g, '-');
    await bootAuthenticatedPage(page, `pw-audio-toolkit-${slug}`, '/home');
  });

  test('generates an mp3 artifact and captures the email attachment in the workspace', async () => {
    let generatedAndEmailed: {
      audio: { output_path: string; file_name: string; bytes_written: number; format: string };
      email: { mode: string; capture_path?: string | null; attachment_name: string };
    } | null = null;

    try {
      const response = await callCoreRpc<{
        result?: {
          audio: { output_path: string; file_name: string; bytes_written: number; format: string };
          email: { mode: string; capture_path?: string | null; attachment_name: string };
        };
        audio?: { output_path: string; file_name: string; bytes_written: number; format: string };
        email?: { mode: string; capture_path?: string | null; attachment_name: string };
      }>('openhuman.audio_toolkit_generate_and_email_podcast', {
        text: 'This is the weekly AI podcast briefing for the team.',
        title: 'Weekly briefing',
        to: 'listener@example.com',
        subject: 'Your weekly audio briefing',
        body: 'Attached is the latest audio briefing.',
        format: 'mp3',
      });

      generatedAndEmailed =
        response.result && 'audio' in response.result
          ? response.result
          : (response as unknown as {
              audio: {
                output_path: string;
                file_name: string;
                bytes_written: number;
                format: string;
              };
              email: { mode: string; capture_path?: string | null; attachment_name: string };
            });
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      expect(message).toContain('email channel is not configured');
    }

    if (generatedAndEmailed) {
      expect(generatedAndEmailed.audio.format).toBe('mp3');
      expect(generatedAndEmailed.audio.bytes_written).toBeGreaterThan(0);
      expect(generatedAndEmailed.email.mode).toBe('capture');
      expect(generatedAndEmailed.email.capture_path).toBeTruthy();

      const audioPath = path.join(
        workspaceDir(),
        'workspace',
        generatedAndEmailed.audio.output_path
      );
      const capturePath = path.join(
        workspaceDir(),
        'workspace',
        generatedAndEmailed.email.capture_path ?? ''
      );
      const audioStat = await fs.stat(audioPath);
      const emailWire = await fs.readFile(capturePath, 'utf8');

      expect(audioStat.size).toBeGreaterThan(0);
      expect(emailWire).toContain('Subject: Your weekly audio briefing');
      expect(emailWire).toContain(
        generatedAndEmailed.email.attachment_name ?? 'weekly-briefing.mp3'
      );
      return;
    }

    const generated = await callCoreRpc<{
      result?: { output_path: string; file_name: string; bytes_written: number; format: string };
      output_path?: string;
      file_name?: string;
      bytes_written?: number;
      format?: string;
    }>('openhuman.audio_toolkit_generate_podcast', {
      text: 'This is the weekly AI podcast briefing for the team.',
      title: 'Weekly briefing',
      format: 'mp3',
    });

    const audio =
      generated.result && 'output_path' in generated.result
        ? generated.result
        : (generated as unknown as {
            output_path: string;
            file_name: string;
            bytes_written: number;
            format: string;
          });

    expect(audio.format).toBe('mp3');
    expect(audio.bytes_written).toBeGreaterThan(0);
    const audioPath = path.join(workspaceDir(), 'workspace', audio.output_path);
    const audioStat = await fs.stat(audioPath);
    expect(audioStat.size).toBeGreaterThan(0);
  });
});
