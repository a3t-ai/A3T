// Trigger mappings written by an agent, at dev time.
//
// agent_a3t_mapper (an A3T agent) proposes the jq that turns a trigger's event
// into a target's input; FlowR checks it on sample events with the trigger's own
// engine (and the flow's input schema); a person approves it, and only then is it
// written into the trigger. When the trigger fires, only that jq runs — no agent.
//
// Every proposal, approval and rejection is kept, append-only, as procedural
// memory of agent_a3t_mapper: a mapping's state is its newest record.
import { API_BASE } from '$lib/config';
import { getAccessToken } from '$lib/services/api';
import { getSession } from '$lib/stores/auth';
import { insight } from '$lib/components/agent/controlPlane';

export const MAPPER = 'agent_a3t_mapper';
export type Field = 'input' | 'message' | 'conversation';

export interface Proposal { jq: string; explanation: string; assumptions: string[]; drops: string[] }
export interface CheckResult {
  all_ok: boolean;
  results: { ok: boolean; value?: unknown; error?: string; schema_errors?: string[] }[];
  schema: unknown | null;
  schema_note: string | null;
}
export interface MappingRecord {
  key: string; at: string;
  event: 'proposed' | 'approved' | 'rejected';
  trigger_id: string; target: string; field: Field;
  jq: string; version: number; by: string;
  source: 'agent' | 'hand';
  explanation?: string; assumptions?: string[]; drops?: string[];
  samples?: number; reason?: string; wants?: string;
}

const auth = () => ({ 'content-type': 'application/json', authorization: `Bearer ${getAccessToken() ?? ''}` });

/// FlowR's form for an expression: `${ … }`.
export function wrapJq(s: string): string {
  const t = s.trim();
  return /^\$\{[\s\S]*\}$/.test(t) ? t : `\${ ${t} }`;
}

/// Ask a design-time A3T agent (on its data plane, as the signed-in user) and
/// read the JSON object it answers with.
export async function askAgentJson(agent: string, ask: Record<string, unknown>, signal?: AbortSignal): Promise<{ ok: true; json: any } | { ok: false; error: string }> {
  try {
    const res = await fetch(`${API_BASE}/chat`, {
      method: 'POST', signal,
      headers: { ...auth(), accept: 'text/event-stream' },
      body: JSON.stringify({ session_id: `${agent}:ask-${Date.now().toString(36)}`, message: JSON.stringify(ask, null, 1) }),
    });
    if (!res.ok) return { ok: false, error: `${agent} answered ${res.status}` };
    const raw = await res.text();
    let text = '', err = '';
    for (const line of raw.split('\n')) {
      if (!line.startsWith('data:')) continue;
      try { const e = JSON.parse(line.slice(5)); if (e.type === 'text-delta') text += e.delta; if (e.type === 'error') err = e.errorText ?? e.message ?? 'error'; } catch { /* keep-alive */ }
    }
    if (!text && err) return { ok: false, error: err };
    // Its answer is JSON (an object, or a list), sometimes in a code fence.
    const starts = [text.indexOf('{'), text.indexOf('[')].filter((i) => i >= 0);
    const start = starts.length ? Math.min(...starts) : 0;
    const end = Math.max(text.lastIndexOf('}'), text.lastIndexOf(']')) + 1;
    return { ok: true, json: JSON.parse(text.slice(start, end)) };
  } catch (e) {
    return { ok: false, error: (e as Error).name === 'AbortError' ? 'Stopped.' : `Could not read its answer: ${(e as Error).message}` };
  }
}

/// Ask agent_a3t_mapper for a mapping.
export async function askMapper(ask: Record<string, unknown>, signal?: AbortSignal): Promise<{ ok: true; proposal: Proposal } | { ok: false; error: string }> {
  const r = await askAgentJson(MAPPER, ask, signal);
  if (!r.ok) return r;
  const p = r.json;
  if (typeof p.jq !== 'string' || !p.jq.trim()) return { ok: false, error: 'It answered without a jq expression.' };
  return { ok: true, proposal: { jq: wrapJq(p.jq), explanation: String(p.explanation ?? ''), assumptions: (p.assumptions ?? []).map(String), drops: (p.drops ?? []).map(String) } };
}

/// Run a mapping on sample bodies as the trigger would (FlowR, nothing runs).
export async function checkMapping(a: { expr: string; samples: unknown[]; field: Field; target?: string; trigger: string; source: string }): Promise<{ ok: true; check: CheckResult } | { ok: false; error: string }> {
  try {
    const res = await fetch(`${API_BASE}/triggers/mapping/check`, { method: 'POST', headers: auth(), body: JSON.stringify(a) });
    const j = await res.json().catch(() => ({}));
    return res.ok ? { ok: true, check: j as CheckResult } : { ok: false, error: j.error ?? `HTTP ${res.status}` };
  } catch (e) { return { ok: false, error: String(e) }; }
}

const asList = (v: unknown): string[] => Array.isArray(v) ? v.map(String) : typeof v === 'string' && v.startsWith('[') ? (() => { try { return JSON.parse(v); } catch { return []; } })() : [];

/// The mapping records of one trigger, newest first.
export async function listRecords(triggerId: string): Promise<{ ok: true; records: MappingRecord[] } | { ok: false; error: string }> {
  const r = await insight.memory(MAPPER);
  if (!r.ok) return { ok: false, error: r.error ?? 'Its memory could not be read.' };
  const rows: any[] = r.data?.memory?.procedural ?? [];
  const records = rows
    .map((e) => ({ key: e.key, at: e.at ?? e.data?.at, ...(e.data ?? e) }))
    .filter((d) => d.kind === 'trigger_mapping' && d.trigger_id === triggerId)
    .map((d) => ({ ...d, version: Number(d.version ?? 0), samples: Number(d.samples ?? 0), assumptions: asList(d.assumptions), drops: asList(d.drops) }) as MappingRecord)
    .sort((a, b) => String(b.at).localeCompare(String(a.at)));
  return { ok: true, records };
}

/// Keep a record (append-only).
export async function putRecord(rec: Omit<MappingRecord, 'key' | 'at' | 'by'>): Promise<{ ok: boolean; error?: string }> {
  const by = getSession()?.user?.email || getSession()?.user?.id || 'unknown';
  const data: Record<string, unknown> = { kind: 'trigger_mapping', ...rec, by, at: new Date().toISOString() };
  for (const k of Object.keys(data)) if (data[k] === undefined || data[k] === '') delete data[k];
  const r = await insight.memoryPut(MAPPER, 'procedural', data);
  return r.ok ? { ok: true } : { ok: false, error: r.error };
}

/// Where a field's mapping stands, from its records and the text in the form.
export type Standing =
  | { kind: 'approved'; rec: MappingRecord }
  | { kind: 'changed'; rec: MappingRecord }
  | { kind: 'proposed'; rec: MappingRecord }
  | { kind: 'hand' }
  | { kind: 'none' };
export function standing(records: MappingRecord[], target: string, field: Field, current: string): Standing {
  const mine = records.filter((r) => r.target === target && r.field === field);
  const approved = mine.find((r) => r.event === 'approved');
  const norm = (s: string) => s.replace(/\s+/g, ' ').trim();
  if (approved && norm(approved.jq) === norm(current || '')) return { kind: 'approved', rec: approved };
  if (approved && current?.trim()) return { kind: 'changed', rec: approved };
  const open = mine[0]?.event === 'proposed' ? mine[0] : undefined;
  if (open) return { kind: 'proposed', rec: open };
  return current?.trim() ? { kind: 'hand' } : { kind: 'none' };
}

export const nextVersion = (records: MappingRecord[], target: string, field: Field) =>
  1 + Math.max(0, ...records.filter((r) => r.target === target && r.field === field && r.event === 'approved').map((r) => r.version || 0));
