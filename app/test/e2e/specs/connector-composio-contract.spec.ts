/**
 * E2E: Composio connector contract — one table-driven spec covering the
 * toolkits that share the same connector-card and state UI
 * (plan.md §2.2). Collapses 11 byte-identical connector-*.spec.ts files.
 *
 * Bespoke connectors keep their own specs:
 *   - connector-jira.spec.ts           (subdomain-field UI)
 */
import { type ConnectorContractConfig, runConnectorContract } from '../helpers/connector-contract';

const TOOLKITS: ConnectorContractConfig[] = [{ name: 'Slack', slug: 'slack', idBase: 'c-slack' }];

for (const toolkit of TOOLKITS) {
  runConnectorContract(toolkit);
}
