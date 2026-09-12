import { COMPOSIO_KIND } from "./composio.jsx";
import { OPENAPI_KIND } from "./openapi.jsx";
import { MCP_KIND } from "./mcp.jsx";
import { SKILLS_KIND } from "./skills.jsx";

/** Tab order, left to right. A fifth plugin kind is one descriptor and one
 *  entry here - nothing in the view itself knows the difference. */
export const PLUGIN_KINDS = [COMPOSIO_KIND, OPENAPI_KIND, MCP_KIND, SKILLS_KIND];

export const PLUGIN_KIND_IDS = PLUGIN_KINDS.map((k) => k.id);

export function getPluginKind(id) {
  return PLUGIN_KINDS.find((k) => k.id === id) ?? PLUGIN_KINDS[0];
}
