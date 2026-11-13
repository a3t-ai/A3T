// Who relies on a resource: the agents whose settings name it (model, tool
// servers, memory), the canvases that depend on it, the triggers that read an
// ADB topic through it. What you want to know before you change or delete one.
import { agentTool, mcpCall, canvasApi } from '$lib/services/api';

export interface Use { kind: 'agent' | 'canvas' | 'trigger'; id: string; name: string; project: string; how: string }

const parse = (v: unknown): any => { if (typeof v !== 'string') return v ?? {}; try { return JSON.parse(v); } catch { return {}; } };

/// The setting of an agent record that names `ref`, in words.
function agentHow(a: any, ref: string): string | null {
  const rb = parse(a.resource_bindings), vars = parse(a.variables);
  const hits: string[] = [];
  if (rb.llm === ref || vars.llm_resource === ref) hits.push('its model');
  if ((vars.mcp_servers ?? []).includes?.(ref) || (rb.mcp ?? []).includes?.(ref)) hits.push('a tool server');
  if (rb.memory === ref || vars.memory_resource === ref) hits.push('its memory');
  if (!hits.length && JSON.stringify(a).includes(`"${ref}"`)) hits.push('its settings');
  return hits.length ? hits.join(', ') : null;
}

export async function usedBy(ref: string): Promise<Use[]> {
  const [ag, cv, tr] = await Promise.all([
    agentTool('agent_a3t_agents', 'tool_a3t_agent_list', { org_id: 'org-default' }),
    canvasApi.list(),
    mcpCall('list_triggers', {}),
  ]);
  const out: Use[] = [];
  for (const a of Array.isArray(ag.data) ? ag.data : []) {
    const how = agentHow(a, ref);
    if (how) out.push({ kind: 'agent', id: a.id, name: a.name ?? a.id, project: a.namespace ?? '', how });
  }
  const canvases: any[] = cv.ok ? (Array.isArray(cv.data) ? cv.data : (cv.data?.resources ?? [])) : [];
  for (const c of canvases) {
    if ((c.config?.health?.dependsOn ?? []).some((d: any) => d?.resource === ref))
      out.push({ kind: 'canvas', id: `${c.namespace}/${c.name}`, name: c.name, project: c.namespace, how: `depends on it (${c.config?.flowRef || 'no flow'})` });
  }
  let t: any = tr.ok ? tr.data : [];
  if (typeof t === 'string') { try { t = JSON.parse(t); } catch { t = []; } }
  for (const g of Array.isArray(t) ? t : (t?.triggers ?? [])) {
    if (g.config?.type === 'adb_topic' && g.config?.resource === ref)
      out.push({ kind: 'trigger', id: g.id, name: g.name, project: g.namespace ?? '', how: `reads topic ${g.config.topic}` });
  }
  return out;
}
