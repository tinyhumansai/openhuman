// @ts-nocheck
import { waitForApp } from '../helpers/app-helpers';
import { captureCheckpoint } from '../helpers/artifacts';
import {
  clickActionButton,
  clickCollectionRow,
  collectionSnapshot,
  detailSnapshot,
  installClipboardProbe,
  loadMore,
  openCoreRegistriesFromHome,
  openRegistryTab,
  readClipboardProbe,
  waitForCoreRegistriesPage,
  waitForDetailHeading,
  waitForText,
} from '../helpers/core-registries';
import { resetApp } from '../helpers/reset-app';
import { startMockServer, stopMockServer } from '../mock-server';

const USER_ID = 'e2e-m224-core-registries';
const PRIMARY_AGENT_KEY = 'agent.registry.001-primary';
const PRIMARY_AGENT_DETAIL = 'agent.registry.001-primary v1';
const PRIMARY_TOOL_KEY = 'tool.registry.reader';
const PRIMARY_TOOL_DETAIL = 'tool.registry.reader v1';
const PRIMARY_BINDING_KEY = 'binding.registry-primary';
const PRIMARY_BINDING_DETAIL = 'binding.registry-primary v2';
const PRIMARY_CONNECTOR_DETAIL = 'connector.registry.feed v2';
const PRIMARY_FINGERPRINT = 'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa';

describe('M2.2.4 Core registries live desktop flow', function () {
  this.timeout(120_000);

  before(async () => {
    await startMockServer(Number(process.env.E2E_MOCK_PORT || 18473));
    await waitForApp();
    await resetApp(USER_ID);
  });

  after(async () => {
    await stopMockServer();
  });

  it('walks the Core registries route through exact links, read-only state, freshness, and fingerprint copy', async () => {
    await openCoreRegistriesFromHome();
    await waitForCoreRegistriesPage();
    await waitForText('Load more agents');

    const initial = await collectionSnapshot();
    expect(initial.tabStates).toContain('Agents:selected');
    expect(initial.loadMoreButtons).toContain('Load more agents');
    expect(initial.text).toContain(
      'This screen never writes configuration, secrets, or runtime state.'
    );
    expect(initial.text).toContain('Observed at');
    expect(initial.text).toContain('Agents · Observed');
    expect(initial.text).not.toContain('Save');
    expect(initial.text).not.toContain('Apply');
    expect(initial.text).not.toContain('Delete');
    expect(initial.text).not.toContain('Approve');
    await captureCheckpoint('m224-registries-home');
    await loadMore('Load more agents');

    await clickCollectionRow('Agents', PRIMARY_AGENT_KEY);
    await waitForDetailHeading(PRIMARY_AGENT_DETAIL);
    const agent = await detailSnapshot();
    expect(agent.text).toContain('Agent lifecycle');
    expect(agent.text).toContain('Exact tool references');
    expect(agent.text).toContain(PRIMARY_TOOL_DETAIL);
    await captureCheckpoint('m224-agent-detail');

    await clickActionButton(PRIMARY_TOOL_DETAIL);
    await waitForDetailHeading(PRIMARY_TOOL_DETAIL);
    const toolDefinition = await detailSnapshot();
    expect(toolDefinition.text).toContain('Definition lifecycle');
    expect(toolDefinition.text).toContain('Schemas');
    expect(toolDefinition.text).toContain('Enabled');
    expect(toolDefinition.text).toContain('No tenant enablement returned');
    expect(toolDefinition.text).toContain(PRIMARY_TOOL_KEY);
    await captureCheckpoint('m224-tool-definition-detail');
    await loadMore('Load more definitions');

    await clickCollectionRow('Enablements', PRIMARY_TOOL_KEY);
    await waitForDetailHeading(PRIMARY_TOOL_DETAIL);
    const enablement = await detailSnapshot();
    expect(enablement.text).toContain('Enablement lifecycle');
    expect(enablement.text).toContain('Definition link');
    expect(enablement.text).toContain('Metadata Only');
    await captureCheckpoint('m224-tool-enablement-detail');

    await openRegistryTab('Connectors');
    await waitForText('Load more types');
    await waitForText('Load more bindings');
    const connectors = await collectionSnapshot();
    expect(connectors.tabStates).toContain('Connectors:selected');
    expect(connectors.loadMoreButtons).toContain('Load more types');
    expect(connectors.loadMoreButtons).toContain('Load more bindings');
    expect(connectors.text).toContain('Bound provider accounts and capability selections.');
    await captureCheckpoint('m224-connectors-collections');
    await loadMore('Load more types');
    await loadMore('Load more bindings');

    await clickCollectionRow('Bindings', PRIMARY_BINDING_KEY, [
      'v2 · connector.registry.feed v2',
      'Active',
    ]);
    await waitForDetailHeading(PRIMARY_BINDING_DETAIL);
    const binding = await detailSnapshot();
    expect(binding.text).toContain('Binding lifecycle');
    expect(binding.text).toContain('Logical references');
    expect(binding.text).toContain('Logical reference only; secret not displayed.');
    expect(binding.text).toContain('credential://registry/primary');
    expect(binding.text).toContain('config://registry/primary');
    expect(binding.text).toContain(PRIMARY_CONNECTOR_DETAIL);
    await captureCheckpoint('m224-binding-detail');

    await clickActionButton(PRIMARY_CONNECTOR_DETAIL);
    await waitForDetailHeading(PRIMARY_CONNECTOR_DETAIL);
    const connectorType = await detailSnapshot();
    expect(connectorType.text).toContain('Type lifecycle');
    expect(connectorType.text).toContain('Contracts');
    expect(connectorType.text).toContain('message.created@1');
    expect(connectorType.text).toContain('push');
    await captureCheckpoint('m224-connector-type-detail');

    await installClipboardProbe();
    await clickActionButton('Copy full fingerprint');
    await browser.waitUntil(async () => (await readClipboardProbe()) === PRIMARY_FINGERPRINT, {
      timeout: 10_000,
      interval: 250,
      timeoutMsg: 'Fingerprint copy did not write the expected value',
    });
  });
});
