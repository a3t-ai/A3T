<script lang="ts">
  /// Releases — where it runs and which version. Its environment (dev, test,
  /// prod) is its owner's setting, not a platform mode. Its deployment is real
  /// (deploy / undeploy; pausing is in Controls); branches, the review before a
  /// merge, and releases are Sample (SRV-12, SRV-20). Forks are real.
  import { untrack, type Snippet } from 'svelte';
  import { opsApi } from '$lib/services/api';
  import { navigateTo } from '$lib/stores/panels.svelte.ts';
  import { factsOf, forksOf, envOf, ago, type AgentRec, type TemplateRec } from './agentData';
  import { branchesOf, addBranch, dropBranch, mergeBranch, rollBack } from './agentBranches.svelte';
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
  const env = $derived(envOf(agent));

  // ── prod: one click each; details in Runtimes ──
  let busy = $state('');
  let opMsg = $state('');
  let confirmUndeploy = $state(false);
  async function op(action: 'undeploy') {
    if (!p?.id) return;
    busy = action; opMsg = '';
    const r = await opsApi.deployment(p.id, action);
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
  let approver = $state('');
</script>

<section class="agt">
  <div class="card">
    <div class="card-head"><h3>Deployment</h3><span class="grow"></span>
      {#if !branchForm}<button class="btn2" onclick={() => (branchForm = true)}>Branch</button>{/if}</div>
    <dl class="kv">
      <dt>Environment</dt><dd>{#if env}<span class="tag" class:prod={env === 'prod'} class:dev={env !== 'prod'}>{env}</span>{:else}<span class="dim">not set</span>{/if} <span class="dim">— its owner's setting, in Settings</span></dd>
      <dt>Release</dt><dd><b class="mono">{prodTag}</b> <span class="tag sample">sample</span></dd>
      <dt>Runs on</dt><dd>{#if live.loading}…{:else if p}<span class="row"><span class="dot {p.deploymentState === 'paused' ? 'warn' : 'ok'}"></span>{p.deploymentState === 'paused' ? 'paused' : 'deployed'} on <span class="mono">{p.runtime || '—'}</span>{p.canvas ? ` · ${p.canvas}` : ''}</span>
        {:else}<span class="row"><span class="dot"></span>not deployed — the shared runtime</span>{/if}</dd>
      <dt>Endpoint</dt><dd class="mono">{f.endpoint ? `/webhook${f.endpoint}` : '—'}</dd>
    </dl>
    {#if !live.loading && p}
      <div class="acts">
        <button class="btn2" disabled={!!busy} onclick={() => (confirmUndeploy = true)}>Undeploy</button>
        <button class="link" onclick={() => navigateTo('runtime', 'Runtimes')}>Runtimes →</button>
        <span class="dim">Pause is in Controls.</span>
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
    {#if live.placeErr}<p class="dim">{live.placeErr}</p>{/if}
  </div>

  {#if branchForm}
    <div class="card">
      <div class="card-head"><h3>Branch</h3><span>a copy of this agent's code and values to change and test; this agent is not touched until the branch is merged</span></div>
      <div class="form">
        <label>Branch<input type="text" class="mono" bind:value={brName} /></label>
        <label>What will you change? <small>(optional)</small><input type="text" bind:value={brIntent} placeholder="e.g. a smaller model for low-priority alerts" /></label>
        <div class="acts"><button class="btn2 primary" disabled={!brName.trim()} onclick={makeBranch}>Make the branch</button><button class="btn2 quiet" onclick={() => (branchForm = false)}>Cancel</button></div>
      </div>
    </div>
  {/if}

  <div class="sample"><b>Sample</b> Branches, reviews, merges and releases are not stored yet — this shows how they will work; nothing here is saved. The deployment and the forks are real.</div>

  <h4>Branches <span class="count">{repo.branches.length}</span></h4>
  {#if mergedMsg}<p class="ok">{mergedMsg}</p>{/if}
  <ul class="list">
    {#each repo.branches as b (b.name)}
      <li class="card">
        <div class="row between">
          <div class="row"><span class="tag dev">branch</span><b class="mono">{b.name}</b><span class="dim">{b.by}, {ago(b.created)}</span></div>
          <div class="acts">
            <button class="btn2" onclick={onstudio}>Test in Studio</button>
            <button class="btn2 primary" onclick={() => { merging = b.name; dropping = ''; approver = ''; }}>Merge</button>
            <button class="btn2 quiet" onclick={() => { dropping = b.name; merging = ''; }}>Drop</button>
          </div>
        </div>
        <ul class="changes">{#each b.changes as c}<li>{c}</li>{:else}<li class="dim">No changes yet — open it in the Studio.</li>{/each}</ul>
        {#if merging === b.name}
          <div class="note">Merging makes <b>v{(Number(prodTag.slice(1)) || 0) + 1}</b> of {agent.name}; callers get it on their next message.
            Its values and code replace the agent's; what it learned on the branch (procedures) is added to the agent's memory.
            {#if env === 'prod'}<label class="appr">It is a <b>prod</b> agent: a Security approver signs off first
              <select bind:value={approver}><option value="">— approver —</option><option>security@acme.com</option><option>ciso@acme.com</option></select></label>{/if}
            <span class="acts"><button class="btn2 primary" disabled={env === 'prod' && !approver} onclick={() => { const t = mergeBranch(agent.id, b.name); merging = ''; mergedMsg = `Merged ${b.name} — ${agent.name} is now ${t}${approver ? `, approved by ${approver}` : ''}.`; }}>{env === 'prod' ? 'Request and merge' : 'Merge'}</button>
            <button class="link" onclick={() => (merging = '')}>Cancel</button></span></div>
        {:else if dropping === b.name}
          <div class="note">Drop <b class="mono">{b.name}</b>? Its changes and what it learned are thrown away; prod is not touched.
            <span class="acts"><button class="btn2 danger" onclick={() => { dropBranch(agent.id, b.name); dropping = ''; }}>Drop it</button>
            <button class="link" onclick={() => (dropping = '')}>Cancel</button></span></div>
        {/if}
      </li>
    {:else}
      <li class="empty">No branches.</li>
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
  .kv { grid-template-columns: 100px 1fr; }
  .card > .note { display: flex; flex-wrap: wrap; gap: 8px; align-items: center; }
  .appr { display: flex; align-items: center; gap: 8px; flex-wrap: wrap; font-size: 12.5px; color: var(--text-2); text-transform: none; letter-spacing: 0; font-weight: 400; }
  .appr select { width: auto; }
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
