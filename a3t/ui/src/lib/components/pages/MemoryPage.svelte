<script lang="ts">
  /// Memory, by agent: every agent in view, how its memory is kept, and a way
  /// into its Memory tab. (This page used to show made-up sample agents.) Live
  /// memory contents need the agent-memory API — .ai/A3T_UI_LIMITATIONS.md UI-1.
  import { onMount } from 'svelte';
  import { execFlow } from '$lib/services/api';
  import { navigateToDetail } from '$lib/stores/panels.svelte.ts';
  import { inContext } from '$lib/stores/context.svelte.ts';
  import { contextRev_ } from '$lib/stores/apps.svelte.ts';

  type A = { id: string; name: string; namespace: string; mode: string; status: string; mcp: string[] };
  let agents = $state<A[]>([]);
  let loading = $state(true);
  let error = $state('');

  async function load() {
    loading = true; error = '';
    const r = await execFlow('flow_a3t_agents-list', 'a3t', { org_id: 'org-default' }, { silent: true });
    if (!r.ok) { error = r.error ?? 'Agents could not be listed'; loading = false; return; }
    const rows: any[] = Array.isArray(r.data) ? r.data : (r.data?.rows ?? r.data?.output ?? []);
    agents = rows.map((x) => {
      let vars: any = {};
      try { vars = typeof x.variables === 'string' ? JSON.parse(x.variables) : (x.variables ?? {}); } catch { /* none */ }
      return { id: x.id, name: x.name ?? x.id, namespace: x.namespace ?? 'a3t', mode: x.mode ?? 'session', status: x.status ?? 'draft', mcp: vars.mcp_servers ?? [] };
    }).filter((a) => inContext(a.namespace, 'shared'));
    loading = false;
  }
  let rev = -1;
  $effect(() => { const r = contextRev_(); if (r !== rev) { rev = r; void load(); } });
  onMount(() => { if (rev === -1) void load(); });

  const memoryOf = (a: A) => a.mcp.find((m) => /memory/i.test(m)) ?? '';
  const open = (a: A) => navigateToDetail('agents', 'Agents', a.id, { tab: 'memory' });
</script>

<div class="screen-header">
  <div class="breadcrumb">Memory <span class="sep">/</span> By agent</div>
  <h1 class="screen-title">Agent memory</h1>
</div>
<div class="body">
  <div class="banner">
    What each agent remembers — working, episodic, semantic and procedural memory, kept in ADB. Open an agent to see how its memory is set up.
    <span class="dim">Reading the entries themselves, and what an agent learned when, needs the agent-memory API (UI-1, UI-5).</span>
  </div>
  {#if loading}
    <!-- the header bar shows loading -->
  {:else if error}<p class="err">{error}</p>
  {:else}
    <table class="tbl">
      <thead><tr><th>Agent</th><th>Project</th><th>Mode</th><th>Memory service</th><th></th></tr></thead>
      <tbody>
        {#each agents as a (a.id)}
          <tr onclick={() => open(a)}>
            <td><b>{a.name}</b><div class="dim mono">{a.id}</div></td>
            <td class="mono">{a.namespace}</td>
            <td>{a.mode === 'persistent' ? 'persistent — across sessions' : a.mode === 'session' ? 'session — one conversation' : a.mode}</td>
            <td>{memoryOf(a) ? `${memoryOf(a)} (ADB)` : 'working memory only'}</td>
            <td class="go">Open memory →</td>
          </tr>
        {:else}
          <tr><td colspan="5" class="dim">No agents in this project.</td></tr>
        {/each}
      </tbody>
    </table>
  {/if}
</div>

<style>
  .screen-header { padding: 18px 32px 12px; border-bottom: 1px solid var(--border); background: var(--bg-page); }
  .breadcrumb { font-size: 12px; color: var(--text-3); margin-bottom: 8px; font-family: var(--font-mono); }
  .breadcrumb .sep { color: var(--text-5, var(--text-4)); }
  .screen-title { font-family: var(--font-serif); font-size: 26px; font-weight: 500; margin: 0; }
  .body { padding: 24px 32px; display: flex; flex-direction: column; gap: 16px; max-width: 1100px; }
  .banner { padding: 12px 14px; border-radius: 8px; background: var(--bg-subtle); border: 1px solid var(--border); font-size: 13px; color: var(--text-2); line-height: 1.55; }
  .dim { color: var(--text-4); font-size: 12px; }
  .mono { font-family: var(--font-mono); }
  .err { color: var(--err); }
  .tbl { width: 100%; border-collapse: collapse; background: var(--bg-surface); border: 1px solid var(--border); border-radius: 8px; overflow: hidden; font-size: 13px; }
  .tbl th { text-align: left; font-size: 10.5px; font-weight: 600; text-transform: uppercase; letter-spacing: 0.05em; color: var(--text-4); padding: 10px 14px; border-bottom: 1px solid var(--border); }
  .tbl td { padding: 10px 14px; border-top: 1px solid var(--divider, var(--border)); vertical-align: top; }
  .tbl tbody tr { cursor: pointer; }
  .tbl tbody tr:hover td { background: var(--bg-subtle); }
  .go { color: var(--spec); white-space: nowrap; text-align: right; }
</style>
