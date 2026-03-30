import fs from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';

import { waitForApp, waitForAppReady } from '../helpers/app-helpers';
import { triggerAuthDeepLink } from '../helpers/deep-link-helpers';
import {
  clickButton,
  textExists,
  waitForText,
  waitForWebView,
  waitForWindowVisible,
} from '../helpers/element-helpers';
import { startMockServer, stopMockServer } from '../mock-server';

interface ServiceMockFailures {
  install?: string;
  start?: string;
  stop?: string;
  status?: string;
  uninstall?: string;
}

interface ServiceMockState {
  installed: boolean;
  running: boolean;
  agent_running: boolean;
  failures: ServiceMockFailures;
}

const DEFAULT_MOCK_STATE: ServiceMockState = {
  installed: false,
  running: false,
  agent_running: false,
  failures: {},
};

const mockStateFile =
  process.env.OPENHUMAN_SERVICE_MOCK_STATE_FILE ||
  path.join(process.env.OPENHUMAN_WORKSPACE || os.tmpdir(), 'service-mock-state.json');

function stepLog(message: string, context?: unknown): void {
  const stamp = new Date().toISOString();
  if (context === undefined) {
    console.log(`[ServiceConnectivityE2E][${stamp}] ${message}`);
    return;
  }
  console.log(`[ServiceConnectivityE2E][${stamp}] ${message}`, JSON.stringify(context, null, 2));
}

async function writeMockState(state: ServiceMockState): Promise<void> {
  stepLog('Writing mock state', state);
  await fs.mkdir(path.dirname(mockStateFile), { recursive: true });
  await fs.writeFile(mockStateFile, JSON.stringify(state, null, 2), 'utf-8');
}

async function readMockState(): Promise<ServiceMockState> {
  const raw = await fs.readFile(mockStateFile, 'utf-8');
  const parsed = JSON.parse(raw) as ServiceMockState;
  stepLog('Read mock state', parsed);
  return parsed;
}

async function waitForServiceStateText(stateText: string, timeoutMs = 15_000): Promise<void> {
  await waitForText(stateText, timeoutMs);
}

describe('Service connectivity flow (UI ↔ Rust service)', () => {
  before(async function beforeSuite() {
    if (process.env.OPENHUMAN_SERVICE_MOCK !== '1') {
      this.skip();
    }

    stepLog('Starting suite with service mock mode enabled', {
      openhumanServiceMock: process.env.OPENHUMAN_SERVICE_MOCK,
      mockStateFile,
    });
    await writeMockState(DEFAULT_MOCK_STATE);
    await startMockServer();
    stepLog('Mock backend started');
    await waitForApp();
    stepLog('App process launched');

    await triggerAuthDeepLink('service-connectivity-token');
    stepLog('Triggered auth deep link');
    await waitForWindowVisible(25_000);
    await waitForWebView(15_000);
    await waitForAppReady(15_000);
    stepLog('App window and webview ready');
  });

  after(async () => {
    stepLog('Stopping mock backend');
    await stopMockServer();
  });

  it('shows the blocking gate when service is not installed', async () => {
    await waitForText('OpenHuman Service Required', 20_000);
    await waitForServiceStateText('NotInstalled');

    expect(await textExists('Install Service')).toBe(true);
    expect(await textExists('Start Service')).toBe(true);
    expect(await textExists('Stop Service')).toBe(true);
    expect(await textExists('Restart Service')).toBe(true);
    expect(await textExists('Uninstall Service')).toBe(true);
  });

  it('installs the service from the gate', async () => {
    stepLog('Clicking Install Service');
    await clickButton('Install Service');
    await waitForServiceStateText('Stopped');

    const state = await readMockState();
    expect(state.installed).toBe(true);
    expect(state.running).toBe(false);
  });

  it('starts the service from the gate', async () => {
    stepLog('Clicking Start Service');
    await clickButton('Start Service');
    await waitForServiceStateText('Running');

    const state = await readMockState();
    expect(state.installed).toBe(true);
    expect(state.running).toBe(true);
  });

  it('stops the service from the gate', async () => {
    stepLog('Clicking Stop Service');
    await clickButton('Stop Service');
    await waitForServiceStateText('Stopped');

    const state = await readMockState();
    expect(state.running).toBe(false);
  });

  it('restarts the service from the gate', async () => {
    stepLog('Clicking Restart Service');
    await clickButton('Restart Service');
    await waitForServiceStateText('Running');

    const state = await readMockState();
    expect(state.running).toBe(true);
  });

  it('keeps user blocked and surfaces error when core start fails', async () => {
    const state = await readMockState();
    await writeMockState({
      ...state,
      running: false,
      failures: { ...state.failures, start: 'simulated start failure' },
    });

    stepLog('Injecting start failure and refreshing gate');
    await clickButton('Refresh');
    await waitForServiceStateText('Stopped');

    stepLog('Attempting start while failure is injected');
    await clickButton('Start Service');
    await waitForText('simulated start failure', 10_000);
    await waitForText('OpenHuman Service Required', 10_000);

    const latest = await readMockState();
    expect(latest.running).toBe(false);
  });

  it('uninstalls the service from the gate', async () => {
    const state = await readMockState();
    await writeMockState({
      ...state,
      failures: { ...state.failures, start: undefined },
    });

    stepLog('Clicking Uninstall Service');
    await clickButton('Uninstall Service');
    await waitForServiceStateText('NotInstalled');

    const latest = await readMockState();
    expect(latest.installed).toBe(false);
    expect(latest.running).toBe(false);
  });

  it('unblocks once service is running and agent is reported healthy', async () => {
    const state = await readMockState();
    await writeMockState({
      ...state,
      installed: true,
      running: true,
      agent_running: true,
      failures: {},
    });

    stepLog('Making service + agent healthy and refreshing');
    await clickButton('Refresh');

    await browser.waitUntil(async () => !(await textExists('OpenHuman Service Required')), {
      timeout: 20_000,
      timeoutMsg: 'Service blocking gate did not clear after healthy status',
    });
    stepLog('Gate cleared as expected');
  });

  it('shows blocking gate again when connection suddenly breaks', async () => {
    const state = await readMockState();
    await writeMockState({
      ...state,
      installed: true,
      running: false,
      agent_running: false,
      failures: {},
    });
    stepLog('Injected sudden disconnect state; waiting for polling-based gate re-block');

    await browser.waitUntil(async () => textExists('OpenHuman Service Required'), {
      timeout: 20_000,
      interval: 500,
      timeoutMsg: 'Service blocking gate did not reappear after sudden disconnect',
    });

    await waitForServiceStateText('Stopped');
    await waitForText('Not Running', 10_000);
    stepLog('Gate reappeared with disconnected state');
  });
});
