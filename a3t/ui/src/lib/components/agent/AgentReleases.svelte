<script lang="ts">
  /// Releases — where the agent runs and what version: prod (its real
  /// deployment, one click to pause or undeploy), dev branches, merges,
  /// releases, the values each environment gives what its code uses, and its
  /// forks. Branches, releases and environment values are Sample (SRV-12, SRV-5/6).
  import { untrack, type Snippet } from 'svelte';
  import { opsApi } from '$lib/services/api';
  import { navigateTo } from '$lib/stores/panels.svelte.ts';
  import { factsOf, forksOf, ago, type AgentRec, type TemplateRec } from './agentData';
  import { branchesOf, addBranch, dropBranch, mergeBranch, rollBack } from './agentBranches.svelte';
  import { usesOf } from './agentValues';
  import type { AgentLive } from './agentLive.svelte';

  let { agent, templates, allAgents, live, me, onstudio, onopen, onchanged, notDeployed, branchForm = $bindable(false) }: {
    agent: AgentRec & { created_at?: string };
    templates: TemplateRec[];
    allAgents: AgentRec[];
    live: AgentLive;
    me: string;
    onstudio: () => void;
    onopen: (id: string) => void;
    /// The deployment changed: reload what is shown.
    onchanged: () => void;
    /// How to deploy it when it is not deployed (the page's profile picker).
    notDeployed: Snippet;
    branchForm?: boolean;
  } = $props();

  const f = $derived(factsOf(agent, templates));
  const forks = $derived(forksOf(agent, allAgents));
  const repo = $derived(branchesOf(agent.id, agent.created_at));
  const prodTag = $derived(repo.releases[0]?.tag ?? 'v1');
  const p = $derived(live.placement);
  const uses = $derived(usesOf(f.spec));

  // ── prod: one click each; details in Runtimes ──
  let busy = $state('');
  let opMsg = $state('');
  let confirmUndeploy = $state(false);
  async function op(action: 'pause' | 'resume' | 'undeploy') {
    if (!p?.id) return;
    busy = action; opMsg = '';
    const r = await opsApi.deployment(p.id, action, action === 'pause' ? `Paused from the agent ${agent.name}` : undefined);
    busy = ''; confirmUndeploy = false;
    if (!r.ok) { opMsg = r.error ?? `Could not ${action} it`; return; }
    onchanged();
  }

  // ── dev branches (sample) ──
  let brName = $state('');
  let brIntent = $state('');
  let merging = $state('');
  let dropping = $state('');
  let mergedMsg = $state('');
  $effect(() => { if (branchForm) untrack(() => { brName = `dev/${new Date().toISOString().slice(5, 10).replace('-', '')}`; brIntent = ''; }); });
  function makeBranch() {
    const n = brName.trim().replace(/\s+/g, '-');
    if (!n) return;
    addBranch(agent.id, n.startsWith('dev/') ? n : `dev/${n}`, me || 'you', brIntent.trim());
    branchForm = false;
  }
  /// A dev value for a name the code uses: the prod one with a dev suffix — how
  /// it will read once projects keep values per environment.
  const devOf = (v: string) => v.includes('/') ? v.replace(/^([^/]+)\//, '$1-dev/') : `${v}-dev`;
</script>

<section class="agt">
  <div class="card">
    <div class="card-head"><h3>Environments</h3><span class="grow"></span>
      {#if !branchForm}<button class="btn2" onclick={() => (branchForm = true)}>Branch to dev</button>{/if}</div>
    <table class="tbl env">
      <thead><tr><th></th><th><span class="tag prod">prod</span></th><th><span class="tag dev">dev</span> <span class="tag sample">sample</span></th></tr></thead>
      <tbody>
        <tr><td class="k">Runs</td><td><b class="mono">{prodTag}</b> <span class="tag sample">sample</span></td>
          <td>{#if repo.branches[0]}<b class="mono">{repo.branches[0].name}</b>{:else}<span class="dim">nothing — branch to dev</span>{/if}</td></tr>
        <tr><td class="k">State</td>
          <td>{#if live.loading}…{:else if p}<span class="row"><span class="dot {p.deploymentState === 'paused' ? 'warn' : 'ok'}"></span>{p.deploymentState === 'paused' ? 'paused' : 'deployed'} on <span class="mono">{p.runtime || '—'}</span>{p.canvas ? ` · ${p.canvas}` : ''}</span>
            {:else}<span class="row"><span class="dot"></span>not deployed — it answers on the shared runtime</span>{/if}</td>
          <td>{repo.branches[0] ? 'deployed with the dev canvas' : '—'}</td></tr>
        <tr><td class="k">Endpoint</td><td class="mono">{f.endpoint ? `/webhook${f.endpoint}` : '—'}</td>
          <td class="mono">{repo.branches[0] && f.endpoint ? `/webhook${f.endpoint.replace(/\/chat$/, '')}/dev/chat` : '—'}</td></tr>
        <tr><td class="k"></td>
          <td>
            {#if !live.loading && p}
              <div class="acts">
                {#if p.deploymentState === 'paused'}<button class="btn2 primary" disabled={!!busy} onclick={() => op('resume')}>{busy === 'resume' ? 'Resuming…' : 'Resume'}</button>
                {:else}<button class="btn2" disabled={!!busy} onclick={() => op('pause')}>{busy === 'pause' ? 'Pausing…' : 'Pause'}</button>{/if}
                <button class="btn2" disabled={!!busy} onclick={() => (confirmUndeploy = true)}>Undeploy</button>
                <button class="link" onclick={() => navigateTo('runtime', 'Runtimes')}>Runtimes →</button>
              </div>
              {#if confirmUndeploy}
                <div class="note">Undeploy it? Its runtime stops; it goes back to the shared runtime.
                  <span class="acts"><button class="btn2 danger" disabled={!!busy} onclick={() => op('undeploy')}>{busy === 'undeploy' ? 'Undeploying…' : 'Undeploy'}</button>
                  <button class="link" onclick={() => (confirmUndeploy = false)}>Cancel</button></span></div>
              {/if}
              {#if opMsg}<p class="err">{opMsg}</p>{/if}
            {:else if !live.loading}
              {@render notDeployed()}
            {/if}
          </td>
          <td>{#if repo.branches[0]}<button class="btn2" onclick={onstudio}>Test in Studio</button>{/if}</td>
        </tr>
      </tbody>
    </table>
    {#if live.placeErr}<p class="dim">{live.placeErr}</p>{/if}
  </div>

  {#if branchForm}
    <div class="card">
      <div class="card-head"><h3>Branch to dev</h3><span>this agent in dev — change and test it there; prod is not touched until you merge</span></div>
      <div class="form">
        <label>Branch<input type="text" class="mono" bind:value={brName} /></label>
        <label>What will you change? <small>(optional)</small><input type="text" bind:value={brIntent} placeholder="e.g. a smaller model for low-priority alerts" /></label>
        <div class="acts"><button class="btn2 primary" disabled={!brName.trim()} onclick={makeBranch}>Make the branch</button><button class="btn2 quiet" onclick={() => (branchForm = false)}>Cancel</button></div>
      </div>
    </div>
  {/if}

  <div class="sample"><b>Sample</b> Branches, merges, releases and per-environment values are not stored yet — this shows how they will work; nothing here is saved. Prod's deployment and the forks are real.</div>

  <h4>Dev branches <span class="count">{repo.branches.length}</span></h4>
  {#if mergedMsg}<p class="ok">{mergedMsg}</p>{/if}
  <ul class="list">
    {#each repo.branches as b (b.name)}
      <li class="card">
        <div class="row between">
          <div class="row"><span class="tag dev">dev</span><b class="mono">{b.name}</b><span class="dim">{b.by}, {ago(b.created)}</span></div>
          <div class="acts">
            <button class="btn2" onclick={onstudio}>Test in Studio</button>
            <button class="btn2 primary" onclick={() => { merging = b.name; dropping = ''; }}>Merge to prod</button>
            <button class="btn2 quiet" onclick={() => { dropping = b.name; merging = ''; }}>Drop</button>
          </div>
        </div>
        <ul class="changes">{#each b.changes as c}<li>{c}</li>{:else}<li class="dim">No changes yet — open it in the Studio.</li>{/each}</ul>
        {#if merging === b.name}
          <div class="note">Merging makes <b>v{(Number(prodTag.slice(1)) || 0) + 1}</b> of prod; callers get it on their next message.
            Its values and code replace prod's; what it learned on the branch (procedures) is added to prod's memory.
            <span class="acts"><button class="btn2 primary" onclick={() => { const t = mergeBranch(agent.id, b.name); merging = ''; mergedMsg = `Merged ${b.name} — prod is now ${t}.`; }}>Merge</button>
            <button class="link" onclick={() => (merging = '')}>Cancel</button></span></div>
        {:else if dropping === b.name}
          <div class="note">Drop <b class="mono">{b.name}</b>? Its changes and what it learned are thrown away; prod is not touched.
            <span class="acts"><button class="btn2 danger" onclick={() => { dropBranch(agent.id, b.name); dropping = ''; }}>Drop it</button>
            <button class="link" onclick={() => (dropping = '')}>Cancel</button></span></div>
        {/if}
      </li>
    {:else}
      <li class="empty">No dev branches.</li>
    {/each}
  </ul>

  <h4>Releases</h4>
  <table class="tbl">
    <tbody>
      {#each repo.releases as r, i (r.tag)}
        <tr><td class="mono k">{r.tag}</td><td>{#if i === 0}<span class="tag prod">prod</span>&nbsp; {/if}{r.note}</td><td class="when">{ago(r.at)}</td>
          <td class="when">{#if i > 0}<button class="link" onclick={() => { const t = rollBack(agent.id, r.tag); mergedMsg = `Prod is now ${t}, the same as ${r.tag}.`; }}>Roll back to {r.tag}</button>{/if}</td></tr>
      {/each}
    </tbody>
  </table>

  <h4>Values per environment</h4>
  {#if uses.length}
    <table class="tbl">
      <thead><tr><th>Its code uses</th><th>Kind</th><th>prod</th><th>dev</th></tr></thead>
      <tbody>
        {#each uses as u (u.name)}
          <tr><td class="mono">{u.name}</td><td class="k">{u.kind}</td><td class="mono">{u.name}</td><td class="mono">{devOf(u.name)}</td></tr>
        {/each}
      </tbody>
    </table>
    <p class="dim">The code names them; each environment of the project says which one it means (Projects → Environments). Today every environment uses the names as written.</p>
  {:else}
    <p class="dim">Its code names no resources or flows directly.</p>
  {/if}

  <h4>Forks <span class="count">{forks.length}</span></h4>
  <ul class="list">
    {#each forks as k (k.id)}
      <li><button class="pick" onclick={() => onopen(k.id)}><b>{k.name}</b><span class="mono dim">{k.namespace}</span></button></li>
    {:else}
      <li class="empty">No forks.</li>
    {/each}
  </ul>
</section>

<style>
  .env td:first-child { width: 90px; }
  .env td { width: 45%; }
  .env .note { margin-top: 8px; display: flex; flex-wrap: wrap; gap: 8px; align-items: center; }
  .list { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 8px; }
  .list .card { gap: 8px; padding: 12px 14px; }
  .list .note { display: flex; flex-wrap: wrap; gap: 8px; align-items: center; }
  .changes { margin: 0; padding-left: 18px; font-size: 12.5px; }
  .changes .dim { list-style: none; margin-left: -18px; }
  .empty { border: 1px dashed var(--border); border-radius: 8px; }
  .when { white-space: nowrap; color: var(--text-4); width: 1%; }
  .pick { width: 100%; display: flex; gap: 10px; align-items: baseline; padding: 9px 12px; border: 1px solid var(--border); border-radius: 8px; background: var(--bg-surface); cursor: pointer; font: inherit; font-size: 13px; text-align: left; }
  .pick:hover { border-color: var(--spec); }
  .pick b { flex: 1; font-weight: 500; }
</style>
