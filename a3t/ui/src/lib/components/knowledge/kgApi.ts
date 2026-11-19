// A knowledge graph database: a LanceDB resource the call:rotate handler keeps
// entities, relations, triples and their RotatE embeddings in. Every call names
// the database (`kg`), so the Knowledge app can hold several. A database is a KG
// when its config says `kind: kg`, or its name does (a3t-kg-db).
import { execFlow } from '$lib/services/api';

export const isKg = (r: { type?: string; name: string; config?: any }) =>
  (r.type ?? '').toLowerCase() === 'lancedb' && (r.config?.kind === 'kg' || /(^|[-_])kg([-_]|$)/i.test(r.name));

async function kg<T = any>(ref: string, action: string, args: Record<string, unknown> = {}): Promise<{ ok: true; data: T } | { ok: false; error: string }> {
  const r = await execFlow('flow_a3t_knowledge', 'a3t', { input: { action, kg: ref, ...args } }, { silent: true });
  return r.ok ? { ok: true, data: r.data as T } : { ok: false, error: r.error ?? `${action} failed` };
}

export const kgApi = {
  stats: (ref: string, sample = 5000, max_edges = 8000) => kg(ref, 'stats', { sample, max_edges }),
  relations: (ref: string) => kg<{ relations: { id: string; name: string; triples: number }[] }>(ref, 'relations'),
  lookup: (ref: string, f: { head?: string; relation?: string; tail?: string; limit?: number }) => kg<{ triples: any[] }>(ref, 'lookup', f),
  predictTail: (ref: string, head: string, relation: string, limit = 10) => kg<any>(ref, 'predict_tail', { head, relation, limit }),
  neighbors: (ref: string, entity: string, limit = 10) => kg<any>(ref, 'neighbors', { entity, limit }),
  ingest: (ref: string, triples: { head: string; relation: string; tail: string }[]) => kg<any>(ref, 'ingest', { triples }),
  train: (ref: string, epochs = 200) => kg<any>(ref, 'train', { epochs }),
  fromSemantic: (ref: string, namespace: string, epochs = 300) => kg<any>(ref, 'from_semantic', { namespace, epochs, train: true }),
};
