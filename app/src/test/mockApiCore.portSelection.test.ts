import net from 'node:net';
import { afterEach, expect, it } from 'vitest';

// @ts-ignore - test-only JS module outside app/src
import {
  getMockServerPort,
  startMockServer,
  stopMockServer,
} from '../../../scripts/mock-api-core.mjs';

const preferredPort = 5005;

function listenOn(port: number): Promise<net.Server> {
  const server = net.createServer();
  return new Promise((resolve, reject) => {
    server.once('error', reject);
    server.listen(port, '127.0.0.1', () => {
      server.off('error', reject);
      resolve(server);
    });
  });
}

function closeServer(server: net.Server | null): Promise<void> {
  return new Promise(resolve => {
    if (!server?.listening) {
      resolve();
      return;
    }
    server.close(() => resolve());
  });
}

afterEach(async () => {
  await stopMockServer();
  await startMockServer(preferredPort, { retryIfInUse: true });
});

it('falls back to an available local port when the preferred Vitest mock port is occupied', async () => {
  await stopMockServer();
  const blocker = await listenOn(preferredPort);

  try {
    const started = await startMockServer(preferredPort, { retryIfInUse: true });

    expect(started.alreadyRunning).toBe(false);
    expect(started.requestedPort).toBe(preferredPort);
    expect(started.retried).toBe(true);
    expect(started.port).not.toBe(preferredPort);
    expect(started.port).toBeGreaterThan(0);
    expect(getMockServerPort()).toBe(started.port);

    const response = await fetch(`http://127.0.0.1:${started.port}/__admin/health`);
    await expect(response.json()).resolves.toMatchObject({ ok: true, port: started.port });
  } finally {
    await closeServer(blocker);
  }
});
