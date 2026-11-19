<script lang="ts">
  /// Controls — how to stop it, and what governs it. Block goes through the A3T
  /// control plane (agent_a3t_agents, tool_a3t_agent_block): its data plane then
  /// refuses every call. Pausing its deployment and revoking its tools are real
  /// too. Then what its code runs and the policies that have decided about it.
  import { opsApi } from '$lib/services/api';
  import { factsOf, type AgentRec, type TemplateRec } from './agentData';
  import { setBlocked } from './controlPlane';
  import type { AgentLive } from './agentLive.svelte';

  let { agent, templates, live, canAdmin, onrevoke, onchanged, onreload, ongo }: {
    agent: AgentRec & { status?: string };
    templates: TemplateRec[];
    live: AgentLive;
    canAdmin: boolean;
    /// Revoke every tool (true) or give back what it had (false); an error, or null.
    onrevoke: (revoke: boolean) => Promise<string | null>;
    onchanged: () => void;
    /// Its record changed (blocked or unblocked): reload it.
    onreload: () => void;
    ongo: (tab: string) => void;
  } = $props();

  const f = $derived(factsOf(agent, templates));
  const revoked = $derived(Array.isArray(f.tools) && f.tools.length === 0 && f.mcp.length > 0);
  const p = $derived(live.placement);
  let busy = $state('');
  let msg = $state('');
  let confirm = $state('');

  // svelte-ignore state_referenced_locally — its status as loaded; a block or unblock updates it
  let status = $state(agent.status ?? '');
  $effect(() => { status = agent.status ?? ''; });
  const blocked = $derived(status === 'blocked');
  let reason = $state('');
  async function block(on: boolean) {
    busy = 'block'; msg = '';
    const r = await setBlocked(agent.id, on, on ? reason.trim() || undefined : undefined);
    busy = ''; confirm = ''; reason = '';
    if (!r.ok) { msg = r.error ?? 'Not changed'; return; }
    status = r.status ?? (on ? 'blocked' : 'live');
    onreload();
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
</script>

<section class="agt">
  <div class="card stop">
    <div class="card-head"><h3>Stop it</h3><span>each takes effect at once and can be undone</span></div>
    {#if !canAdmin}<p class="dim">Only its project's admins can use these.</p>{/if}
    <ul class="sw">
      <li>
        <div><b>Block calls</b><span class="dim">{blocked ? 'Blocked: every call to it is refused — its chat endpoint and its webhook. Its record and memory stay.' : 'Open: it answers on its data plane (POST /chat) and its webhook, if it has one.'}</span></div>
        {#if blocked}<button class="btn2" disabled={!canAdmin || !!busy} onclick={() => block(false)}>{busy === 'block' ? '…' : 'Unblock'}</button>
        {:else}<button class="btn2 danger" disabled={!canAdmin || !!busy} onclick={() => (confirm = 'block')}>Block</button>{/if}
      </li>
      {#if confirm === 'block'}<li class="cf">Block every call to {agent.name}? Apps calling it get an error until it is unblocked.
        <input class="why" type="text" bind:value={reason} placeholder="Why (kept in its history)" />
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
    <div class="card-head"><h3>What its code runs</h3><span>around every turn</span><span class="grow"></span><button class="link" onclick={() => ongo('hooks')}>Hooks →</button></div>
    <p class="dim">The core agent's error handler (<span class="mono">system/flow_system_error_handler</span>, locked) answers every failed turn, and every turn is logged.{f.hooks.length ? ` Its template adds ${f.hooks.length} hook${f.hooks.length === 1 ? '' : 's'}: ${f.hooks.map((h) => h.name).join(', ')}.` : ' Its template adds no hooks: nothing filters what goes in or out (no redaction, no guard).'}</p>
  </div>

  <p class="dim">Which policies decide about it, and a test of what they would decide: <button class="link" onclick={() => ongo('audit')}>Audit → Policies</button>.</p>
</section>

<style>
  .stop { border-color: color-mix(in srgb, var(--err) 25%, var(--border)); }
  .sw { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; }
  .sw li { display: flex; align-items: center; justify-content: space-between; gap: 14px; padding: 10px 0; border-top: 1px solid var(--divider, var(--border)); }
  .sw li:first-child { border-top: 0; }
  .sw li > div { display: flex; flex-direction: column; gap: 2px; }
  .sw b { font-size: 13px; color: var(--text-1); }
  .sw li.cf { justify-content: flex-start; flex-wrap: wrap; padding: 10px 12px; border-radius: 8px; background: var(--bg-subtle); border-top: 0; font-size: 12.5px; }
  .why { flex: 1 1 200px; height: 28px; font-size: 12.5px; }
  .mono { font-family: var(--font-mono); font-size: 0.92em; }
</style>
