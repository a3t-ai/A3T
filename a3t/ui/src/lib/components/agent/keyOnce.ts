// A key just made for an agent, held in memory until its owner says it is saved
// (or 15 minutes pass). Never in a URL, panel props or storage: a reload loses it.
const KEEP_MS = 15 * 60 * 1000;
const pending = new Map<string, { name: string; key: string; at: number }>();

export function keepKey(agentId: string, k: { name: string; key: string }) {
  pending.set(agentId, { ...k, at: Date.now() });
}

/// The key waiting to be copied for this agent, if any.
export function peekKey(agentId: string): { name: string; key: string } | null {
  const k = pending.get(agentId);
  if (!k) return null;
  if (Date.now() - k.at > KEEP_MS) { pending.delete(agentId); return null; }
  return { name: k.name, key: k.key };
}

/// Its owner has saved it: forget it.
export function dropKey(agentId: string) {
  pending.delete(agentId);
}
