<script lang="ts">
  /// The agent inventory (GOV-10): every agent of an org (or of every org, for
  /// platform InfoSec) — owner, what it calls and reaches, keys, last activity,
  /// and a risk tier with the reason. Export as CSV for an audit.
  import { untrack } from 'svelte';
  import PropertiesTree from '$lib/components/shared/PropertiesTree.svelte';
  import WizardHead from '$lib/components/shared/WizardHead.svelte';
  import '$lib/components/agent/agent.css';
  import { agentInventory, getAccessToken, loadDirectory, type InventoryRow } from '$lib/services/api';
  import { API_BASE } from '$lib/config';
  import AgentSecurity from './AgentSecurity.svelte';

  let { org }: { org: string } = $props();
  let rows = $state<InventoryRow[] | null>(null);
  let err = $state('');
  let risk = $state('');
  let q = $state('');
  let open = $state<InventoryRow | null>(null);
  async function load(o: string) {
    rows = null; err = '';
    const r = await agentInventory(o === 'all' ? undefined : o);
    if (o !== org) return;
    if (!r.ok) { err = r.status === 403 ? 'The inventory is for an org\'s admins and security admins, and platform InfoSec.' : (r.error ?? 'It could not be read.'); return; }
    rows = r.data?.agents ?? [];
  }
  $effect(() => { const o = org; untrack(() => void load(o)); });
  // Owners are user ids: show their name or email where the directory knows them.
  let people = $state<Record<string, string>>({});
  $effect(() => { untrack(() => void loadDirectory().then((d) => { people = Object.fromEntries(d.map((u) => [u.id, u.name || u.email])); }).catch(() => {})); });
  const ownerOf = (id: string) => people[id] ?? (/^[0-9a-f-]{36}$/.test(id) ? `${id.slice(0, 8)}…` : id);
  const shown = $derived((rows ?? []).filter((r) => (!risk || r.risk === risk) && (!q.trim() || `${r.name} ${r.id} ${r.project} ${r.owner} ${people[r.owner] ?? ''} ${(r.mcp_servers ?? []).join(' ')}`.toLowerCase().includes(q.trim().toLowerCase()))));
  const n = (t: string) => (rows ?? []).filter((r) => r.risk === t).length;
  async function csv() {
    const res = await fetch(`${API_BASE}/security/agents?format=csv${org === 'all' ? '' : `&org=${encodeURIComponent(org)}`}`, { headers: { authorization: `Bearer ${getAccessToken() ?? ''}` } });
    const blob = await res.blob();
    const a = document.createElement('a'); a.href = URL.createObjectURL(blob); a.download = `a3t-agents-${org}.csv`; a.click(); URL.revokeObjectURL(a.href);
  }
  const when = (s?: string | null) => { const t = s ? Date.parse(s) : NaN; return Number.isFinite(t) ? new Date(t).toLocaleDateString(undefined, { month: 'short', day: 'numeric' }) : '—'; };
  const step = { id: 'inventory', label: 'Agents', icon: '<rect x="3" y="1" width="10" height="7" rx="2"/><path d="M5 8v2a3 3 0 006 0V8"/>', hint: 'Every agent: who owns it, what it reaches, its keys, and how risky it looks' };
</script>

{#if open}
  {#key open.id}<AgentSecurity row={open} onback={() => (open = null)} />{/key}
{:else}
<div class="panel-layout">
  <PropertiesTree icon="AG" color="#5530a8" name="Agents" meta={org === 'all' ? 'every org' : `org ${org}`} nodes={[]} steps={[step]} activeNode="inventory" onselect={() => {}} />
  <div class="panel-col"><div class="panel-content"><div class="pf">
    <WizardHead {step} />
    <section class="agt">
      {#if err}<div class="note bad">{err}</div>
      {:else if !rows}<p class="dim">Reading…</p>
      {:else}
        <div class="card">
          <div class="card-head"><h3>1 · {rows.length} agent{rows.length === 1 ? '' : 's'}</h3><span>{n('high')} high · {n('medium')} medium · {n('low')} low</span><span class="grow"></span>
            <button class="btn2" onclick={csv}>Export CSV</button></div>
          <div class="filters">
            <input type="search" class="grow" bind:value={q} placeholder="Filter by name, project, owner or server" aria-label="Filter agents" />
            <select bind:value={risk} aria-label="Risk"><option value="">Any risk</option><option value="high">High</option><option value="medium">Medium</option><option value="low">Low</option></select>
          </div>
          <div class="tscroll"><table class="tbl">
            <thead><tr><th>Risk</th><th>Agent</th><th>Reaches</th><th class="num">Keys</th><th>Own rules</th><th>Last used</th></tr></thead>
            <tbody>
              {#each shown as r (r.id)}
                <tr class="click" onclick={() => (open = r)}>
                  <td><span class="risk {r.risk}" title={r.why}>{r.risk}</span></td>
                  <td class="who"><button class="link" onclick={(e) => { e.stopPropagation(); open = r; }}>{r.name}</button>{#if r.status === 'blocked'} <span class="dim">· stopped</span>{/if}
                    <small><span class="mono">{r.project}</span> · <span title={r.owner}>{ownerOf(r.owner)}</span></small></td>
                  <td class="reach">{(r.mcp_servers ?? []).join(', ') || '—'}{r.egress ? ` · only ${r.egress.join(', ')}` : ''}</td>
                  <td class="num">{r.keys_live}</td>
                  <td>{r.own_policy ? `${r.rules} rule${r.rules === 1 ? '' : 's'}${r.role_ceiling ? ' · ceiling' : ''}` : '—'}</td>
                  <td class="nw">{when(r.last_activity)}</td>
                </tr>
              {/each}
            </tbody>
          </table></div>
          <p class="dim">High: tools on servers outside A3T, or tools that delete, with no rule of its own and no egress list — or a live key that never expires. Medium: it reaches outside A3T or has live keys, with limits set. Low: A3T's own tools, no live keys.</p>
        </div>
      {/if}
    </section>
  </div></div></div>
</div>
{/if}

<style>
  .risk { padding: 1px 8px; border-radius: 10px; font-size: 11.5px; font-weight: 600; text-transform: capitalize; }
  .risk.high { background: color-mix(in srgb, var(--err, #b42318) 14%, transparent); color: var(--err, #b42318); }
  .risk.medium { background: color-mix(in srgb, #b45309 14%, transparent); color: #b45309; }
  .risk.low { background: color-mix(in srgb, var(--ok, #1a7158) 14%, transparent); color: var(--ok, #1a7158); }
  .pf { padding: 24px 32px; max-width: var(--detail-w); box-sizing: border-box; }
  .grow { flex: 1 1 auto; }
  .filters { display: flex; gap: 8px; margin: 4px 0 12px; }
  .filters input { flex: 1 1 240px; min-width: 0; height: 36px; padding: 0 12px; font-size: 13px; }
  .filters select { flex: 0 0 150px; width: 150px; height: 36px; padding-top: 0; padding-bottom: 0; font-size: 13px; }
  .tbl td { vertical-align: top; }
  .nw { white-space: nowrap; }
  .num { text-align: right; }
  .tscroll { overflow-x: auto; }
  .who { min-width: 0; }
  .who .link { overflow-wrap: anywhere; text-align: left; }
  .who small { display: block; margin-top: 2px; font-size: 11.5px; color: var(--text-4); }
  .reach { overflow-wrap: anywhere; min-width: 140px; }
  @container details (max-width: 560px) {
    .tbl thead { display: none; }
    .tbl, .tbl tbody { display: block; }
    .tbl tr { display: grid; grid-template-columns: auto minmax(0, 1fr) auto; column-gap: 10px; row-gap: 4px; padding: 10px 12px; border-bottom: 1px solid var(--border); }
    .tbl td { padding: 0; border: 0; }
    .tbl td:nth-child(3) { grid-column: 2 / 4; grid-row: 2; font-size: 12px; color: var(--text-3); }
    .tbl td:nth-child(4), .tbl td:nth-child(5) { display: none; }
    .tbl td:nth-child(6) { grid-column: 3; grid-row: 1; font-size: 12px; color: var(--text-4); }
    .reach { min-width: 0; }
    .pf { padding: 16px 14px; } .filters { flex-wrap: wrap; } .filters select { flex: 1 1 100%; width: 100%; } }
  tr.click { cursor: pointer; }
  tr.click:hover td { background: var(--bg-subtle); }
</style>
