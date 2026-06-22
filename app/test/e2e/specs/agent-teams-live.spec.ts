import { waitForApp, waitForAppReady } from '../helpers/app-helpers';
import { callOpenhumanRpc, expectRpcOk } from '../helpers/core-rpc';
import { triggerAuthDeepLinkBypass } from '../helpers/deep-link-helpers';
import {
  clickText,
  textExists,
  waitForText,
  waitForWebView,
  waitForWindowVisible,
} from '../helpers/element-helpers';
import { supportsExecuteScript } from '../helpers/platform';
import { completeOnboardingIfVisible, navigateViaHash } from '../helpers/shared-flows';
import { startMockServer, stopMockServer } from '../mock-server';

/**
 * Agent-teams live coordination surface spec (#3374 PR4).
 *
 * Goal: prove the Intelligence → Teams tab renders a seeded team's header,
 * task board and teammate-message timeline, and exposes the PR4 interactive
 * affordances — the message composer and the per-member live-start control.
 * The team + task + lead message are seeded over the core JSON-RPC the app
 * already talks to (driver-agnostic HTTP). The end-to-end *completion* of a
 * live teammate run is asserted headlessly in the Rust JSON-RPC e2e
 * (`json_rpc_agent_team_live_member_run_roundtrip`); this spec focuses on the
 * UI surface.
 *
 * Mac2 skipped — the Intelligence pane is not mapped to Appium helpers
 * (mirrors `insights-dashboard.spec.ts`); runs under tauri-driver on Linux CI.
 */
function stepLog(message: string, context?: unknown): void {
  const stamp = new Date().toISOString();
  if (context === undefined) {
    console.log(`[AgentTeamsLiveE2E][${stamp}] ${message}`);
    return;
  }
  console.log(`[AgentTeamsLiveE2E][${stamp}] ${message}`, JSON.stringify(context, null, 2));
}

interface SeededTeam {
  teamId: string;
  memberId: string;
  taskTitle: string;
  summary: string;
  messageBody: string;
}

interface TeamCreateResult {
  team: { id: string };
  members: Array<{ id: string; name: string }>;
}
interface TaskAssignResult {
  task: { id: string };
}

describe('Agent teams live coordination surface', () => {
  let seeded: SeededTeam | null = null;

  before(async function beforeSuite() {
    this.timeout(90_000);
    if (!supportsExecuteScript()) {
      stepLog('Skipping suite on Mac2 — Intelligence pane not mapped');
      this.skip();
    }

    stepLog('starting mock server');
    await startMockServer();
    stepLog('waiting for app');
    await waitForApp();
    stepLog('triggering auth bypass deep link');
    await triggerAuthDeepLinkBypass('e2e-agent-teams-live');
    await waitForWindowVisible(25_000);
    await waitForWebView(15_000);
    await waitForAppReady(15_000);
    await completeOnboardingIfVisible('[AgentTeamsLiveE2E]');

    // Seed a team + member + task + a lead-originated message over the core RPC.
    const summary = `E2E team ${Date.now()}`;
    const taskTitle = 'Draft the launch note';
    const messageBody = 'kick off the draft when ready';

    stepLog('seeding team');
    const created = await callOpenhumanRpc<TeamCreateResult>('openhuman.agent_team_create', {
      leadAgentId: 'lead',
      summary,
      members: [{ name: 'scout', agentId: 'researcher' }],
    });
    expectRpcOk('agent_team_create', created);
    const teamId = created.result!.team.id;
    const memberId = created.result!.members[0].id;

    stepLog('seeding task');
    const assigned = await callOpenhumanRpc<TaskAssignResult>('openhuman.agent_team_assign_task', {
      teamId,
      title: taskTitle,
      ownerMemberId: memberId,
      dependsOn: [],
    });
    expectRpcOk('agent_team_assign_task', assigned);

    stepLog('seeding lead message');
    const messaged = await callOpenhumanRpc('openhuman.agent_team_message_member', {
      teamId,
      toMemberId: memberId,
      content: messageBody,
    });
    expectRpcOk('agent_team_message_member', messaged);

    seeded = { teamId, memberId, taskTitle, summary, messageBody };
    stepLog('seeded', seeded);
  });

  after(async () => {
    stepLog('stopping mock server');
    await stopMockServer();
  });

  it('renders the seeded team header and task board on the Teams tab', async () => {
    if (!seeded) throw new Error('seed missing');
    stepLog('navigating to /intelligence');
    await navigateViaHash('/intelligence');
    await waitForWebView(15_000);

    stepLog('opening Teams tab');
    await clickText('Teams', 10_000);

    // Team summary (list or auto-selected header) + the owned task title.
    await waitForText(seeded.summary, 15_000);
    await waitForText(seeded.taskTitle, 15_000);
    stepLog('team header + task rendered');
  });

  it('shows the lead message in the activity timeline', async () => {
    if (!seeded) throw new Error('seed missing');
    expect(await textExists(seeded.messageBody)).toBe(true);
  });

  it('exposes the PR4 composer affordance', async () => {
    // The composer footer placeholder proves the interactive send surface is
    // mounted on the active team (read-only PR2 had no input).
    await waitForText('Message a teammate', 10_000);
    stepLog('composer affordance present');
  });
});
