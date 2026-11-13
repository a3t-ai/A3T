// The agents control plane, as the dash calls it: one flow, a3t/flow_a3t_agent_control,
// one action per call (create, get, lineage, relate, compare, merge, evolve,
// shares, share, unshare, shared_with). Memory copies with history (fork, branch,
// promote) stay FlowR's REST routes under /agents/{id}; the flow records where an
// agent came from. See .ai/A3T_AGENTS_API_PLAN.md (branch core-agent).
import { execFlow } from '$lib/services/api';
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
