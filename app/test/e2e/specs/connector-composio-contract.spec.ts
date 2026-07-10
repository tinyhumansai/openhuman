/**
 * E2E: Composio connector contract — one table-driven spec covering the
 * toolkits that share the identical connect/sync/execute/disconnect flow
 * (plan.md §2.2). Collapses 11 byte-identical connector-*.spec.ts files.
 *
 * Bespoke connectors keep their own specs:
 *   - connector-jira.spec.ts           (subdomain-field UI)
 *   - connector-gmail-composio.spec.ts (400-on-fetch-emails handling)
 */
import { type ConnectorContractConfig, runConnectorContract } from '../helpers/connector-contract';

const TOOLKITS: ConnectorContractConfig[] = [
  {
    name: 'Airtable',
    slug: 'airtable',
    idBase: 'c-airtable',
    executeAction: 'AIRTABLE_LIST_BASES',
  },
  { name: 'Asana', slug: 'asana', idBase: 'c-asana', executeAction: 'ASANA_LIST_TASKS' },
  { name: 'ClickUp', slug: 'clickup', idBase: 'c-clickup', executeAction: 'CLICKUP_LIST_TASKS' },
  {
    name: 'Confluence',
    slug: 'confluence',
    idBase: 'c-confluence',
    executeAction: 'CONFLUENCE_LIST_PAGES',
  },
  {
    name: 'Google Calendar',
    slug: 'googlecalendar',
    idBase: 'c-gcal',
    executeAction: 'GOOGLECALENDAR_LIST_EVENTS',
  },
  {
    name: 'Google Drive',
    slug: 'googledrive',
    idBase: 'c-gdrive',
    executeAction: 'GOOGLEDRIVE_LIST_FILES',
  },
  {
    name: 'Google Sheets',
    slug: 'googlesheets',
    idBase: 'c-gsheets',
    executeAction: 'GOOGLESHEETS_LIST_SPREADSHEETS',
  },
  { name: 'Notion', slug: 'notion', idBase: 'c-notion', executeAction: 'NOTION_LIST_PAGES' },
  { name: 'Slack', slug: 'slack', idBase: 'c-slack', executeAction: 'SLACK_LIST_CHANNELS' },
  { name: 'Todoist', slug: 'todoist', idBase: 'c-todoist', executeAction: 'TODOIST_LIST_PROJECTS' },
  {
    name: 'YouTube',
    slug: 'youtube',
    idBase: 'c-youtube',
    executeAction: 'YOUTUBE_LIST_PLAYLISTS',
  },
];

for (const toolkit of TOOLKITS) {
  runConnectorContract(toolkit);
}
