// What the console can say about one agent, from the APIs that exist today:
// its record and template (what it is), its serving flow's runs, the decision
// log, the caller's own conversations with it, and its deployment's stats.
// Where an answer needs an API that does not exist yet, the gap is named
// (see .ai/A3T_UI_LIMITATIONS.md, UI-n) rather than faked.
import * as YAML from 'yaml';
import { audit, fetchChatSessions, opsApi, runsApi, type ChatSessionSummary } from '$lib/services/api';

export type AgentRec = {
  id: string; name: string; namespace: string; mode: string; status: string;
  template_id?: string; template_version?: string;
  variables?: string; resource_bindings?: string; created_at?: string; created_by?: string;
};
export type TemplateRec = { id: string; name: string; version: string; spec_yaml: string };

const json = (s: unknown): any => {
  if (s && typeof s === 'object') return s;
  try { return JSON.parse(String(s ?? '')) ?? {}; } catch { return {}; }
};

export type Hook = { name: string; extend: string; when?: string; flows: string[] };
export type Facts = {
  /** What it is for, in a sentence (the template's description). */
  purpose: string;
  template: { id: string; name: string; version: string } | null;
  target: string;          // the flow that serves it (ns/name)
  endpoint: string;        // its webhook path
  llm: string;
  mcp: string[];
  tools: string[] | null;  // allowed tools; null = not restricted
  prompt: string;
  hooks: Hook[];
  memory: Record<string, unknown> | null;   // memory settings in its config, if any
  spec: any;
};

/// What the agent is, from its record and its template's spec.
export function factsOf(agent: AgentRec, templates: TemplateRec[]): Facts {
  const vars = json(agent.variables);
  const rb = json(agent.resource_bindings);
  const tpl = templates.find((t) => t.id === agent.template_id) ?? null;
  let spec: any = {};
  try { spec = tpl?.spec_yaml ? YAML.parse(tpl.spec_yaml) ?? {} : {}; } catch { spec = {}; }
  // The call task's `with.config`: the template's defaults for model, tools, memory.
  let cfg: any = {};
  for (const task of spec.do ?? []) for (const def of Object.values(task ?? {}) as any[]) if (def?.call && typeof def.with?.config === 'object') cfg = def.with.config;
  const hooks: Hook[] = (spec.use?.extensions ?? []).map((ext: any, i: number) => {
    const name = typeof ext === 'object' ? Object.keys(ext)[0] : `ext-${i}`;
    const d = (typeof ext === 'object' ? ext[name] : {}) ?? {};
    const flows: string[] = [];
    const collect = (list: any) => { for (const t of Array.isArray(list) ? list : []) for (const v of Object.values(t ?? {}) as any[]) { const w = v?.run?.workflow; if (w?.name) flows.push(`${w.namespace ?? ''}/${w.name}`.replace(/^\//, '')); else if (v?.call) flows.push(String(v.call)); } };
    collect(d.do); collect(d.before); collect(d.after);
    return { name, extend: String(d.extend ?? 'all'), when: d.when, flows };
  });
  return {
    purpose: spec.document?.metadata?.description ?? spec.document?.summary ?? '',
    template: tpl ? { id: tpl.id, name: tpl.name, version: tpl.version } : null,
    target: vars.target ?? '',
    endpoint: vars.ep_path ?? '',
    llm: rb.llm ?? cfg.llm_resource ?? '',
    mcp: vars.mcp_servers ?? cfg.mcp_servers ?? [],
    tools: Array.isArray(vars.enabled_tools) ? vars.enabled_tools : null,
    prompt: vars.system_prompt ?? cfg.system_prompt ?? '',
    hooks,
    memory: (cfg.memory && typeof cfg.memory === 'object') ? cfg.memory : null,
    spec,
  };
}

export type Run = { id: string; flow: string; status: string; started: string; ms: number | null; error: string };
/// Runs of the flow that serves it. Agents can share a serving flow (the chat
/// engine), so these are the flow's runs, not only this agent's (UI-2).
export async function servingRuns(f: Facts, limit = 100): Promise<{ runs: Run[]; error: string }> {
  if (!f.target.includes('/')) return { runs: [], error: '' };
  const [namespace, ...rest] = f.target.split('/');
  const r = await runsApi.list({ namespace, name: rest.join('/'), limit });
  if (!r.ok) return { runs: [], error: r.error ?? 'Runs could not be read' };
  const rows = Array.isArray(r.data?.runs) ? r.data.runs : [];
  return {
    runs: rows.map((x: any) => ({
      id: x.run_id, flow: `${x.namespace}/${x.flow}`, status: String(x.status ?? '').toLowerCase(), started: x.started_at ?? x.created_at ?? '',
      ms: x.duration_us != null ? x.duration_us / 1000 : x.completed_at && x.started_at ? Date.parse(x.completed_at) - Date.parse(x.started_at) : null,
      error: x.error ? String(typeof x.error === 'string' ? x.error : x.error.message ?? JSON.stringify(x.error)) : '',
    })),
    error: '',
  };
}

export type Decision = { ts: number; allow: boolean; principal: string; obo: string; action: string; resource: string; reason: string };
/// Authorization decisions that involve this agent: as the principal, acting on
/// behalf of someone, or as the resource. Read from the latest `limit` records of
/// the whole log and filtered here (UI-3: no server-side filter yet).
export async function decisionsFor(agent: AgentRec, limit = 500): Promise<{ list: Decision[]; error: string; scanned: number }> {
  const r = await audit.recent(limit);
  if (!r.ok) return { list: [], error: r.status === 403 ? 'The decision log needs an admin or security role.' : (r.error ?? 'Decision log unavailable'), scanned: 0 };
  const rows: any[] = Array.isArray(r.data) ? r.data : (r.data?.records ?? r.data?.decisions ?? []);
  const id = agent.id.toLowerCase();
  const hit = (s: unknown) => typeof s === 'string' && s.toLowerCase().includes(id);
  return {
    list: rows.filter((x) => hit(x.principal_id) || hit(x.on_behalf_of) || hit(x.resource_id) || hit(x.context?.agent_id)).map((x) => ({
      ts: x.ts_ms ?? 0,
      allow: String(x.decision ?? '').toLowerCase() === 'allow',
      principal: `${x.principal_kind ?? ''}:${x.principal_id ?? ''}`, obo: x.on_behalf_of ?? '',
      action: x.action ?? '', resource: `${x.resource_type ?? ''}:${x.resource_id ?? ''}`, reason: x.reason ?? (Array.isArray(x.policy_ids) ? x.policy_ids.join(', ') : ''),
    })),
    error: '', scanned: rows.length,
  };
}

/// Your own conversations with it (server ids are "<agent>:<client id>"). Other
/// users' conversations are not readable from here (UI-4).
export async function conversationsWith(agent: AgentRec): Promise<ChatSessionSummary[]> {
  const all = await fetchChatSessions();
  return all.filter((s) => s.session_id.startsWith(`${agent.id}:`));
}

export type Placement = {
  /** The deployment's id, for pause / resume / undeploy. */
  id: string;
  runtime: string; runtimeState: string; serving: string; deploymentState: string; canvas: string;
  stats: { runs: number; inFlight: number; failed: number; refused: number; completed: number; runsLastHour: number; errorRate: number; p50Ms?: number; p95Ms?: number; lastRunAt?: string; lastError?: string } | null;
  host: { cpu?: number; cpuLimit?: number; memUsed?: number; memLimit?: number } | null;
};
/// Where it is deployed and how that deployment is doing (the ops overview).
export async function placementOf(agent: AgentRec, f: Facts): Promise<{ placement: Placement | null; error: string }> {
  const r = await opsApi.overview();
  if (!r.ok) return { placement: null, error: r.error ?? 'Runtime overview unavailable' };
  const host = r.data?.host;
  const h = host ? { cpu: host.cpu?.coresUsed, cpuLimit: host.cpu?.limitCores, memUsed: host.memory?.usedBytes, memLimit: host.memory?.limitBytes } : null;
  const match = (row: any) => row?.deployment && (row.deployment.agentRef === agent.id || row.deployment.agentRef?.endsWith(`/${agent.id}`) || (!!f.target && row.deployment.flowRef === f.target && row.deployment.kind === 'agent'));
  for (const rt of r.data?.runtimes ?? []) {
    const row = (rt.deployments ?? []).find(match);
    if (row) return { placement: { id: row.deployment.id ?? '', runtime: rt.runtime?.name ?? '', runtimeState: rt.state, serving: row.serving, deploymentState: row.deployment.state, canvas: row.deployment.canvas, stats: row.stats ?? null, host: h }, error: '' };
  }
  const un = (r.data?.unplaced ?? []).find(match);
  if (un) return { placement: { id: un.deployment.id ?? '', runtime: '', runtimeState: '', serving: 'unplaced', deploymentState: un.deployment.state, canvas: un.deployment.canvas, stats: un.stats ?? null, host: null }, error: '' };
  return { placement: null, error: '' };
}

/// Where an agent came from: made new from a template, cloned (an independent
/// copy) or forked (a copy that keeps its link to the parent). Read from its
/// variables; agents made before lineage was stored are "from a template".
export type Lineage = { kind: 'template' | 'clone' | 'fork'; from?: { id: string; name: string; exists: boolean } };
export function lineageOf(agent: AgentRec, all: { id: string; name: string }[]): Lineage {
  const v = json(agent.variables);
  const id = v.forkedFrom ?? v.clonedFrom;
  if (!id) return { kind: 'template' };
  const parent = all.find((a) => a.id === id);
  return { kind: v.forkedFrom ? 'fork' : 'clone', from: { id, name: parent?.name ?? v.fromName ?? id, exists: !!parent } };
}
/// Agents forked from this one (they follow it); clones are independent and not listed.
export function forksOf(agent: AgentRec, all: AgentRec[]): AgentRec[] {
  return all.filter((a) => json(a.variables).forkedFrom === agent.id);
}

// ── Formatting ──
export function ago(t: string | number): string {
  const ms = typeof t === 'number' ? t : Date.parse(t);
  if (!ms) return '—';
  const s = Math.round((Date.now() - ms) / 1000);
  if (s < 60) return 'just now';
  if (s < 3600) return `${Math.round(s / 60)} min ago`;
  if (s < 86400) return `${Math.round(s / 3600)} h ago`;
  return `${Math.round(s / 86400)} d ago`;
}
export const dur = (ms: number | null | undefined) => (ms == null ? '—' : ms < 1000 ? `${Math.round(ms)} ms` : ms < 60000 ? `${(ms / 1000).toFixed(1)} s` : `${(ms / 60000).toFixed(1)} min`);
export const bytes = (n?: number) => (n == null ? '—' : n > 1e9 ? `${(n / 1e9).toFixed(1)} GB` : `${Math.round(n / 1e6)} MB`);
/// "draft" and friends, in words.
export const STATUS_WORDS: Record<string, string> = {
  draft: 'Draft — defined, not deployed. It answers on its endpoint through the shared runtime until a canvas deploys it.',
  active: 'Active — deployed and answering.',
  running: 'Running — deployed and answering.',
  paused: 'Paused — deployed, but not taking new work.',
  stopped: 'Stopped — not answering.',
};
