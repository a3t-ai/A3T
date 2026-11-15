<script lang="ts">
  /// Releases — where it runs, its versions and its branches. A version is a
  /// snapshot: its template version and a tag on its memory. A branch is a copy of
  /// its memory to try changes on: merged back when only procedures changed and
  /// nothing conflicts, or promoted to a new agent (evolve). Both are FlowR's
  /// /agents/{id}/versions and /branches, on the agent's memory space. Its
  /// deployment is real (deploy / undeploy; pausing is in Controls).
  import { untrack, type Snippet } from 'svelte';
  import { opsApi, agentSpaceApi, type AgentVersion, type MemoryBranch } from '$lib/services/api';
  import { navigateTo } from '$lib/stores/panels.svelte.ts';
  import { factsOf, forksOf, envOf, ago, type AgentRec, type TemplateRec } from './agentData';
  import type { AgentLive } from './agentLive.svelte';
  import AgentTryIt from './AgentTryIt.svelte';

  let { agent, templates, allAgents, live, canEdit, onopen, onmade, onchanged, notDeployed, branchForm = $bindable(false) }: {
    agent: AgentRec & { created_at?: string };
    templates: TemplateRec[];
    allAgents: AgentRec[];
    live: AgentLive;
    canEdit: boolean;
    onopen: (id: string) => void;
    /// A branch became a new agent: reload the list and open it.
    onmade: (id: string) => void;
    /// The deployment changed: reload what is shown.
    onchanged: () => void;
    /// How to deploy it when it is not deployed (the page's profile picker).
    notDeployed: Snippet;
    branchForm?: boolean;
  } = $props();

  const f = $derived(factsOf(agent, templates));
  const forks = $derived(forksOf(agent, allAgents));
  const p = $derived(live.placement);
  const env = $derived(envOf(agent));

  // ── deployment ──
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

  // ── its memory space, versions and branches ──
  let space = $state<{ own: boolean; namespace: string | null } | null>(null);
  let versions = $state<AgentVersion[] | null>(null);
  let branches = $state<MemoryBranch[] | null>(null);
  let err = $state('');
  let msg = $state('');
  async function load(id: string) {
    space = null; versions = null; branches = null; err = '';
    const m = await agentSpaceApi.memory(id);
    if (agent.id !== id) return;
    if (!m.ok) { err = m.error ?? 'Its memory space could not be read.'; return; }
    space = { own: !!m.data?.own, namespace: m.data?.namespace ?? null };
    if (!space.own) return;
    const [v, b] = await Promise.all([agentSpaceApi.versions(id), agentSpaceApi.branches(id)]);
    if (agent.id !== id) return;
    versions = v.ok ? [...(v.data?.versions ?? [])].sort((x, y) => Number(y.version) - Number(x.version)) : [];
    branches = b.ok ? b.data?.branches ?? [] : [];
    if (!v.ok || !b.ok) err = (v.ok ? b.error : v.error) ?? 'Not read.';
  }
  $effect(() => { const id = agent.id; untrack(() => load(id)); });

  async function register() {
    busy = 'register'; err = '';
    const r = await agentSpaceApi.register(agent.id);
    busy = '';
    if (!r.ok) { err = r.error ?? 'Not registered.'; return; }
    await load(agent.id);
  }

  let note = $state('');
  async function makeVersion() {
    busy = 'version'; err = ''; msg = '';
    const r = await agentSpaceApi.makeVersion(agent.id, note.trim());
    busy = '';
    if (!r.ok) { err = r.error ?? 'No version made.'; return; }
    note = ''; msg = `Version ${r.data?.version} made.`;
    await load(agent.id);
  }

  let brName = $state('');
  $effect(() => { if (branchForm) untrack(() => { brName = `try-${new Date().toISOString().slice(5, 10).replace('-', '')}`; }); });
  const brValid = $derived(/^[a-z0-9][a-z0-9-]{0,62}$/.test(brName));
  async function makeBranch() {
    busy = 'branch'; err = ''; msg = '';
    const r = await agentSpaceApi.branch(agent.id, brName);
    busy = '';
    if (!r.ok) { err = r.error ?? 'No branch made.'; return; }
    branchForm = false; msg = `Branch ${brName} made from its memory now.`;
    await load(agent.id);
  }

  /// Per branch: what merging it would do, then the action chosen.
  let open = $state('');
  let check = $state<{ clean: number; conflicts: number; main: number; changes: { key: string; kind: string; text: string; conflict: boolean }[] } | null>(null);
  /// A branch being tried: a conversation with it on that branch.
  let trying = $state('');
  /// One line for a changed memory entry.
  const entryLine = (e: any): { key: string; kind: string; text: string } => {
    const d = e?.data ?? e?.branch ?? e?.theirs ?? e ?? {};
    const text = d.content ?? d.text ?? d.object ?? d.summary ?? JSON.stringify(d).slice(0, 160);
    return { key: String(e?.key ?? d.key ?? ''), kind: String(e?.type ?? e?.kind ?? d.kind ?? ''), text: String(text).slice(0, 200) };
  };
  let promoteName = $state('');
  async function review(name: string) {
    open = name; check = null; err = ''; msg = '';
    promoteName = `${agent.name}-${name}`.slice(0, 63);
    const r = await agentSpaceApi.conflicts(agent.id, name);
    if (!r.ok) { err = r.error ?? 'Not compared.'; return; }
    const clean: any[] = r.data?.clean ?? [], conflicts: any[] = r.data?.conflicts ?? [];
    check = { clean: clean.length, conflicts: conflicts.length, main: r.data?.main?.offset ?? 0,
      changes: [...conflicts.map((e) => ({ ...entryLine(e), conflict: true })), ...clean.map((e) => ({ ...entryLine(e), conflict: false }))] };
  }
  async function merge(name: string) {
    if (!check) return;
    busy = 'merge'; err = '';
    const r = await agentSpaceApi.merge(agent.id, name, { main_offset: check.main, resolutions: [] });
    busy = '';
    if (!r.ok) { err = r.error ?? 'Not merged.'; return; }
    msg = `Merged ${name}: ${r.data?.added ?? 0} added, ${r.data?.changed ?? 0} changed.`;
    open = '';
    await load(agent.id);
  }
  async function promote(name: string) {
    busy = 'promote'; err = '';
    const id = `agt-${crypto.randomUUID().slice(0, 8)}`;
    const r = await agentSpaceApi.promote(agent.id, name, { id, name: promoteName });
    busy = '';
    if (!r.ok) { err = r.error ?? 'Not promoted.'; return; }
    open = '';
    onmade(id);
  }
  async function drop(name: string) {
    busy = 'drop'; err = '';
    const r = await agentSpaceApi.dropBranch(agent.id, name);
    busy = '';
    if (!r.ok) { err = r.error ?? 'Not dropped.'; return; }
    open = ''; msg = `Branch ${name} dropped.`;
    await load(agent.id);
  }
</script>

<section class="agt">
  <div class="card">
    <div class="card-head"><h3>Deployment</h3><span class="grow"></span>
      {#if !branchForm && space?.own}<button class="btn2" onclick={() => (branchForm = true)}>Branch</button>{/if}</div>
    <dl class="kv">
      <dt>Environment</dt><dd>{#if env}<span class="tag" class:prod={env === 'prod'} class:dev={env !== 'prod'}>{env}</span>{:else}<span class="dim">not set</span>{/if} <span class="dim">— its owner's setting, in Settings</span></dd>
      <dt>Version</dt><dd>{#if versions?.length}<b class="mono">v{versions[0].version}</b> <span class="dim">— {versions[0].note || 'no note'}, {ago(versions[0].created_at)}</span>{:else}<span class="dim">none yet</span>{/if}</dd>
      <dt>Made from</dt><dd class="mono">{agent.template_id}@{agent.template_version}</dd>
      <dt>Runs on</dt><dd>{#if live.loading}…{:else if p}<span class="row"><span class="dot {p.deploymentState === 'paused' ? 'warn' : 'ok'}"></span>{p.deploymentState === 'paused' ? 'paused' : 'deployed'} on <span class="mono">{p.runtime || '—'}</span>{p.canvas ? ` · ${p.canvas}` : ''}</span>
        {:else}<span class="row"><span class="dot"></span>not deployed — the shared runtime</span>{/if}</dd>
      <dt>Webhook</dt><dd class="mono">{f.endpoint ? `/webhook${f.endpoint}` : "none"}</dd>
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

  {#if err}<p class="err">{err}</p>{/if}
  {#if msg}<p class="ok">{msg}</p>{/if}

  {#if space && !space.own}
    <div class="card">
      <div class="card-head"><h3>Versions and branches</h3></div>
      <p class="dim">They work on its memory space: the namespace its memory lives in on the org's ADB, <span class="mono">{agent.id}</span>. It is not registered yet; agents made from now on get it when they are made.</p>
      <div class="acts"><button class="btn2 primary" disabled={!canEdit || !!busy} onclick={register}>{busy === 'register' ? 'Registering…' : 'Register its memory space'}</button></div>
    </div>
  {:else if space}
    {#if branchForm}
      <div class="card">
        <div class="card-head"><h3>Branch</h3><span>a copy of its memory as it is now, to try changes on; the agent is not touched until the branch is merged</span></div>
        <div class="form">
          <label>Name<input type="text" class="mono" bind:value={brName} /></label>
          {#if brName && !brValid}<span class="err">Lower-case letters, digits and dashes.</span>{/if}
          <div class="acts"><button class="btn2 primary" disabled={!brValid || !!busy} onclick={makeBranch}>{busy === 'branch' ? 'Making…' : 'Make the branch'}</button><button class="btn2 quiet" onclick={() => (branchForm = false)}>Cancel</button></div>
        </div>
      </div>
    {/if}

    <h4>Versions <span class="count">{versions?.length ?? '…'}</span></h4>
    <div class="row">
      <input type="text" class="grow" bind:value={note} placeholder="What this version is (optional)" />
      <button class="btn2" disabled={!canEdit || !!busy} onclick={makeVersion}>{busy === 'version' ? 'Making…' : 'Make a version'}</button>
    </div>
    {#if versions?.length}
      <table class="tbl">
        <thead><tr><th>Version</th><th>Code</th><th>Memory</th><th>Note</th><th>By</th><th>When</th></tr></thead>
        <tbody>{#each versions as v (v.version)}<tr><td class="mono k">v{v.version}</td><td class="mono">{v.template?.id}@{v.template?.version}</td><td class="mono">{v.memory?.tag ?? '—'}</td><td>{v.note || '—'}</td><td class="mono">{v.created_by}</td><td class="when">{ago(v.created_at)}</td></tr>{/each}</tbody>
      </table>
    {:else if versions}<p class="dim">No versions yet. A version keeps its code version and a tag on its memory, to come back to.</p>{/if}

    <h4>Branches <span class="count">{branches?.length ?? '…'}</span></h4>
    <ul class="list">
      {#each branches ?? [] as b (b.name)}
        <li class="card">
          <div class="row between">
            <div class="row"><span class="tag dev">branch</span><b class="mono">{b.name}</b><span class="dim">{b.created_by}, {ago(Math.floor(b.point_ts / 1000))}</span></div>
            <div class="acts">
              <button class="btn2" disabled={!canEdit} onclick={() => (trying = trying === b.name ? '' : b.name)}>{trying === b.name ? 'Close' : 'Try it on this branch'}</button>
              <button class="btn2 primary" disabled={!canEdit || !!busy} onclick={() => review(b.name)}>Merge or promote…</button>
              <button class="btn2 quiet" disabled={!canEdit || !!busy} onclick={() => drop(b.name)}>{busy === 'drop' ? '…' : 'Drop'}</button>
            </div>
          </div>
          {#if trying === b.name}
            <AgentTryIt agentId={agent.id} branch={b.name} />
          {/if}
          {#if open === b.name}
            {#if check?.changes.length}
              <table class="tbl"><thead><tr><th>What the branch changes</th><th>Kind</th><th></th></tr></thead>
                <tbody>{#each check.changes as c, i (i)}<tr><td>{c.text}</td><td class="mono">{c.kind}</td><td class:bad={c.conflict}>{c.conflict ? 'conflicts' : 'merges'}</td></tr>{/each}</tbody></table>
            {:else if check}<p class="dim">The branch has no changes of its own yet: try it on the branch first.</p>{/if}
            <div class="note">
              {#if !check}Comparing it with its memory now…
              {:else if check.conflicts === 0}
                <span>Nothing conflicts{check.clean ? `; ${check.clean} change${check.clean === 1 ? '' : 's'} merge cleanly` : ''}. Merging adds what it learned (procedures) to {agent.name}.</span>
                <span class="acts"><button class="btn2 primary" disabled={!!busy} onclick={() => merge(b.name)}>{busy === 'merge' ? 'Merging…' : 'Merge'}</button></span>
              {:else}
                <span>{check.conflicts} conflict{check.conflicts === 1 ? '' : 's'} with its memory now: it can't merge back. Promote it to a new agent instead (evolve).</span>
              {/if}
              {#if check}
                <span class="acts"><input type="text" class="mono" bind:value={promoteName} aria-label="New agent's name" />
                  <button class="btn2" disabled={!!busy || !/^[a-z0-9][a-z0-9-]{1,62}$/.test(promoteName)} onclick={() => promote(b.name)}>{busy === 'promote' ? 'Promoting…' : 'Promote to a new agent'}</button>
                  <button class="link" onclick={() => (open = '')}>Cancel</button></span>
              {/if}
            </div>
          {/if}
        </li>
      {:else}
        {#if branches}<li class="empty">No branches.</li>{/if}
      {/each}
    </ul>
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
  .kv { grid-template-columns: 100px 1fr; }
  .card > .note { display: flex; flex-wrap: wrap; gap: 8px; align-items: center; }
  .list { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 8px; }
  .list .card { gap: 8px; padding: 12px 14px; }
  .list .note { display: flex; flex-wrap: wrap; gap: 8px; align-items: center; }
  .grow { flex: 1 1 auto; }
  .ok { color: var(--green, #1a7158); font-size: 12.5px; margin: 0; }
  .empty { border: 1px dashed var(--border); border-radius: 8px; }
  .when { white-space: nowrap; color: var(--text-4); width: 1%; }
  .pick { width: 100%; display: flex; gap: 10px; align-items: baseline; padding: 9px 12px; border: 1px solid var(--border); border-radius: 8px; background: var(--bg-surface); cursor: pointer; font: inherit; font-size: 13px; text-align: left; }
  .pick:hover { border-color: var(--spec); }
  .pick b { flex: 1; font-weight: 500; }
</style>
