import { waitForApp } from '../helpers/app-helpers';
import { callOpenhumanRpc } from '../helpers/core-rpc';
import { resetApp } from '../helpers/reset-app';
import { startMockServer, stopMockServer } from '../mock-server';

const USER_ID = 'e2e-audio-toolkit';

describe('Audio toolkit flow', () => {
  before(async () => {
    await startMockServer();
    await waitForApp();
    await resetApp(USER_ID);
  });

  after(async () => {
    await stopMockServer();
  });

  it('generates an mp3 artifact and captures the email attachment in the workspace', async () => {
    const response = await callOpenhumanRpc<{
      result: {
        audio: { output_path: string; file_name: string; bytes_written: number; format: string };
        email: { mode: string; capture_path?: string | null; attachment_name: string };
      };
    }>('openhuman.audio_toolkit_generate_and_email_podcast', {
      text: 'This is the weekly AI podcast briefing for the team.',
      title: 'Weekly briefing',
      to: 'listener@example.com',
      subject: 'Your weekly audio briefing',
      body: 'Attached is the latest audio briefing.',
      format: 'mp3',
    });

    expect(response.ok).toBe(true);
    const result = (response.result?.result ?? response.result) as
      | {
          audio: { output_path: string; file_name: string; bytes_written: number; format: string };
          email: { mode: string; capture_path?: string | null; attachment_name: string };
        }
      | undefined;
    expect(result?.audio.format).toBe('mp3');
    expect(result?.audio.bytes_written).toBeGreaterThan(0);
    expect(result?.email.mode).toBe('capture');
    expect(result?.email.capture_path).toBeTruthy();

    const workspaceFiles = await callOpenhumanRpc<{
      result: { entries: Array<{ rel_path: string; size: number; is_dir: boolean }> };
    }>('openhuman.test_support_list_workspace_files', { rel_root: 'artifacts', max_depth: 4 });
    expect(workspaceFiles.ok).toBe(true);
    const entries =
      (
        (workspaceFiles.result?.result ?? workspaceFiles.result) as
          | { entries?: Array<{ rel_path: string; size: number; is_dir: boolean }> }
          | undefined
      )?.entries ?? [];
    const audioArtifact = entries.find(
      e =>
        e.rel_path === result?.audio.output_path ||
        !!result?.audio.output_path?.endsWith(`/${e.rel_path}`) ||
        e.rel_path.endsWith(`/${result?.audio.output_path ?? ''}`)
    );
    expect(audioArtifact?.size ?? 0).toBeGreaterThan(0);
    const capturedEmail = entries.find(
      e =>
        e.rel_path === result?.email.capture_path ||
        !!result?.email.capture_path?.endsWith(`/${e.rel_path}`) ||
        e.rel_path.endsWith(`/${result?.email.capture_path ?? ''}`)
    );
    expect(capturedEmail?.size ?? 0).toBeGreaterThan(0);

    const emailRead = await callOpenhumanRpc<{ result: { content_utf8: string } }>(
      'openhuman.test_support_read_workspace_file',
      { rel_path: result?.email.capture_path, max_bytes: 131072 }
    );
    expect(emailRead.ok).toBe(true);
    const wire =
      ((emailRead.result?.result ?? emailRead.result) as { content_utf8?: string } | undefined)
        ?.content_utf8 ?? '';
    expect(wire).toContain('Subject: Your weekly audio briefing');
    expect(wire).toContain(result?.email.attachment_name ?? 'weekly-briefing.mp3');
  });
});
