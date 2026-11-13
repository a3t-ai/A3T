<script lang="ts">
  /// The Decision log — every allow, deny and approval asked at the policy choke
  /// points. SuperAdmin and SecurityAdmin only (the server refuses anyone else).
  /// Items lists views (all, denials, approvals asked); the one picked opens here
  /// in the agents' frame, scoped to the project selected in the top bar:
  /// Overview (counts, what is refused most, who), Decisions (the entries,
  /// filterable, each one's full detail).
  import { untrack } from 'svelte';
  import SlidePanel from '$lib/components/shared/SlidePanel.svelte';
  import PropertiesTree from '$lib/components/shared/PropertiesTree.svelte';
  import WizardHead from '$lib/components/shared/WizardHead.svelte';
  import '$lib/components/agent/agent.css';
  import '$lib/components/triggers/triggers.css';
  import { audit, getEdition } from '$lib/services/api';
  import { getActivePanel, navigateTo, navigateToDetail } from '$lib/stores/panels.svelte.ts';
  import { getProject } from '$lib/stores/context.svelte.ts';
  import { contextRev_ } from '$lib/stores/apps.svelte.ts';
  import { inBackground } from '$lib/core/status/activity';
  import { onDestroy } from 'svelte';

  interface Rec {
    ts_ms: number; principal_kind: string; principal_id: string; on_behalf_of?: string | null;
    action: string; resource_type: string; resource_id: string;
    decision: string; policy_ids: string[]; reason?: string | null; project?: string | null;
  }
  const VIEWS: Record<string, { label: string; hint: string }> = {
    all: { label: 'All decisions', hint: 'every allow, deny and approval asked' },
    deny: { label: 'Denials', hint: 'what was refused, and why' },
    ask: { label: 'Approvals asked', hint: 'what waited for a person to approve' },
  };

  let v = $state<string | null>(null);
  let tab = $state('overview');
  let recs = $state<Rec[]>([]);
  let loading = $state(false);
  let err = $state('');
  let edition = $state<'ce' | 'ee' | null>(null);
  let scope = $state('');
  /// The project in the top bar; empty = all projects.
  let project = $state(getProject());

  let shown = '';
  $effect(() => {
    const p = getActivePanel()?.props ?? {};
    const id = p.selectedId as string | undefined;
    untrack(() => {
      if (p.q && !q) q = String(p.q);
      if (id && id !== shown && VIEWS[id]) { shown = id; v = id; tab = 'overview'; open = null; void load(); } else if (!id) { shown = ''; v = null; }
    });
  });
  // A different project in the top bar is a different log.
  let rev = contextRev_();
  $effect(() => { const r = contextRev_(); if (r !== rev) untrack(() => { rev = r; project = getProject(); if (v) void load(); }); });

  async function load() {
    loading = true; err = '';
    if (!edition) { const e = await getEdition(); edition = e.ok && e.data?.edition === 'ee' ? 'ee' : 'ce'; }
    const r = await audit.recent(1000, v === 'deny', project || undefined);
    loading = false;
    if (!r.ok) { recs = []; err = r.status === 403 ? 'The Decision log is for SuperAdmin and SecurityAdmin.' : (r.error ?? 'It could not be read.'); return; }
    scope = r.data?.scope ?? '';
    recs = ((r.data?.records ?? []) as Rec[]).filter((x) => v !== 'ask' || x.decision === 'ask');
  }
  const close = () => navigateTo('logs', 'Decision log');

  // ── Filters ──
  let q = $state('');
  let who = $state('');
  let dec = $state('');
  const filtered = $derived(recs.filter((r) =>
    (!dec || r.decision === dec) && (!who || r.principal_id === who || r.on_behalf_of === who) &&
    (!q.trim() || `${r.principal_id} ${r.on_behalf_of ?? ''} ${r.action} ${r.resource_type} ${r.resource_id} ${r.reason ?? ''} ${(r.policy_ids ?? []).join(' ')} ${r.project ?? ''}`.toLowerCase().includes(q.trim().toLowerCase()))));
  let open = $state<Rec | null>(null);

  // ── Overview ──
  const count = (d: string) => recs.filter((r) => r.decision === d).length;
  const top = (key: (r: Rec) => string, list: Rec[], n = 5) => {
    const m = new Map<string, number>();
    for (const r of list) { const k = key(r); if (k) m.set(k, (m.get(k) ?? 0) + 1); }
    return [...m.entries()].sort((a, b) => b[1] - a[1]).slice(0, n);
  };
  const denials = $derived(recs.filter((r) => r.decision === 'deny'));
  const topDenied = $derived(top((r) => `${r.action} · ${r.resource_id}`, denials));
  const topWho = $derived(top((r) => r.on_behalf_of ? `${r.principal_id} for ${r.on_behalf_of}` : r.principal_id, recs));
  const span = $derived(recs.length ? `${new Date(recs[recs.length - 1].ts_ms).toLocaleString()} – ${new Date(recs[0].ts_ms).toLocaleTimeString()}` : '');

  // Follow live.
  let live = $state(false);
  let timer: any = null;
  function toggleLive() { live = !live; if (live) timer = setInterval(() => void inBackground(load), 4000); else { clearInterval(timer); timer = null; } }
  onDestroy(() => { if (timer) clearInterval(timer); });

  const steps = $derived([
    { id: 'overview', label: 'Overview', icon: '<rect x="2" y="2" width="12" height="12" rx="2"/><path d="M5 6h6M5 9h4"/>', hint: `${VIEWS[v ?? 'all']?.hint ?? ''} — ${project ? `project ${project}` : 'all projects'}` },
    { id: 'list', label: 'Decisions', icon: '<rect x="2" y="1" width="12" height="14" rx="1.5"/><path d="M5 5h6M5 8h6M5 11h3"/>', hint: 'Each decision: who, for whom, what, which policy, and why' },
  ]);
  const when = (ms: number) => (ms ? new Date(ms).toLocaleString(undefined, { month: 'short', day: 'numeric', hour: '2-digit', minute: '2-digit', second: '2-digit' }) : '—');
  const mark = (d: string) => (d === 'deny' ? '✕' : d === 'ask' ? '?' : '✓');
</script>

<SlidePanel open={!!v} onclose={close}>
  {#if v}
    <div class="panel-layout">
      <PropertiesTree icon={v === 'deny' ? '✕' : v === 'ask' ? '?' : 'DL'} color="#5530a8" name={VIEWS[v].label} meta={`decision log · ${project || 'all projects'}`} nodes={[]} {steps} activeNode={tab} onselect={(id) => (tab = id)} />
      <div class="panel-col"><div class="panel-content"><div class="pf">
        <WizardHead step={steps.find((s) => s.id === tab)} />
        <section class="agt">
        {#if err}<div class="note bad">{err}</div>
        {:else}
          {#if edition === 'ce'}<div class="note">A Community runtime allows everything and decides nothing, so this log stays empty. Connect an EE runtime to enforce and record decisions.</div>{/if}
          <div class="note warn">Scope: {scope || 'this FlowR replica, in memory'} — each FlowR keeps its own recent decisions, and a restart clears them (CA-39).</div>

          {#if tab === 'overview'}
            <div class="card">
              <div class="card-head"><h3>1 · Decisions</h3><span>{loading ? 'reading…' : span || 'none yet'}</span><span class="grow"></span>
                <button class="btn2 quiet rm" onclick={toggleLive}>{live ? 'Following' : 'Follow live'}</button></div>
              <div class="tiles">
                <button class="tile" onclick={() => { dec = ''; tab = 'list'; }}><span class="v">{recs.length}</span><span class="k">{project ? `in ${project}` : 'in all projects'}</span></button>
                <button class="tile" onclick={() => { dec = 'allow'; tab = 'list'; }}><span class="v">{count('allow')}</span><span class="k">allowed</span></button>
                <button class="tile" class:bad={count('deny') > 0} onclick={() => { dec = 'deny'; tab = 'list'; }}><span class="v">{count('deny')}</span><span class="k">denied</span></button>
                <button class="tile" onclick={() => { dec = 'ask'; tab = 'list'; }}><span class="v">{count('ask')}</span><span class="k">asked a person</span></button>
              </div>
            </div>
            <div class="card">
              <div class="card-head"><h3>2 · Refused most</h3><span>action · resource</span></div>
              {#if topDenied.length}<ul class="rows wide">{#each topDenied as [k, n] (k)}<li><span class="lbl mono">{k}</span><span class="val">{n} time{n === 1 ? '' : 's'}</span><button class="btn2" onclick={() => { q = k.split(' · ')[1]; dec = 'deny'; tab = 'list'; }}>See</button></li>{/each}</ul>
              {:else}<p class="empty">Nothing refused.</p>{/if}
            </div>
            <div class="card">
              <div class="card-head"><h3>3 · Who</h3><span>the principals behind the most decisions</span></div>
              {#if topWho.length}<ul class="rows wide">{#each topWho as [k, n] (k)}<li><span class="lbl mono">{k}</span><span class="val">{n}</span><button class="btn2" onclick={() => { who = k.split(' for ')[0]; dec = ''; tab = 'list'; }}>See</button></li>{/each}</ul>
              {:else}<p class="empty">No decisions.</p>{/if}
            </div>

          {:else}
            <div class="card">
              <div class="card-head"><h3>Filter</h3><span>{filtered.length} of {recs.length}</span><span class="grow"></span><button class="btn2 quiet rm" disabled={loading} onclick={load}>{loading ? 'Reading…' : 'Refresh'}</button></div>
              <div class="frow">
                <input class="fin grow" type="search" bind:value={q} placeholder="Who, action, resource, policy, reason" />
                <select class="fin" bind:value={dec}><option value="">any decision</option><option value="allow">allowed</option><option value="deny">denied</option><option value="ask">asked</option></select>
                {#if who}<button class="btn2 quiet" onclick={() => (who = '')}>{who} ✕</button>{/if}
              </div>
            </div>
            <ul class="tl">
              {#each filtered.slice(0, 300) as r, i (r.ts_ms + ':' + i)}
                <li class="ev" class:err={r.decision === 'deny'}>
                  <span class="ic">{mark(r.decision)}</span>
                  <button class="evb" onclick={() => (open = open === r ? null : r)}>
                    <div class="t">{r.principal_id}{r.on_behalf_of ? ` for ${r.on_behalf_of}` : ''} · <span class="mono">{r.action}</span> {r.resource_type} <span class="mono">{r.resource_id}</span></div>
                    {#if r.reason}<div class="m">{r.reason}</div>{/if}
                    {#if open === r}
                      <dl class="kv kvd">
                        <dt>Decision</dt><dd>{r.decision}</dd>
                        <dt>Principal</dt><dd class="mono">{r.principal_kind} · {r.principal_id}</dd>
                        {#if r.on_behalf_of}<dt>For</dt><dd class="mono">{r.on_behalf_of}</dd>{/if}
                        <dt>Action</dt><dd class="mono">{r.action}</dd>
                        <dt>Resource</dt><dd class="mono">{r.resource_type} · {r.resource_id}</dd>
                        <dt>Project</dt><dd class="mono">{r.project ?? 'platform-wide'}</dd>
                        <dt>Policies</dt><dd class="mono">{(r.policy_ids ?? []).join(', ') || '—'}</dd>
                        <dt>When</dt><dd>{new Date(r.ts_ms).toISOString()}</dd>
                      </dl>
                    {/if}
                  </button>
                  <time>{when(r.ts_ms)}</time>
                </li>
              {:else}<li class="ev"><span></span><span class="m">{loading ? 'Reading…' : 'No decisions match.'}</span><span></span></li>{/each}
            </ul>
            {#if filtered.length > 300}<p class="dim">The first 300 shown — narrow the filter.</p>{/if}
          {/if}
        {/if}
        </section>
      </div></div></div>
    </div>
  {/if}
</SlidePanel>

<style>
  .panel-layout { display: grid; grid-template-columns: 280px 1fr; height: 100%; }
  .panel-col { display: flex; flex-direction: column; min-height: 0; min-width: 0; height: 100%; }
  .panel-content { overflow-y: auto; flex: 1; min-height: 0; }
  .pf { padding: 24px 32px; max-width: var(--detail-w); box-sizing: border-box; }
  @container details (max-width: 560px) { .pf { padding: 16px 14px; } }
  .frow { display: flex; align-items: center; gap: 8px; flex-wrap: wrap; }
  .fin { height: 34px; padding: 0 10px; border: 1px solid var(--border); border-radius: 8px; background: var(--bg-surface); font: 13px var(--font-sans); color: var(--text-1); min-width: 0; }
  .fin.grow { flex: 1 1 240px; }
  .evb { all: unset; display: block; cursor: pointer; min-width: 0; }
  .kvd { margin-top: 8px; grid-template-columns: 100px 1fr; font-size: 12px; }
</style>
