// Reading ADB memory with AQL, from the console: Memory (an agent's space) and
// Knowledge (the semantic space a graph is built from). Read only — a console
// that could STORE, UPDATE or FORGET would bypass the agent's own memory tools
// and their checks, so only RECALL is sent.
import { execFlow } from '$lib/services/api';

export const MEMORY_RESOURCE = 'a3t/adb';
export type Kind = 'WORKING' | 'EPISODIC' | 'SEMANTIC' | 'PROCEDURAL';
export const KINDS: { k: Kind; label: string; what: string }[] = [
  { k: 'WORKING', label: 'Working', what: 'what it holds for the task at hand' },
  { k: 'EPISODIC', label: 'Episodic', what: 'what happened, turn by turn' },
  { k: 'SEMANTIC', label: 'Semantic', what: 'facts it knows' },
  { k: 'PROCEDURAL', label: 'Procedural', what: 'how it does things' },
];

export interface Rec { id: string; key: string; at: string; data: Record<string, unknown> }

const q = (s: string) => `"${s.replace(/\\/g, '\\\\').replace(/"/g, '\\"')}"`;

/// RECALL FROM <kind> [WHERE f = "v" [AND …]] NAMESPACE "<space>" LIMIT n
export function recall(kind: Kind, space: string, where: { field: string; value: string }[] = [], limit = 100): string {
  const w = where.filter((c) => c.field.trim() && c.value !== '');
  const cond = w.length ? `WHERE ${w.map((c) => `${c.field.trim()} = ${/^-?\d+(\.\d+)?$/.test(c.value) ? c.value : q(c.value)}`).join(' AND ')}` : '';
  return `RECALL FROM ${kind} ${cond ? cond + ' ' : ''}NAMESPACE ${q(space)} LIMIT ${limit}`;
}

export const isRead = (aql: string) => /^\s*RECALL\b/i.test(aql) && !/\b(STORE|UPDATE|FORGET|DELETE|DROP)\b/i.test(aql.replace(/"(?:[^"\\]|\\.)*"/g, '""'));

export async function runAql(aql: string, resource = MEMORY_RESOURCE): Promise<{ ok: true; records: Rec[]; ms: number | null } | { ok: false; error: string }> {
  if (!isRead(aql)) return { ok: false, error: 'Only RECALL runs here: change memory through the agent (its Memory tab), so its checks apply.' };
  const r = await execFlow('flow_a3t_resource_test', 'a3t', { input: { test_type: 'adb', resource, query: aql.trim() } }, { silent: true });
  if (!r.ok) return { ok: false, error: r.error ?? 'It did not run.' };
  const d: any = r.data ?? {};
  if (d.success === false) return { ok: false, error: d.error?.message ?? d.error ?? 'It did not run.' };
  const rows: any[] = d.data?.records ?? [];
  return {
    ok: true, ms: d.metadata?.execution_time_ms ?? null,
    records: rows.map((x) => ({
      id: String(x.id ?? ''), key: String(x.data?.key ?? x.id ?? '').split('|~').pop() ?? '',
      at: String(x.data?.at ?? x.data?.created_at ?? x.metadata?.created_at ?? x.created_at ?? ''), data: (x.data ?? {}) as Record<string, unknown>,
    })),
  };
}
