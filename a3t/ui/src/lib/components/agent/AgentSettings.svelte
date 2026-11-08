<script lang="ts">
  /// Settings — its name and description, where it came from, moving it to a
  /// newer release of its template, and deleting it.
  import { factsOf, lineageOf, forksOf, type AgentRec, type TemplateRec } from './agentData';
  import type { AgentLive } from './agentLive.svelte';
  import type { Bindings } from './agentValues';

  let { agent, templates, allAgents, live, bindings = $bindable(), dirty, saving, saveError, onsave, ondiscard, ongo, onopen, onupgrade, ondelete }: {
    agent: AgentRec & { created_by?: string; created_at?: string };
    templates: TemplateRec[];
    allAgents: AgentRec[];
    live: AgentLive;
    bindings: Bindings;
    dirty: boolean;
    saving: boolean;
    saveError: string;
    onsave: () => void;
    ondiscard: () => void;
    ongo: (tab: string) => void;
    onopen: (id: string) => void;
    onupgrade: (templateId: string, version: string) => Promise<string | null>;
    /// Delete it (and its endpoint and own canvas); an error, or null.
    ondelete: () => Promise<string | null>;
  } = $props();

  const f = $derived(factsOf(agent, templates));
  const lineage = $derived(lineageOf(agent, allAgents));
  const forks = $derived(forksOf(agent, allAgents));
  const cmp = (a: string, b: string) => { const x = a.split('.').map(Number), y = b.split('.').map(Number); for (let i = 0; i < 3; i++) if ((x[i] || 0) !== (y[i] || 0)) return (x[i] || 0) - (y[i] || 0); return 0; };
  const newer = $derived(f.template ? templates.filter((t) => t.name === f.template!.name && cmp(t.version, f.template!.version) > 0).sort((a, b) => cmp(b.version, a.version))[0] ?? null : null);
  let upgrading = $state(false);
  let upgradeMsg = $state('');
  async function upgrade() {
    if (!newer) return;
    upgrading = true;
    upgradeMsg = (await onupgrade(newer.id, newer.version)) ?? '';
    upgrading = false;
  }

  let confirmName = $state('');
  let deleting = $state(false);
  let deleteErr = $state('');
  const p = $derived(live.placement);
  async function del() {
    deleting = true;
    deleteErr = (await ondelete()) ?? '';
    deleting = false;
  }
</script>

<section class="agt">
  {#if dirty || saveError}
    <div class="dirty" class:bad={!!saveError}>
      <span>{saveError ? `Not saved: ${saveError}` : 'Unsaved changes'}</span>
      <span class="acts"><button class="btn2 quiet" disabled={saving} onclick={ondiscard}>Discard</button>
        <button class="btn2 primary" disabled={saving} onclick={onsave}>{saving ? 'Saving…' : 'Save'}</button></span>
    </div>
  {/if}

  <div class="card">
    <div class="card-head"><h3>About it</h3></div>
    <div class="form">
      <label>Name<input type="text" class="mono" bind:value={bindings.name} spellcheck="false" /></label>
      <label>Description <small>shown to people picking an agent, and in its API listing</small>
        <textarea rows="3" bind:value={bindings.description} placeholder={f.purpose || 'One or two sentences'}></textarea></label>
    </div>
  </div>

  <div class="card">
    <div class="card-head"><h3>Where it comes from</h3></div>
    <dl class="kv">
      <dt>Code</dt><dd class="mono">{f.template ? `${f.template.name}@${f.template.version}` : agent.template_id ?? '—'}</dd>
      <dt>Made as</dt>
      <dd>{#if lineage.kind === 'template'}new, from the template{:else}{lineage.kind === 'fork' ? 'a fork of' : 'a clone of'}
        {#if lineage.from?.exists}<button class="link" onclick={() => onopen(lineage.from!.id)}>{lineage.from.name}</button>{:else}{lineage.from?.name} <span class="dim">(deleted)</span>{/if}{/if}</dd>
      <dt>Id</dt><dd class="mono">{agent.id}</dd>
      <dt>Project</dt><dd class="mono">{agent.namespace}</dd>
      <dt>Owner</dt><dd>{agent.created_by || 'unowned'}</dd>
      {#if agent.created_at}<dt>Created</dt><dd>{new Date(agent.created_at).toLocaleString()}</dd>{/if}
    </dl>
    {#if newer}
      <div class="note row">
        <span>Its template has a newer release, <b class="mono">{newer.name}@{newer.version}</b>.</span>
        <button class="btn2" disabled={upgrading} onclick={upgrade}>{upgrading ? 'Moving…' : `Move to ${newer.version}`}</button>
        {#if upgradeMsg}<span class="err">{upgradeMsg}</span>{/if}
      </div>
    {/if}
    <p class="dim">To move it to another project, clone it there and delete this one. Who may change it is set by its project's access.</p>
  </div>

  <div class="card danger">
    <div class="card-head"><h3>Delete this agent</h3></div>
    {#if live.loading}
      <p class="dim">Checking where it runs…</p>
    {:else if p}
      <p>It is deployed{p.runtime ? ` on ${p.runtime}` : ''}. Undeploy it first, so no app calls an agent that is gone.</p>
      <div class="acts"><button class="btn2" onclick={() => ongo('releases')}>Go to Releases</button></div>
    {:else}
      <p>This removes, for good:</p>
      <ul class="goes">
        <li>the agent and its values</li>
        {#if f.endpoint}<li>its endpoint <span class="mono">POST /webhook{f.endpoint}</span> — apps calling it get an error</li>{/if}
        <li>its own deployment canvas, if it has one</li>
      </ul>
      <p class="dim">Its template is not touched.{forks.length ? ` Its ${forks.length} fork${forks.length === 1 ? '' : 's'} keep working on their own.` : ''}</p>
      <label class="confirm">Type <b class="mono">{agent.name}</b> to confirm<input type="text" bind:value={confirmName} autocomplete="off" spellcheck="false" /></label>
      <div class="acts">
        <button class="btn2 danger" disabled={confirmName !== agent.name || deleting} onclick={del}>{deleting ? 'Deleting…' : 'Delete agent'}</button>
        {#if deleteErr}<span class="err">{deleteErr}</span>{/if}
      </div>
    {/if}
  </div>
</section>

<style>
  .kv { grid-template-columns: 90px 1fr; }
  .danger { border-color: color-mix(in srgb, var(--err, #b42318) 35%, var(--border)); }
  .danger p { margin: 0; }
  .goes { margin: 0; padding-left: 18px; display: flex; flex-direction: column; gap: 3px; }
  .confirm { display: flex; flex-direction: column; gap: 5px; font-size: 12.5px; color: var(--text-2); text-transform: none; letter-spacing: 0; font-weight: 500; }
</style>
