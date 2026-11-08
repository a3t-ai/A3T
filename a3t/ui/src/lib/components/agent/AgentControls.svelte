<script lang="ts">
  /// Controls — how to stop it, and what governs it. The stop switches are
  /// real: block its endpoint, pause its deployment, revoke its tools. Then the
  /// hooks its code runs, the policies that have decided about it, and the
  /// approval rules for changing it (Sample).
  import { mcpCall, opsApi } from '$lib/services/api';
  import { navigateTo } from '$lib/stores/panels.svelte.ts';
  import { factsOf, envOf, type AgentRec, type TemplateRec } from './agentData';
  import type { AgentLive } from './agentLive.svelte';

  let { agent, templates, live, canAdmin, onrevoke, onchanged }: {
    agent: AgentRec;
    templates: TemplateRec[];
    live: AgentLive;
    canAdmin: boolean;
    /// Revoke every tool (true) or give back what it had (false); an error, or null.
    onrevoke: (revoke: boolean) => Promise<string | null>;
    onchanged: () => void;
  } = $props();

  const f = $derived(factsOf(agent, templates));
  const revoked = $derived(Array.isArray(f.tools) && f.tools.length === 0 && f.mcp.length > 0);
  const p = $derived(live.placement);
  let busy = $state('');
  let msg = $state('');
  let confirm = $state('');

  async function block(on: boolean) {
    if (!live.trigger) return;
    busy = 'block'; msg = '';
    const r = await mcpCall(on ? 'pause_trigger' : 'enable_trigger', { id: live.trigger.id });
    busy = ''; confirm = '';
    if (!r.ok) { msg = r.error ?? 'Not changed'; return; }
    await live.loadTrigger(f.endpoint);
  }
  async function pause(on: boolean) {
    if (!p?.id) return;
    busy = 'pause'; msg = '';
    const r = await opsApi.deployment(p.id, on ? 'pause' : 'resume', on ? `Paused from the agent's Controls` : undefined);
    busy = ''; confirm = '';
    if (!r.ok) { msg = r.error ?? 'Not changed'; return; }
    onchanged();
  }
  async function revoke(on: boolean) {
    busy = 'revoke'; msg = '';
    msg = (await onrevoke(on)) ?? '';
    busy = ''; confirm = '';
  }
  /// Policies named in the decisions about it.
  const policies = $derived.by(() => {
    const m = new Map<string, { id: string; allow: number; deny: number }>();
    for (const d of live.decisions) for (const id of (d.reason || '').split(/,\s*/).filter((x) => x && !/\s/.test(x))) {
      const e = m.get(id) ?? { id, allow: 0, deny: 0 };
      d.allow ? e.allow++ : e.deny++;
      m.set(id, e);
    }
    return [...m.values()].sort((a, b) => b.deny - a.deny || b.allow - a.allow);
  });
  // Sample approval rules: who must approve a change, by environment.
  let rules = $state({ prod: true, test: false, tools: true });
</script>

<section class="agt">
  <div class="card stop">
    <div class="card-head"><h3>Stop it</h3><span>each takes effect at once and can be undone</span></div>
    {#if !canAdmin}<p class="dim">Only its project's admins can use these.</p>{/if}
    <ul class="sw">
      <li>
        <div><b>Block calls</b><span class="dim">{live.trigger ? `Its endpoint /webhook${live.trigger.path} — ${live.trigger.enabled ? 'open' : 'blocked: callers get an error'}` : f.endpoint ? 'Finding its endpoint…' : 'It has no endpoint.'}</span></div>
        {#if live.trigger}
          {#if live.trigger.enabled}<button class="btn2 danger" disabled={!canAdmin || !!busy} onclick={() => (confirm = 'block')}>Block</button>
          {:else}<button class="btn2" disabled={!canAdmin || !!busy} onclick={() => block(false)}>{busy === 'block' ? '…' : 'Unblock'}</button>{/if}
        {/if}
      </li>
      {#if confirm === 'block'}<li class="cf">Block every call to {agent.name}? Apps calling it get an error until it is unblocked.
        <button class="btn2 danger" onclick={() => block(true)}>{busy === 'block' ? 'Blocking…' : 'Block calls'}</button><button class="link" onclick={() => (confirm = '')}>Cancel</button></li>{/if}
      <li>
        <div><b>Pause its deployment</b><span class="dim">{p ? `${p.runtime || 'its runtime'} — ${p.deploymentState === 'paused' ? 'paused' : 'running'}` : 'Not deployed: it runs on the shared runtime, which is not paused for one agent — block its calls instead.'}</span></div>
        {#if p}
          {#if p.deploymentState === 'paused'}<button class="btn2" disabled={!canAdmin || !!busy} onclick={() => pause(false)}>{busy === 'pause' ? '…' : 'Resume'}</button>
          {:else}<button class="btn2 danger" disabled={!canAdmin || !!busy} onclick={() => (confirm = 'pause')}>Pause</button>{/if}
        {/if}
      </li>
      {#if confirm === 'pause'}<li class="cf">Pause it? Its runtime stops taking new work; runs in progress finish.
        <button class="btn2 danger" onclick={() => pause(true)}>{busy === 'pause' ? 'Pausing…' : 'Pause'}</button><button class="link" onclick={() => (confirm = '')}>Cancel</button></li>{/if}
      <li>
        <div><b>Revoke its tools</b><span class="dim">{revoked ? 'Revoked: it can call no tool.' : f.mcp.length ? `${f.mcp.length} tool server${f.mcp.length === 1 ? '' : 's'}${f.tools ? `, ${f.tools.length} tools allowed` : ', every tool'}.` : 'It has no tools.'}</span></div>
        {#if f.mcp.length}
          {#if revoked}<button class="btn2" disabled={!canAdmin || !!busy} onclick={() => revoke(false)}>{busy === 'revoke' ? '…' : 'Give back'}</button>
          {:else}<button class="btn2 danger" disabled={!canAdmin || !!busy} onclick={() => (confirm = 'revoke')}>Revoke</button>{/if}
        {/if}
      </li>
      {#if confirm === 'revoke'}<li class="cf">Revoke every tool? It keeps answering, but can no longer act. What it had is kept, to give back.
        <button class="btn2 danger" onclick={() => revoke(true)}>{busy === 'revoke' ? 'Revoking…' : 'Revoke tools'}</button><button class="link" onclick={() => (confirm = '')}>Cancel</button></li>{/if}
    </ul>
    {#if msg}<p class="err">{msg}</p>{/if}
  </div>

  <div class="card">
    <div class="card-head"><h3>Hooks</h3><span>what its code runs around each step — from its template</span></div>
    {#if f.hooks.length}
      <table class="tbl">
        <thead><tr><th>Hook</th><th>Applies to</th><th>Runs</th></tr></thead>
        <tbody>{#each f.hooks as h (h.name)}<tr><td class="mono">{h.name}</td><td>{h.extend}{h.when ? ` · when ${h.when}` : ''}</td><td class="mono">{h.flows.join(', ') || '—'}</td></tr>{/each}</tbody>
      </table>
    {:else}
      <p class="dim">Its template has no hooks — nothing filters what goes in or out (no redaction, no guard).</p>
    {/if}
  </div>

  <div class="card">
    <div class="card-head"><h3>Policies</h3><span>that have decided about it, in the latest {live.scanned} decisions</span><span class="grow"></span>
      <button class="link" onclick={() => navigateTo('security', 'Security', { selectedId: 'policies' })}>All policies →</button></div>
    {#if live.decErr}<p class="dim">{live.decErr}</p>
    {:else if policies.length}
      <table class="tbl">
        <thead><tr><th>Policy</th><th>Allowed</th><th>Blocked</th></tr></thead>
        <tbody>{#each policies as po (po.id)}<tr><td class="mono">{po.id}</td><td>{po.allow}</td><td class:err={po.deny > 0}>{po.deny}</td></tr>{/each}</tbody>
      </table>
    {:else}
      <p class="dim">No decision about it names a policy yet. Which policies <em>would</em> apply needs a policy query (UI-6).</p>
    {/if}
  </div>

  <div class="card">
    <div class="card-head"><h3>Approval to change it</h3><span class="tag sample">sample</span></div>
    <label class="ck"><input type="checkbox" bind:checked={rules.prod} /> A change to a <b>prod</b> agent needs a Security approver</label>
    <label class="ck"><input type="checkbox" bind:checked={rules.test} /> A change to a <b>test</b> agent needs its project's admin</label>
    <label class="ck"><input type="checkbox" bind:checked={rules.tools} /> Adding a tool server or a destructive tool always needs a Security approver</label>
    <p class="dim">It is <b>{envOf(agent) || 'not set to an environment'}</b>. Approvals are not enforced yet (SRV-20) — this shows how they will be set.</p>
  </div>
</section>

<style>
  .stop { border-color: color-mix(in srgb, var(--err) 25%, var(--border)); }
  .sw { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; }
  .sw li { display: flex; align-items: center; justify-content: space-between; gap: 14px; padding: 10px 0; border-top: 1px solid var(--divider, var(--border)); }
  .sw li:first-child { border-top: 0; }
  .sw li > div { display: flex; flex-direction: column; gap: 2px; }
  .sw b { font-size: 13px; color: var(--text-1); }
  .sw li.cf { justify-content: flex-start; flex-wrap: wrap; padding: 10px 12px; border-radius: 8px; background: var(--bg-subtle); border-top: 0; font-size: 12.5px; }
  .ck { display: flex; align-items: center; gap: 8px; font-size: 13px; color: var(--text-2); text-transform: none; letter-spacing: 0; font-weight: 400; }
  .ck input { width: auto; }
  td.err { color: var(--err); }
</style>
