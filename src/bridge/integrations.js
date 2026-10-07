import { call, subscribe } from "./envelope";

/**
 * The three tool sources: MCP servers, OpenAPI imports and Composio apps.
 *
 * All three, together, or the Integrations screen shows nothing: `isDesktop()`
 * in `lib/integrations.js` is `composioAPI && mcpAPI && openapiAPI`, so one
 * missing namespace blanked the whole pane and told the user it needed a
 * desktop app they were already running.
 */

/* -- MCP ----------------------------------------------------------------- */

export function mcpBridge() {
  return {
    list: () => call("mcp_list"),
    add: (record) => call("mcp_save", { record: record ?? {} }),

    /**
     * Two parameters, deliberately.
     *
     * `lib/integrations.js` checks `bridge.update.length >= 2` and falls back to
     * patching the record through the workspace bridge when it is 1 - a
     * workaround for an Electron preload that forwarded only the first
     * argument. Taking both means the direct path is used and the fallback
     * stays dormant.
     */
    update: (id, changes) => call("mcp_save_patch", { id, changes: changes ?? {} }),

    remove: (id) => call("mcp_delete", { id }),
    connect: (id) => call("mcp_connect", { id }),
    disconnect: (id) => call("mcp_disconnect", { id }),
    test: (record) => call("mcp_test", { record: record ?? {} }),
    status: () => call("mcp_status"),
    tools: (id) => call("mcp_tools", { id }),
  };
}

/* -- OpenAPI ------------------------------------------------------------- */

export function openapiBridge() {
  return {
    list: () => call("openapi_list"),
    import: (input) => call("openapi_import", { record: input ?? {} }),
    remove: (id) => call("openapi_delete", { id }),
    update: (id, changes) => call("openapi_update", { id, changes: changes ?? {} }),
    operations: (id) => call("openapi_operations", { id }),
    setOperations: (id, operations) =>
      call("openapi_set_operations", { id, operations: operations ?? [] }),
    test: (id, operationId, args) => call("openapi_test", { id, operationId, args: args ?? {} }),
    details: (id) => call("openapi_details", { id }),
  };
}

/* -- Composio ------------------------------------------------------------ */

export function composioBridge() {
  return {
    configured: () => call("composio_configured"),
    // The query is an object - `{ search, category, refresh }` - and passing
    // its `search` alone would silently drop Refresh on the floor.
    toolkits: (query) => call("composio_toolkits", { query: query ?? {} }),
    tools: (toolkitSlug) => call("composio_tools", { toolkitSlug }),
    logo: (url) => call("composio_logo", { url }),
    connections: () => call("composio_connections"),
    connect: (toolkitSlug) => call("composio_start_connection", { toolkit: toolkitSlug }),
    status: (id) => call("composio_connection_status", { id }),
    disconnect: (id) => call("composio_disconnect", { id }),
    permissions: (id) => call("composio_permissions", { id }),
    setPermissions: (id, patch) => call("composio_set_permissions", { id, patch: patch ?? {} }),
    // Two different acts, and they were pointed at one command: `reconnect`
    // starts the handshake again for a row that already exists, `refresh`
    // only drops the cached tool lists. Sharing `composio_refresh` meant
    // Refresh in the tool picker called it with no `id` and failed.
    reconnect: (id) => call("composio_reconnect", { id }),
    sync: () => call("composio_load_all"),
    refresh: (toolkitSlug) => call("composio_invalidate", { toolkitSlug }),

    onEvent: (callback) => subscribe("composio:event", callback),
  };
}
