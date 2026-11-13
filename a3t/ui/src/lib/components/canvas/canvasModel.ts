// A canvas: one flow's deployment — where it loads (placement), how it runs
// (lifecycle) and what it depends on (health). The a3t.io/v1alpha1 CanvasSpec,
// mirrored from flowr-types::canvas. The editor, its tabs and the canvas graph
// in the designer all read it from here.
import * as YAML from 'yaml';
import { mcpCall, opsApi } from '$lib/services/api';

export type Mode = 'shared' | 'isolated';
export type OnDown = 'queue' | 'cordon' | 'circuit-break' | 'fail';

export interface CanvasSpec {
  flowRef: string;
  description: string;
  agentRef: string;
  shared: boolean;
  placement: { mode: Mode; runtimeSelector: Record<string, string>; namespace: string; tolerations: any[]; runtime: string | null };
  lifecycle: {
    state: 'running' | 'paused';
    session: 'ephemeral' | 'persistent';
    scaling: { min: number; max: number; metric: string | null; target: number | null };
    restart: { policy: string; backoff: string; maxRetries: number };
  };
  health: { dependsOn: { resource: string }[]; onDependencyDown: OnDown; circuit: any };
}

/// The complete spec — every key present, unset ones null/[] — from any partial one.
export function fullSpec(c: any = {}): CanvasSpec {
  const p = c.placement ?? {}, l = c.lifecycle ?? {}, h = c.health ?? {}, sc = l.scaling ?? {}, rs = l.restart ?? {};
  return {
    flowRef: c.flowRef ?? '',
    description: c.description ?? '',
    agentRef: c.agentRef ?? '',
    shared: c.shared ?? true,
    placement: { mode: p.mode ?? 'shared', runtimeSelector: p.runtimeSelector ?? {}, namespace: p.namespace ?? '', tolerations: Array.isArray(p.tolerations) ? p.tolerations : [], runtime: p.runtime || null },
    lifecycle: {
      state: l.state ?? 'running', session: l.session ?? 'ephemeral',
      scaling: { min: Number(sc.min ?? 1), max: Number(sc.max ?? 1), metric: sc.metric || null, target: sc.target === '' || sc.target == null ? null : Number(sc.target) },
      restart: { policy: rs.policy ?? 'on-failure', backoff: rs.backoff ?? 'exponential', maxRetries: Number(rs.maxRetries ?? 5) },
    },
    health: { dependsOn: Array.isArray(h.dependsOn) ? h.dependsOn.filter((d: any) => d?.resource) : [], onDependencyDown: h.onDependencyDown ?? 'queue', circuit: h.circuit ?? null },
  };
}

/// The spec as the YAML the Studio and the designer edit, with what each part is for.
export function specYaml(s: CanvasSpec): string {
  const doc = new YAML.Document(s);
  doc.commentBefore = ' a3t.io/v1alpha1 · CanvasSpec — one flow\'s deployment. Schema: GET /canvas/schema';
  const note = (k: string, t: string) => { const n: any = doc.get(k, true); if (n) n.commentBefore = ' ' + t; };
  note('placement', 'where it loads: a shared runtime pool or its own isolated sandbox');
  note('lifecycle', 'how it runs: state, session, scaling, restarts');
  note('health', 'what it depends on, and what to do when one is down');
  return String(doc);
}

export const selectorText = (sel?: Record<string, string>) => sel ? Object.entries(sel).map(([k, v]) => `${k}=${v}`).join(', ') : '';
export function textSelector(t: string): Record<string, string> {
  const out: Record<string, string> = {};
  for (const part of t.split(',').map((x) => x.trim()).filter(Boolean)) { const [k, v] = part.split('=').map((x) => x.trim()); if (k && v) out[k] = v; }
  return out;
}

// ── Placement: the server's select_runtime, with a reason for every runtime ──
export type Verdict = { rt: any; ok: boolean; why: string };
export function verdicts(s: CanvasSpec, runtimes: any[]): Verdict[] {
  const ns = s.placement.namespace.trim();
  return runtimes.map((r) => {
    if (ns && r.namespace !== ns) return { rt: r, ok: false, why: `namespace ${r.namespace}, needs ${ns}` };
    for (const [k, v] of Object.entries(s.placement.runtimeSelector)) {
      const have = (r.labels ?? {})[k];
      if (have !== v) return { rt: r, ok: false, why: have ? `${k}=${have}, needs ${k}=${v}` : `no ${k} label (needs ${k}=${v})` };
    }
    if (s.placement.runtime && r.name !== s.placement.runtime) return { rt: r, ok: false, why: `not the pinned runtime ${s.placement.runtime}` };
    return { rt: r, ok: true, why: '' };
  });
}
export function placedOn(s: CanvasSpec, runtimes: any[]): any | null {
  const ok = verdicts(s, runtimes).filter((v) => v.ok).map((v) => v.rt);
  if (s.placement.runtime) return ok.find((r) => r.name === s.placement.runtime) ?? null;
  if (!Object.keys(s.placement.runtimeSelector).length) return ok.find((r) => r.default) ?? ok[0] ?? null;
  return ok[0] ?? null;
}

/// What is wrong with it, as CanvasSpec::validate says, plus what the editor can see.
export function problems(s: CanvasSpec, o: { requireFlow: boolean; flowExists: boolean | null; runtimes: any[]; pinKnown: boolean }): string[] {
  const e: string[] = [];
  if (!s.flowRef && o.requireFlow) e.push('Pick the flow it deploys.');
  else if (s.flowRef && !s.flowRef.includes('/')) e.push("The flow is 'project/name'.");
  else if (s.flowRef && o.flowExists === false) e.push(`The flow ${s.flowRef} does not exist (any more): pick another.`);
  if (s.lifecycle.scaling.max < s.lifecycle.scaling.min) e.push('Scaling: max must be at least min.');
  if (s.placement.runtime && !o.pinKnown) e.push(`Pinned runtime ${s.placement.runtime} is not registered.`);
  if (o.runtimes.length && !placedOn(s, o.runtimes)) e.push('No runtime takes it: change the labels, the namespace or the pin.');
  for (const d of s.health.dependsOn) if (!d.resource.includes('/')) e.push(`Dependency ${d.resource} is not 'project/name'.`);
  return e;
}

// ── What is around it: whether its flow exists, what feeds it, what it calls, whether it is live ──
export interface Around { flowExists: boolean | null; triggers: any[]; calls: string[]; live: any | null }
const SERVING: Record<string, string> = { serving: 'Serving', paused: 'Paused', 'runtime-paused': 'Runtime paused', hibernated: 'Hibernated', unplaced: 'Unplaced' };
export const servingLabel = (s?: string) => (s ? SERVING[s] ?? s : 'Not deployed');

export async function around(flowRef: string, agentRef = ''): Promise<Around> {
  if (!flowRef.includes('/')) return { flowExists: null, triggers: [], calls: [], live: null };
  const [ns, name] = flowRef.split('/');
  const agent = agentRef ? agentRef.split('/').pop() : '';
  const [tr, fl, ops] = await Promise.all([mcpCall('list_triggers', {}), mcpCall('get_flow', { namespace: ns, name }), opsApi.overview()]);
  let d: any = tr.ok ? tr.data : [];
  if (typeof d === 'string') { try { d = JSON.parse(d); } catch { d = []; } }
  const all: any[] = Array.isArray(d) ? d : (d?.triggers ?? []);
  const triggers = all.filter((t) => (t.targets ?? []).some((g: any) => (g.type === 'workflow' && `${g.namespace}/${g.name}` === flowRef) || (agent && (g.type === 'agent' || g.type === 'a3t_agent') && g.agent_id === agent)));
  const text = typeof fl.data === 'string' ? fl.data : JSON.stringify(fl.data ?? '');
  const flowExists = fl.ok && !/not found|no such|does not exist/i.test(text);
  const calls = flowExists ? [...new Set([...text.matchAll(/call:\s*["']?([\w:.\-]+)/g)].map((m) => m[1]))] : [];
  let live: any = null;
  if (ops.ok) {
    const rows = [...(ops.data?.runtimes ?? []).flatMap((r: any) => r.deployments ?? []), ...(ops.data?.unplaced ?? [])];
    live = rows.find((row: any) => (agent ? row.deployment?.target === agent : row.deployment?.flowRef === flowRef)) ?? null;
  }
  return { flowExists, triggers, calls, live };
}

export function triggerLabel(t: any): string {
  const c = t.config ?? {};
  if (c.type === 'webhook') return `${c.method ?? 'POST'} /webhook${c.path ?? ''}`;
  if (c.type === 'cron') return `schedule ${c.expression ?? ''}`;
  if (c.type === 'adb_topic') return `topic ${c.topic ?? ''}`;
  return `${c.type ?? 'event'} ${c.source ?? ''}`;
}
