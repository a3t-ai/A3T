<script lang="ts">
  /// Lineage: where the agent came from (template, then the agents it inherits from
  /// or was forked from), what came from it, and the forks that want to come back.
  /// A fork whose only change is procedural memory merges into its source (ADB
  /// sequences each write); a fork that changed more evolves into its own agent.
  import { control, type Ancestor, type Descendant, type Compare } from './controlPlane';
  import type { AgentRec } from './agentData';
  import AgentMakeFrom from './AgentMakeFrom.svelte';

  let { agent, allAgents, onopen, onmake, making = null, projects = [], onmakefrom, oncancelmake }: {
    agent: AgentRec;
    allAgents: AgentRec[];
    onopen: (id: string) => void;
    onmake: (kind: 'inherit' | 'fork' | 'branch') => void;
    /// A new agent being made from this one: its form opens here, inline.
    making?: 'clone' | 'fork' | 'inherit' | null;
    projects?: string[];
    onmakefrom?: (name: string, project: string) => Promise<string | null>;
    oncancelmake?: () => void;
  } = $props();

  let up = $state<Ancestor[]>([]);
  let down = $state<Descendant[]>([]);
  let checks = $state<Record<string, Compare | string>>({});
  let busy = $state('');
  let msg = $state('');
  let error = $state('');
  let loaded = $state(false);
  /// This agent against its source, when it is a fork: can it merge back?
  let self = $state<Compare | string | null>(null);
  const isFork = $derived(up[0]?.relation === 'fork');
  const canMerge = $derived(isFork && typeof self === 'object' && self !== null && self.mergeable);
  const canEvolve = $derived(isFork && typeof self === 'object' && self !== null && !self.mergeable);
  const why = $derived(!isFork ? 'Only a fork merges back or evolves.' : self === null ? 'Comparing it with its source…' : typeof self === 'string' ? self : '');

  const nameOf = (id: string) => allAgents.find((a) => a.id === id)?.name ?? id;
  const RELATION: Record<string, string> = { template: 'from template', inherit: 'inherits', clone: 'clone', fork: 'fork', branch: 'branch', evolved: 'evolved' };

  async function load(id: string) {
    loaded = false; error = ''; checks = {};
    const r = await control.lineage(id);
    if (!r.ok) { error = r.error ?? 'Lineage could not be read.'; loaded = true; return; }
    up = r.data?.ancestors ?? [];
    down = r.data?.descendants ?? [];
    loaded = true;
    self = null;
    if (up[0]?.relation === 'fork') {
      const c = await control.compare(id);
      if (agent.id === id) self = c.ok && c.data ? c.data : (c.error ?? 'compare failed');
    }
    for (const d of down.filter((x) => x.relation === 'fork')) {
      const c = await control.compare(d.id);
      checks = { ...checks, [d.id]: c.ok && c.data ? c.data : (c.error ?? 'compare failed') };
    }
  }
  $effect(() => { load(agent.id); });

  async function merge(id: string) {
    busy = id; msg = ''; error = '';
    const r = await control.merge(id);
    busy = '';
    if (!r.ok) { error = r.error ?? 'Merge failed.'; return; }
    msg = `${nameOf(id)}: ${r.data?.procedures ?? 0} procedures merged into ${agent.name}.`;
  }
  async function evolve(id: string) {
    busy = id; msg = ''; error = '';
    const r = await control.evolve(id);
    busy = '';
    if (!r.ok) { error = r.error ?? 'It did not evolve.'; return; }
    msg = `${nameOf(id)} is now its own agent, with ${agent.name} as its ancestor.`;
    await load(agent.id);
  }
  async function mergeSelf() {
    busy = agent.id; msg = ''; error = '';
    const r = await control.merge(agent.id);
    busy = '';
    if (!r.ok) { error = r.error ?? 'Merge failed.'; return; }
    msg = `${r.data?.procedures ?? 0} procedures merged into ${up[0] ? nameOf(up[0].id) : 'its source'}.`;
  }
  async function evolveSelf() {
    busy = agent.id; msg = ''; error = '';
    const r = await control.evolve(agent.id);
    busy = '';
    if (!r.ok) { error = r.error ?? 'It did not evolve.'; return; }
    msg = `${agent.name} is now its own agent.`;
    await load(agent.id);
  }
</script>

<section class="agt">
  <div class="ways">
    <div class="way inherit"><b>Inherit</b><span>Create a new agent from here.</span><em>new agent · child</em></div>
    <div class="way fork"><b>Fork</b><span>Also creates a new agent.</span><em>new agent · fork</em></div>
    <div class="way branch"><b>Branch</b><span>The same agent with a different version.</span><em>same agent</em></div>
    <div class="way merge"><b>Merge</b><span>If it can merge without conflicts.</span><em>back to its source</em></div>
    <div class="way evolve"><b>Evolve</b><span>It can't merge: it becomes a new agent.</span><em>new agent</em></div>
  </div>

  <div class="card">
    {#if !loaded}
      <p class="dim">Reading lineage…</p>
    {:else}
      <div class="fam">
        <div class="col">
          <h4>Ancestors</h4>
          {#each [...up].reverse() as a (a.kind + a.id)}
            <div class="node">
              {#if a.kind === 'agent'}<button class="link mono" onclick={() => onopen(a.id)}>{nameOf(a.id)}</button>{:else}<span class="mono">{a.id}{a.version ? `@${a.version}` : ''}</span>{/if}
              <span class="dim">{a.kind === 'template' ? 'template' : 'agent'}</span>
            </div>
          {:else}
            <p class="dim">None recorded. Agents made before lineage was kept show none.</p>
          {/each}
        </div>
        <div class="col">
          <h4>This agent</h4>
          <div class="node me">
            <span class="mono">{agent.namespace ?? 'default'}/{agent.name}</span>
            {#if up[0]}<span class="dim">{RELATION[up[0].relation] ?? up[0].relation} {up[0].kind === 'agent' ? nameOf(up[0].id) : up[0].id}</span>{/if}
          </div>
        </div>
        <div class="col">
          <h4>Descendants <span class="count">{down.length}</span></h4>
          {#each down as d (d.id)}
            <div class="node">
              <button class="link mono" onclick={() => onopen(d.id)}>{nameOf(d.id)}</button>
              <span class="tag" class:dev={d.relation === 'inherit'}>{d.relation}</span>
            </div>
          {:else}
            <p class="dim">Nothing made from it yet.</p>
          {/each}
        </div>
      </div>
    {/if}
    {#if error}<p class="err">{error}</p>{/if}
  </div>

  {#if down.some((d) => d.relation === 'fork')}
    <div class="card">
      <div class="card-head"><h3>Forks that can come back</h3><span>the one move that fits each</span></div>
      <ul class="forks">
        {#each down.filter((d) => d.relation === 'fork') as d (d.id)}
          {@const c = checks[d.id]}
          <li>
            <span class="mono">{nameOf(d.id)}</span>
            {#if c === undefined}
              <span class="dim grow">comparing…</span>
            {:else if typeof c === 'string'}
              <span class="err grow">{c}</span>
            {:else if c.mergeable}
              <span class="grow">Only procedures changed.</span>
              <button class="btn2 primary" disabled={busy === d.id} onclick={() => merge(d.id)}>{busy === d.id ? 'Merging…' : `Merge into ${agent.name}`}</button>
            {:else}
              <span class="grow">Changed: <span class="mono">{c.changed.join(', ')}</span></span>
              <button class="btn2" disabled={busy === d.id} onclick={() => evolve(d.id)}>{busy === d.id ? 'Evolving…' : 'Evolve into its own agent'}</button>
            {/if}
          </li>
        {/each}
      </ul>
    </div>
  {/if}

  <div class="row between">
    <span>{#if msg}<span class="ok">{msg}</span>{/if}</span>
    <div class="acts">
      <button class="btn2" class:on={making === 'inherit'} onclick={() => onmake('inherit')}>Inherit</button>
      <button class="btn2" class:on={making === 'fork'} onclick={() => onmake('fork')}>Fork</button>
      <button class="btn2" onclick={() => onmake('branch')}>Branch</button>
      <button class="btn2" disabled={!canMerge || busy === agent.id} title={canMerge ? `Merge into ${up[0] ? nameOf(up[0].id) : 'its source'}` : why || 'It changed more than procedures: evolve it instead.'} onclick={() => mergeSelf()}>{busy === agent.id && canMerge ? 'Merging…' : 'Merge'}</button>
      <button class="btn2" disabled={!canEvolve || busy === agent.id} title={canEvolve ? 'Make it its own agent' : why || 'It can merge: merge it instead.'} onclick={() => evolveSelf()}>{busy === agent.id && canEvolve ? 'Evolving…' : 'Evolve'}</button>
    </div>
  </div>
  {#if making && onmakefrom}
    {#key making}
      <AgentMakeFrom kind={making} name={agent.name} {projects} current={agent.namespace} onmake={onmakefrom} oncancel={() => oncancelmake?.()} />
    {/key}
  {/if}
</section>

<style>
  .btn2.on { border-color: var(--spec); color: var(--spec); }
  .fam { display: grid; grid-template-columns: repeat(3, minmax(0, 1fr)); gap: 14px; align-items: start; }
  .col { display: flex; flex-direction: column; gap: 8px; }
  .node { display: flex; flex-direction: column; gap: 3px; padding: 9px 11px; border: 1px solid var(--border); border-radius: 8px; background: var(--bg-ivory); }
  .node .tag { align-self: flex-start; }
  .node.me { border: 2px solid var(--spec); background: var(--signal-bg); }
  .ways { display: grid; grid-template-columns: repeat(5, minmax(0, 1fr)); gap: 10px; }
  @container details (max-width: 900px) { .ways { grid-template-columns: repeat(2, minmax(0, 1fr)); } }
  .way { display: flex; flex-direction: column; gap: 6px; padding: 11px 12px; border: 1px solid var(--border); border-top-width: 3px; border-radius: 10px; background: var(--bg-surface); font-size: 12px; }
  .way b { font: 400 15px var(--font-serif); color: var(--text-1); }
  .way em { font: normal 11.5px var(--font-mono); color: var(--text-1); }
  .way.inherit { border-top-color: var(--spec); } .way.fork { border-top-color: var(--text-3); }
  .way.branch { border-top-color: var(--signal); } .way.merge { border-top-color: var(--runtime); } .way.evolve { border-top-color: var(--checkpoint); }
  .forks { list-style: none; margin: 0; padding: 0; }
  .forks li { display: flex; align-items: center; gap: 12px; padding: 9px 0; border-top: 1px solid var(--divider); }
</style>
