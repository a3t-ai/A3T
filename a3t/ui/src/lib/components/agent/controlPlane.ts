// The agents control plane, as the dash calls it: one flow, a3t/flow_a3t_agent_control,
// one action per call (create, get, lineage, relate, compare, merge, evolve,
// shares, share, unshare, shared_with). Memory copies with history (fork, branch,
// promote) stay FlowR's REST routes under /agents/{id}; the flow records where an
// agent came from. See .ai/A3T_AGENTS_API_PLAN.md (branch core-agent).
import { execFlow, agentTool } from '$lib/services/api';
import { getSession } from '$lib/stores/auth';
import { apiBase } from './agentApi';

export type Relation = 'template' | 'inherit' | 'clone' | 'fork' | 'branch' | 'evolved';
export interface Parent { kind: 'template' | 'agent'; id: string; version?: string }
export interface Ancestor extends Parent { relation: Relation }
export interface Descendant { id: string; relation: Relation; since: string }
export interface Share { agent_id: string; target: string; can_call: string; can_inherit: string; can_fork: string; can_merge_back: string; granted_by: string; granted_at: string }
export interface Rights { call: boolean; inherit: boolean; fork: boolean; merge_back: boolean }
export interface Compare { mergeable: boolean; changed: string[]; next: 'merge' | 'evolve'; source: string }
export interface Endpoints { control: string; memory?: string; data: { chat: string; session_id: string; stream: string } }

async function ctl<T>(action: string, args: Record<string, unknown> = {}): Promise<{ ok: boolean; data?: T; error?: string }> {
  const s = getSession();
  const r = await execFlow('flow_a3t_agent_control', 'a3t', { action, org_id: s?.user?.org_id ?? 'org-default', user_id: s?.user?.id ?? '', ...args }, { silent: true });
  return r.ok ? { ok: true, data: r.data as T } : { ok: false, error: r.error };
}

export const control = {
  get: (id: string) => ctl<{ relation: { kind: Relation; parent: Parent } | null; visibility: 'private' | 'namespaces' | 'public'; shares: Share[]; endpoints: Endpoints }>('get', { id }),
  lineage: (id: string) => ctl<{ ancestors: Ancestor[]; descendants: Descendant[] }>('lineage', { id }),
  relate: (id: string, parent: Parent, relation: Relation) => ctl('relate', { id, parent, relation }),
  compare: (id: string) => ctl<Compare>('compare', { id }),
  merge: (id: string) => ctl<{ procedures: number; failed: unknown[] }>('merge', { id }),
  evolve: (id: string) => ctl('evolve', { id }),
  shares: (id: string) => ctl<{ shares: Share[] }>('shares', { id }),
  share: (id: string, target: string, rights: Rights) => ctl('share', { id, target, rights }),
  unshare: (id: string, target: string) => ctl('unshare', { id, target }),
};

export interface TurnSummary { turns: number; failed: number; degraded: number; tool_calls: number; avg_ms: number; last_at: string | null }
export interface Turn { at: string; session_id: string; outcome: string; iterations: string; tool_calls: string; duration_ms: string; llm: string; knowledge: string; tool_hints: string; degraded: string }
export type MemoryType = 'working' | 'episodic' | 'semantic' | 'procedural';

/// The agent's own figures and memory, from agent_a3t_agents's tools (AT-1).
export const insight = {
  turns: async (id: string) => {
    const r = await agentTool('agent_a3t_agents', 'tool_a3t_agent_turns', { id });
    return r.ok ? { ok: true as const, data: r.data as { summary: TurnSummary; turns: Turn[] } } : { ok: false as const, error: r.error };
  },
  memory: async (id: string) => {
    const r = await agentTool('agent_a3t_agents', 'tool_a3t_agent_memory', { id });
    return r.ok ? { ok: true as const, data: r.data as { namespace: string; counts: Record<MemoryType, number>; memory: Record<MemoryType, any[]> } } : { ok: false as const, error: r.error };
  },
  /// Its history (agent_events), newest first: who changed what.
  events: async (id: string) => {
    const r = await agentTool('agent_a3t_agents', 'tool_a3t_agent_events', { id });
    if (!r.ok) return { ok: false as const, error: r.error };
    const rows = (Array.isArray(r.data) ? r.data : []) as { event_type: string; payload: string; created_at: string }[];
    const evs: AgentEvent[] = rows.map((x) => {
      let p: any = {}; try { p = JSON.parse(x.payload || '{}'); } catch { /* kept empty */ }
      return { type: x.event_type, at: x.created_at, by: p.by ?? '', payload: p };
    }).sort((a, b) => b.at.localeCompare(a.at));
    return { ok: true as const, data: evs };
  },
};
export interface AgentEvent { type: string; at: string; by: string; payload: any }

/// Stop and start its data plane, through agent_a3t_agents. Blocked, every call to
/// it (POST /chat, its webhook) is refused; its record and memory stay.
export async function setBlocked(id: string, blocked: boolean, reason?: string) {
  const s = getSession();
  const r = await agentTool('agent_a3t_agents', blocked ? 'tool_a3t_agent_block' : 'tool_a3t_agent_unblock',
    { org_id: s?.user?.org_id ?? 'org-default', id, ...(reason ? { reason } : {}) });
  return r.ok ? { ok: true as const, status: (r.data as { status: string })?.status } : { ok: false as const, error: r.error };
}

/// One line for an event of its history.
export function describeEvent(e: AgentEvent): string {
  const p = e.payload ?? {};
  switch (e.type) {
    case 'created': return p.from?.kind === 'agent' ? `made from ${p.from.id} (${p.relation ?? 'copy'})` : `created from ${p.from?.id ?? p.template ?? 'its template'}`;
    case 'updated': return (p.changed ?? []).length ? `changed ${(p.changed as string[]).map((c) => c.replace(/^values\./, '')).join(', ')}` : 'saved, no change';
    case 'blocked': return `blocked${p.reason ? `: ${p.reason}` : ''}`;
    case 'unblocked': return 'unblocked';
    case 'shared': return `shared with ${p.target === '*' ? 'everyone (public)' : p.target}`;
    case 'unshared': return `no longer shared with ${p.target === '*' ? 'everyone' : p.target}`;
    case 'merged': return `procedures merged in from ${p.from} (${p.procedures ?? 0})`;
    default: return e.type;
  }
}

export const rightsOf = (s: Share): Rights => ({ call: s.can_call === 'true', inherit: s.can_inherit === 'true', fork: s.can_fork === 'true', merge_back: s.can_merge_back === 'true' });

/// The agent's two addresses, as an app outside A3T uses them. Per-agent hostnames
/// (agt-xxxx.run.a3t.io) wait for the gateway; until then both are console paths.
export function addresses(id: string) {
  const base = apiBase();
  return {
    control: `${base}/agents/${id}`,
    data: `${base}/chat`,
    stream: `${base}/chat/stream/${id}:<session>`,
    session: `${id}:<session>`,
  };
}

export function chatCurl(id: string): string {
  const a = addresses(id);
  return [
    `curl -N ${a.data} \\`,
    `  -H "Authorization: Bearer $A3T_TOKEN" \\`,
    `  -H "Content-Type: application/json" \\`,
    `  -d '{"session_id":"${id}:s-1","message":"Hello"}'`,
  ].join('\n');
}
