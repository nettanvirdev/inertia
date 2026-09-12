/**
 * App integrations and user-installed tool sources (MCP / OpenAPI).
 */

export const INTEGRATIONS = [
  {
    id: 'int-slack',
    name: 'Slack',
    category: 'communication',
    icon: 'MessageSquare',
    provider: 'native',
    connected: true,
    account: 'inertia.slack.com',
    scopes: ['channels:read', 'chat:write', 'files:write', 'users:read'],
    toolCount: 11,
    lastSyncAt: '2026-09-01T14:12:00Z',
    description:
      'Post to channels, read history, and open threads. Agents use it for incident timelines, the morning brief and the inbox digest.',
  },
  {
    id: 'int-gmail',
    name: 'Gmail',
    category: 'communication',
    icon: 'Mail',
    provider: 'composio',
    connected: true,
    account: 'support@inertia.dev (shared)',
    scopes: ['gmail.readonly', 'gmail.compose', 'gmail.labels'],
    toolCount: 14,
    lastSyncAt: '2026-09-01T13:00:00Z',
    description:
      'Read and label the shared inbox and write drafts. Send scope is deliberately not granted - every reply waits for a human.',
  },
  {
    id: 'int-github',
    name: 'GitHub',
    category: 'dev',
    icon: 'Github',
    provider: 'native',
    connected: true,
    account: 'inertia (org) · 12 repos',
    scopes: ['repo', 'workflow', 'read:org', 'issues:write'],
    toolCount: 23,
    lastSyncAt: '2026-09-01T14:16:00Z',
    description:
      'Branches, pull requests, issues and Actions runs. Forge and Sable both live here; write access is scoped to non-protected branches.',
  },
  {
    id: 'int-linear',
    name: 'Linear',
    category: 'productivity',
    icon: 'ListTodo',
    provider: 'composio',
    connected: true,
    account: 'Inertia · INE workspace',
    scopes: ['issues:read', 'issues:write', 'comments:write'],
    toolCount: 9,
    lastSyncAt: '2026-09-01T11:24:00Z',
    description: 'File and update issues from QA runs and design sweeps, with screenshots attached to the issue body.',
  },
  {
    id: 'int-notion',
    name: 'Notion',
    category: 'productivity',
    icon: 'NotebookText',
    provider: 'composio',
    connected: true,
    account: 'Inertia HQ',
    scopes: ['pages:read', 'pages:write', 'search'],
    toolCount: 7,
    lastSyncAt: '2026-08-31T19:40:00Z',
    description: 'Read specs and runbooks, and file postmortems into the Incidents database.',
  },
  {
    id: 'int-stripe',
    name: 'Stripe',
    category: 'data',
    icon: 'CreditCard',
    provider: 'native',
    connected: true,
    account: 'acct_1PQm… (live, read-only)',
    scopes: ['customers:read', 'invoices:read', 'balance:read'],
    toolCount: 6,
    lastSyncAt: '2026-09-01T05:02:00Z',
    description:
      'Read-only access to customers, invoices and balance transactions for reconciliation. No write scope anywhere in the workspace.',
  },
  {
    id: 'int-snowflake',
    name: 'Snowflake',
    category: 'data',
    icon: 'Database',
    provider: 'native',
    connected: true,
    account: 'INERTIA_PROD · role ANALYTICS_BOT',
    scopes: ['analytics:read', 'analytics:write', 'raw:read'],
    toolCount: 5,
    lastSyncAt: '2026-09-01T02:19:00Z',
    description: 'Query and build in the analytics schema. Raw is read-only; DDL against raw is blocked at the role level.',
  },
  {
    id: 'int-gdrive',
    name: 'Google Drive',
    category: 'storage',
    icon: 'HardDrive',
    provider: 'composio',
    connected: true,
    account: 'ops@inertia.dev',
    scopes: ['drive.file', 'drive.readonly'],
    toolCount: 8,
    lastSyncAt: '2026-08-30T16:10:00Z',
    description: 'Read shared docs and drop generated briefs and reports into the Team Drive.',
  },
  {
    id: 'int-figma',
    name: 'Figma',
    category: 'dev',
    icon: 'Frame',
    provider: 'openapi',
    connected: true,
    account: 'Inertia Design · 4 files',
    scopes: ['file_read', 'file_comments:write'],
    toolCount: 4,
    lastSyncAt: '2026-08-31T16:18:00Z',
    description: 'Pull frames and component specs for visual diffing, and leave comments on drifting frames.',
    endpoint: 'https://api.figma.com/openapi.json',
  },
  {
    id: 'int-pagerduty',
    name: 'PagerDuty',
    category: 'dev',
    icon: 'Siren',
    provider: 'pipedream',
    connected: true,
    account: 'inertia · infra-primary schedule',
    scopes: ['incidents:read', 'oncall:read'],
    toolCount: 5,
    lastSyncAt: '2026-09-01T14:20:00Z',
    description: 'Read open incidents and the current on-call rotation. Acknowledging and resolving stays with humans.',
  },
  {
    id: 'int-hubspot',
    name: 'HubSpot',
    category: 'crm',
    icon: 'Contact',
    provider: 'composio',
    connected: false,
    account: null,
    scopes: ['crm.objects.contacts.read', 'crm.objects.deals.read'],
    toolCount: 12,
    lastSyncAt: null,
    description: 'Deal and contact context for sales handoffs. Waiting on an admin to approve the OAuth app.',
  },
  {
    id: 'int-jira',
    name: 'Jira',
    category: 'productivity',
    icon: 'SquareKanban',
    provider: 'composio',
    connected: false,
    account: null,
    scopes: ['read:jira-work', 'write:jira-work'],
    toolCount: 16,
    lastSyncAt: null,
    description: 'Not connected - the team standardised on Linear. Kept in the catalogue for customers migrating in.',
  },
  {
    id: 'int-dropbox',
    name: 'Dropbox',
    category: 'storage',
    icon: 'Package',
    provider: 'pipedream',
    connected: false,
    account: null,
    scopes: ['files.content.read', 'files.content.write'],
    toolCount: 6,
    lastSyncAt: null,
    description: 'Alternative file storage. Disconnected since the team moved everything to Google Drive in June.',
  },
  {
    id: 'int-salesforce',
    name: 'Salesforce',
    category: 'crm',
    icon: 'Cloud',
    provider: 'composio',
    connected: false,
    account: null,
    scopes: ['api', 'refresh_token'],
    toolCount: 21,
    lastSyncAt: null,
    description: 'Enterprise CRM connector. Never enabled in this workspace.',
  },
  {
    id: 'int-mcp-warehouse',
    name: 'Warehouse MCP',
    category: 'data',
    icon: 'Server',
    provider: 'mcp',
    connected: true,
    account: 'self-hosted · bearer auth',
    scopes: ['query', 'describe', 'lineage'],
    toolCount: 6,
    lastSyncAt: '2026-09-01T02:00:00Z',
    description:
      'Internal MCP server wrapping the warehouse. Adds dbt lineage lookups and a safe query tool with a 30-second statement timeout.',
    endpoint: 'https://mcp.internal.inertia.dev/warehouse',
  },
  {
    id: 'int-mcp-playwright',
    name: 'Playwright MCP',
    category: 'dev',
    icon: 'MousePointerClick',
    provider: 'mcp',
    connected: true,
    account: 'local stdio',
    scopes: ['browser.navigate', 'browser.click', 'browser.snapshot'],
    toolCount: 13,
    lastSyncAt: '2026-09-01T13:41:00Z',
    description:
      'Accessibility-tree browser driving used by Sable for the critical-path suite. Runs on the Build Box over stdio.',
    endpoint: 'stdio://npx @playwright/mcp@latest',
  },
  {
    id: 'int-openapi-statuspage',
    name: 'Statuspage API',
    category: 'communication',
    icon: 'Activity',
    provider: 'openapi',
    connected: false,
    account: null,
    scopes: ['incidents:write', 'components:write'],
    toolCount: 7,
    lastSyncAt: null,
    description:
      'Imported OpenAPI spec for posting customer-facing incident updates. Installed but intentionally left disconnected until the escalation policy is signed off.',
    endpoint: 'https://api.statuspage.io/v1/openapi.json',
  },
];

export const INTEGRATION_CATEGORY_META = {
  communication: { label: 'Communication', icon: 'MessagesSquare' },
  dev: { label: 'Developer', icon: 'Code' },
  productivity: { label: 'Productivity', icon: 'CheckSquare' },
  data: { label: 'Data', icon: 'Database' },
  storage: { label: 'Storage', icon: 'FolderOpen' },
  crm: { label: 'CRM', icon: 'Contact' },
};

export const INTEGRATION_PROVIDER_META = {
  composio: { label: 'Composio', icon: 'Blocks' },
  pipedream: { label: 'Pipedream', icon: 'Workflow' },
  mcp: { label: 'MCP server', icon: 'Server' },
  openapi: { label: 'OpenAPI', icon: 'FileJson' },
  native: { label: 'Built in', icon: 'Puzzle' },
};

export function getIntegrationById(id) {
  return INTEGRATIONS.find((i) => i.id === id);
}

export function getConnectedIntegrations() {
  return INTEGRATIONS.filter((i) => i.connected);
}

export function getIntegrationsByCategory(category) {
  return INTEGRATIONS.filter((i) => i.category === category);
}

/** MCP and OpenAPI sources the user installed themselves. */
export function getCustomToolSources() {
  return INTEGRATIONS.filter((i) => i.provider === 'mcp' || i.provider === 'openapi');
}

export function getTotalToolCount() {
  return getConnectedIntegrations().reduce((sum, i) => sum + i.toolCount, 0);
}
