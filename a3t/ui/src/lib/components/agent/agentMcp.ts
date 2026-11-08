// Each agent is exposed as an MCP server of its own: whoever connects —
// Claude Code, VS Code, a frontend, another agent — sees the agent as a tool
// whose input schema is its template's `input` and whose description is the
// agent's. The server does not serve this yet (SRV-19); this is what it will
// serve, and what the Exposure tab shows as Sample.
import type { Field } from '$lib/components/templates/templateSpec';
import { apiBase } from './agentApi';

export const mcpUrl = (agentName: string, project: string) => `${apiBase()}/agents/${project}/${agentName}/mcp`;

/// The tool name: the agent's name, as an MCP tool name.
export const toolName = (agentName: string) => agentName.replace(/[^a-zA-Z0-9_]/g, '_');

/// The tool an MCP client sees for the agent.
export function toolOf(agent: { name: string; description?: string }, purpose: string, inputs: Field[], outputs: Field[]) {
  const fields = inputs.length ? inputs : [{ name: 'message', type: 'string', required: true, description: 'What to ask it' }];
  const props: Record<string, unknown> = {};
  for (const f of fields) props[f.name] = { type: f.type === 'any' ? 'string' : f.type, ...(f.description ? { description: f.description } : {}) };
  props.conversation_id = { type: 'string', description: 'Keep a conversation going: send the same id again' };
  const tool: Record<string, unknown> = {
    name: toolName(agent.name),
    description: agent.description || purpose || `The ${agent.name} agent`,
    inputSchema: { type: 'object', properties: props, required: fields.filter((f) => f.required).map((f) => f.name) },
  };
  if (outputs.length) {
    const out: Record<string, unknown> = {};
    for (const f of outputs) out[f.name] = { type: f.type === 'any' ? 'string' : f.type, ...(f.description ? { description: f.description } : {}) };
    tool.outputSchema = { type: 'object', properties: out };
  }
  return tool;
}

export type Client = 'claude' | 'vscode' | 'ts';
export function connectSnippet(c: Client, name: string, url: string): string {
  if (c === 'claude') return `claude mcp add --transport http ${name} ${url}\n# then, in Claude Code: /mcp → ${name} → Authenticate`;
  if (c === 'vscode') return JSON.stringify({ servers: { [name]: { type: 'http', url } } }, null, 2) + '\n// .vscode/mcp.json';
  return `import { Client } from '@modelcontextprotocol/sdk/client/index.js';
import { StreamableHTTPClientTransport } from '@modelcontextprotocol/sdk/client/streamableHttp.js';

const client = new Client({ name: 'my-app', version: '1.0.0' });
await client.connect(new StreamableHTTPClientTransport(new URL('${url}'), {
  requestInit: { headers: { Authorization: \`Bearer \${process.env.A3T_KEY}\` } },
}));
const res = await client.callTool({ name: '${toolName(name)}', arguments: { message: 'Hello', conversation_id: 'user-42' } });`;
}

// ── A2A (Agent2Agent): other agents find it by its agent card and send it
// tasks. Not served yet either (SRV-19).
export const a2aUrl = (agentName: string, project: string) => `${apiBase()}/agents/${project}/${agentName}/a2a`;
export const cardUrl = (agentName: string, project: string) => `${apiBase()}/agents/${project}/${agentName}/.well-known/agent-card.json`;

/// The agent card other agents read: who it is, what it can do, how to call it.
export function agentCard(agent: { name: string; namespace: string; description?: string }, purpose: string, version: string, inputs: Field[]) {
  return {
    protocolVersion: '0.3.0',
    name: agent.name,
    description: agent.description || purpose || `The ${agent.name} agent`,
    url: a2aUrl(agent.name, agent.namespace),
    preferredTransport: 'JSONRPC',
    version,
    capabilities: { streaming: true, pushNotifications: false },
    defaultInputModes: ['text/plain', 'application/json'],
    defaultOutputModes: ['text/plain', 'application/json'],
    securitySchemes: { oauth: { type: 'oauth2', description: 'A3T sign-in' }, appKey: { type: 'http', scheme: 'bearer' } },
    security: [{ oauth: [] }, { appKey: [] }],
    skills: [{
      id: toolName(agent.name),
      name: agent.name,
      description: agent.description || purpose || '',
      tags: ['a3t'],
      ...(inputs.length ? { examples: [`{${inputs.map((i) => `"${i.name}": …`).join(', ')}}`] } : {}),
    }],
  };
}
