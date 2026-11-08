// SAMPLE — an agent's branches and releases, kept in the browser only.
//
// Agents are meant to work like repos: prod is what callers get; a dev branch
// is the same agent with changes being tried; merging a branch makes a new
// release of prod. None of that is stored by the server yet (SRV-12, the
// agents-as-repos spec), so the Branches tab runs on this sample: what it
// shows is how the screen will work, and nothing here is saved.

export type Branch = {
  name: string;
  /** What was changed on it, in words. */
  changes: string[];
  created: number;
  by: string;
};
export type Release = { tag: string; at: number; note: string };

type State = { branches: Branch[]; releases: Release[] };

const DAY = 86_400_000;
let store = $state<Record<string, State>>({});

function seed(_agentId: string, created?: string): State {
  const t0 = Date.parse(created ?? '') || Date.now() - 20 * DAY;
  return {
    branches: [],
    releases: [
      { tag: 'v2', at: Math.max(t0, Date.now() - 3 * DAY), note: 'Prompt tightened; pages on-call only for high priority' },
      { tag: 'v1', at: t0, note: 'First release, from its template' },
    ],
  };
}

/// An agent's branches and releases. Reading never writes (it is used in
/// derived values); the seed is stored on the first change.
export function branchesOf(agentId: string, created?: string): State {
  return store[agentId] ?? seed(agentId, created);
}
function edit(agentId: string): State {
  return (store[agentId] ??= seed(agentId));
}

export function addBranch(agentId: string, name: string, by: string, intent: string) {
  edit(agentId).branches.unshift({ name, changes: intent ? [intent] : [], created: Date.now(), by });
}

export function dropBranch(agentId: string, name: string) {
  const s = edit(agentId);
  s.branches = s.branches.filter((b) => b.name !== name);
}

/// Merging makes the next release of prod and closes the branch.
export function mergeBranch(agentId: string, name: string): string {
  const s = edit(agentId);
  const b = s.branches.find((x) => x.name === name);
  const n = s.releases.reduce((m, r) => Math.max(m, Number(r.tag.slice(1)) || 0), 0) + 1;
  const tag = `v${n}`;
  s.releases.unshift({ tag, at: Date.now(), note: b?.changes.join('; ') || `Merged ${name}` });
  s.branches = s.branches.filter((x) => x.name !== name);
  return tag;
}

/// Rolling back makes a new release with an older one's content.
export function rollBack(agentId: string, to: string): string {
  const s = edit(agentId);
  const n = s.releases.reduce((m, r) => Math.max(m, Number(r.tag.slice(1)) || 0), 0) + 1;
  const tag = `v${n}`;
  s.releases.unshift({ tag, at: Date.now(), note: `Rolled back to ${to}` });
  return tag;
}
