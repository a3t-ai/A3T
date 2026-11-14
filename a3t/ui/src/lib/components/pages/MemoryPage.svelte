<script lang="ts">
  /// Memory, by agent. Items lists the agents; the one picked opens here, in the
  /// agents' frame: Overview (what it remembers, kind by kind), Memory (a bar of
  /// the kinds across the page — pick one, filter it), AQL (read its memory with
  /// RECALL). Read only: memory is changed through the agent's own Memory tab.
  import { untrack } from 'svelte';
  import SlidePanel from '$lib/components/shared/SlidePanel.svelte';
  import PropertiesTree from '$lib/components/shared/PropertiesTree.svelte';
  import WizardHead from '$lib/components/shared/WizardHead.svelte';
  import '$lib/components/agent/agent.css';
  import '$lib/components/triggers/triggers.css';
  import { mcpCall } from '$lib/services/api';
  import { getActivePanel, navigateTo, navigateToDetail } from '$lib/stores/panels.svelte.ts';
  import { insight } from '$lib/components/agent/controlPlane';
  import { KINDS, recall, runAql, isRead, MEMORY_RESOURCE, type Kind, type Rec } from '$lib/components/memory/aql';

  type Agent = { id: string; name: string; namespace: string; mode: string; status: string };
  let agent = $state<Agent | null>(null);
  let space = $state<{ namespace: string; counts: Record<string, number>; attached: { namespace: string; entries: any[] }[] } | null>(null);
  let spaceErr = $state('');
  let tab = $state('overview');

  // ── The agent picked in Items ──
  let shown = '';
  $effect(() => {
    const id = getActivePanel()?.props?.selectedId as string | undefined;
    untrack(() => { if (id && id !== shown) { shown = id; void open(id); } else if (!id) { shown = ''; agent = null; } });
  });
  async function open(id: string) {
    tab = 'overview'; space = null; spaceErr = ''; records = []; aqlRes = null; filters = [{ field: '', value: '' }]; contains = '';
    const l = await mcpCall('list_agents', {});
    const rows: any[] = l.ok ? (Array.isArray(l.data) ? l.data : (l.data?.agents ?? [])) : [];
    const r = rows.find((x) => (x.id ?? x.name) === id);
    agent = r ? { id: r.id ?? r.name, name: r.name ?? r.id, namespace: r.namespace ?? '', mode: r.mode ?? r.agent_mode ?? 'session', status: r.status ?? '' } : { id, name: id, namespace: '', mode: '', status: '' };
    const m = await insight.memory(id);
    if (m.ok && m.data) space = { namespace: m.data.namespace, counts: m.data.counts ?? {}, attached: m.data.attached ?? [] };
    else spaceErr = m.error ?? 'Its memory could not be read.';
    if (space) { kind = 'EPISODIC'; aql = recall('EPISODIC', space.namespace, [], 50); }
  }
  const close = () => navigateTo('memory', 'Memory');

  // ── Memory tab: the kind bar, filters, results ──
  /// A kind of the agent's own space, or an attached semantic space ("space:<ns>").
  let kind = $state<string>('EPISODIC');
  let filters = $state<{ field: string; value: string }[]>([{ field: '', value: '' }]);
  let contains = $state('');
  let limit = $state(50);
  let records = $state<Rec[]>([]);
  let running = $state(false);
  let err = $state('');
  let lastAql = $state('');
  const kindSpace = $derived(kind.startsWith('space:') ? kind.slice(6) : space?.namespace ?? '');
  const kindType = $derived((kind.startsWith('space:') ? 'SEMANTIC' : kind) as Kind);
  const count = (k: string) => k.startsWith('space:') ? (space?.attached.find((a) => a.namespace === k.slice(6))?.entries.length ?? 0) : (space?.counts?.[k.toLowerCase()] ?? 0);
  /// The bar's parts: each kind, then each attached space, with its own light colour.
  const TONE: Record<string, [string, string]> = {
    WORKING: ['var(--ephemeral)', 'var(--ephemeral-bg)'], EPISODIC: ['var(--control)', 'var(--control-bg)'],
    SEMANTIC: ['var(--runtime)', 'var(--runtime-bg)'], PROCEDURAL: ['var(--signal)', 'var(--signal-bg)'],
  };
  const parts = $derived.by(() => {
    const all = [
      ...KINDS.map((k) => ({ k: k.k as string, label: k.label, space: false, c: TONE[k.k][0], cb: TONE[k.k][1] })),
      ...(space?.attached ?? []).map((a) => ({ k: `space:${a.namespace}`, label: a.namespace, space: true, c: 'var(--spec)', cb: 'var(--spec-bg)' })),
    ].map((p) => ({ ...p, n: count(p.k) }));
    const sum = all.reduce((t, p) => t + p.n, 0);
    // A share of the bar by count, but never so thin a part can't be clicked.
    return all.map((p) => ({ ...p, w: sum ? Math.max(p.n / sum, 0.08) : 1 }));
  });
  const total = $derived(parts.reduce((t, p) => t + p.n, 0));
  const shownRecs = $derived(contains.trim() ? records.filter((r) => JSON.stringify(r.data).toLowerCase().includes(contains.trim().toLowerCase())) : records);

  async function apply() {
    if (!kindSpace) return;
    running = true; err = '';
    lastAql = recall(kindType, kindSpace, filters, limit);
    const r = await runAql(lastAql);
    running = false;
    if (r.ok) records = [...r.records].sort((a, b) => String(b.at).localeCompare(String(a.at)));
    else { records = []; err = r.error; }
  }
  function pick(k: string) { kind = k; filters = [{ field: '', value: '' }]; contains = ''; void apply(); }
  $effect(() => { if (tab === 'memory' && agent && space && !lastAql) untrack(() => void apply()); });

  // ── AQL tab ──
  let aql = $state('');
  let aqlRes = $state<{ records: Rec[]; ms: number | null } | null>(null);
  let aqlErr = $state('');
  let showRaw = $state(false);
  async function runIt() {
    running = true; aqlErr = '';
    const r = await runAql(aql);
    running = false;
    if (r.ok) aqlRes = { records: r.records, ms: r.ms }; else { aqlRes = null; aqlErr = r.error; }
  }
  function toAql() { aql = lastAql || recall(kindType, kindSpace, filters, limit); tab = 'aql'; }

  /// One line for a record: what it says, by kind.
  function line(r: Rec): string {
    const d: any = r.data;
    if (d.subject || d.predicate) return `${d.subject ?? '?'} — ${d.predicate ?? '?'} — ${d.object ?? '?'}`;
    if (d.role || d.content) return `${d.role ? d.role + ': ' : ''}${String(d.content ?? d.text ?? '').slice(0, 220)}`;
    const rest = Object.entries(d).filter(([k]) => !['key', 'at', 'created_at', 'kind'].includes(k));
    return rest.slice(0, 3).map(([k, v]) => `${k}: ${typeof v === 'string' ? v : JSON.stringify(v)}`).join(' · ').slice(0, 220);
  }
  const when = (iso: string) => { const d = new Date(iso); return isNaN(+d) ? '' : d.toLocaleString(undefined, { day: 'numeric', month: 'short', hour: '2-digit', minute: '2-digit' }); };

  const steps = [
    { id: 'overview', label: 'Overview', icon: '<rect x="2" y="2" width="12" height="12" rx="2"/><path d="M5 6h6M5 9h4"/>', hint: 'What it remembers, kind by kind, and where it is kept' },
    { id: 'memory', label: 'Memory', icon: '<ellipse cx="8" cy="4" rx="6" ry="2"/><path d="M2 4v8c0 1.1 2.7 2 6 2s6-.9 6-2V4"/><path d="M2 8c0 1.1 2.7 2 6 2s6-.9 6-2"/>', hint: 'Pick a kind of memory, then filter what it holds' },
    { id: 'aql', label: 'AQL', icon: '<path d="M5 3L1 8l4 5M11 3l4 5-4 5"/>', hint: 'Read its memory with RECALL — read only' },
  ];
</script>

{#snippet recList(list: Rec[])}
  {#if list.length}
    <ul class="tl">
      {#each list as r, i (r.id + i)}
        <li class="ev">
          <span class="ic">{i + 1}</span>
          <details>
            <summary class="t">{line(r) || r.key}</summary>
            <div class="m mono">{r.key}</div>
            <dl class="kv kvm">{#each Object.entries(r.data) as [k, v] (k)}<dt>{k}</dt><dd class="mono">{typeof v === 'string' ? v : JSON.stringify(v)}</dd>{/each}</dl>
          </details>
          <time>{when(r.at)}</time>
        </li>
      {/each}
    </ul>
  {:else}<p class="empty">Nothing matches.</p>{/if}
{/snippet}

<SlidePanel open={!!agent} onclose={close}>
  {#if agent}
    <div class="panel-layout">
      <PropertiesTree icon={agent.name.slice(0, 2).toUpperCase()} color="#1a7158" name={agent.name} meta={`memory · ${space?.namespace ?? agent.id}`}
        nodes={[]} {steps} activeNode={tab} onselect={(id) => (tab = id)} />
      <div class="panel-col"><div class="panel-content"><div class="pf">
        <WizardHead step={steps.find((s) => s.id === tab)} />
        <section class="agt">
        {#if spaceErr}<div class="note bad">{spaceErr}</div>{/if}

        {#if tab === 'overview'}
          <div class="card">
            <div class="card-head"><h3>1 · Agent</h3><span>whose memory this is</span></div>
            <ul class="rows">
              <li><span class="lbl">Agent</span><span class="val">{agent.name} <span class="dim">· {agent.namespace}</span></span><button class="btn2" onclick={() => navigateToDetail('agents', 'Agents', agent!.id)}>Open</button></li>
              <li><span class="lbl">Remembers</span><span class="val">{agent.mode === 'persistent' ? 'across conversations' : 'within one conversation'}</span><span></span></li>
              <li><span class="lbl">Kept in</span><span class="val mono">{MEMORY_RESOURCE} · space {space?.namespace ?? '…'}</span><span></span></li>
            </ul>
          </div>
          <div class="card">
            <div class="card-head"><h3>2 · What it remembers</h3><span>entries of each kind, in its own space and the spaces it reads</span></div>
            <ul class="rows">
              {#each KINDS as k (k.k)}
                <li><span class="lbl">{k.label}</span><span class="val">{space ? count(k.k) : '…'} <span class="dim">· {k.what}</span></span>
                  <button class="btn2" onclick={() => { tab = 'memory'; pick(k.k); }}>Open</button></li>
              {/each}
              {#each space?.attached ?? [] as a (a.namespace)}
                <li><span class="lbl mono">{a.namespace}</span><span class="val">{a.entries.length} <span class="dim">· an attached semantic space</span></span>
                  <button class="btn2" onclick={() => { tab = 'memory'; pick(`space:${a.namespace}`); }}>Open</button></li>
              {/each}
            </ul>
          </div>

        {:else if tab === 'memory'}
          <!-- Like a disk-usage bar: one bar, a light colour per kind, each part
               sized by its share of entries; the bar and the key below both pick. -->
          <div class="card">
            <div class="card-head"><h3>1 · Kinds</h3><span>{space ? `${total} entries — pick a kind` : 'reading…'}</span></div>
            <div class="mbar" role="tablist" aria-label="Memory kinds">
              {#each parts as s (s.k)}
                <button role="tab" class="seg" class:on={kind === s.k} aria-selected={kind === s.k} style="flex-grow:{s.w}; --c:{s.c}; --cb:{s.cb}"
                  title="{s.label}: {s.n}" onclick={() => pick(s.k)}><span>{s.n}</span></button>
              {/each}
            </div>
            <div class="mkey">
              {#each parts as s (s.k)}
                <button class:on={kind === s.k} style="--c:{s.c}; --cb:{s.cb}" onclick={() => pick(s.k)}>
                  <i></i><b class:mono={s.space}>{s.label}</b><span>{space ? s.n : '…'}</span></button>
              {/each}
            </div>
          </div>
          <div class="card">
            <div class="card-head"><h3>2 · Filter</h3><span>{kind.startsWith('space:') ? 'facts in this space' : KINDS.find((k) => k.k === kind)?.what}</span></div>
            {#each filters as f, i (i)}
              <div class="frow">
                <input class="mono fin" type="text" bind:value={f.field} placeholder={kindType === 'SEMANTIC' ? 'subject' : kindType === 'EPISODIC' ? 'session_id' : 'key'} />
                <span class="dim">=</span>
                <input class="mono fin grow" type="text" bind:value={f.value} placeholder="value" onkeydown={(e) => { if (e.key === 'Enter') apply(); }} />
                {#if filters.length > 1}<button class="btn2 quiet" onclick={() => (filters = filters.filter((_, j) => j !== i))}>Remove</button>{/if}
              </div>
            {/each}
            <div class="acts">
              <button class="btn2 quiet" onclick={() => (filters = [...filters, { field: '', value: '' }])}>+ Field</button>
              <label class="lim">at most <input class="fin num" type="number" min="1" max="1000" bind:value={limit} /></label>
              <span class="grow"></span>
              <button class="btn2 primary" disabled={running} onclick={apply}>{running ? 'Reading…' : 'Apply'}</button>
            </div>
            {#if lastAql}<p class="dim">As AQL: <span class="mono">{lastAql}</span> <button class="link" onclick={toAql}>Edit as AQL →</button></p>{/if}
            {#if err}<div class="note bad">{err}</div>{/if}
          </div>
          <div class="card">
            <div class="card-head"><h3>3 · Entries</h3><span>{shownRecs.length}{contains ? ` of ${records.length}` : ''}, newest first</span><span class="grow"></span>
              <input class="fin rm" type="search" bind:value={contains} placeholder="Find in these" /></div>
            {@render recList(shownRecs)}
          </div>

        {:else}
          <div class="card">
            <div class="card-head"><h3>1 · RECALL</h3><span>Ctrl+Enter runs it · only RECALL is sent</span></div>
            <textarea class="aql" rows="4" spellcheck="false" bind:value={aql} onkeydown={(e) => { if ((e.ctrlKey || e.metaKey) && e.key === 'Enter') runIt(); }}></textarea>
            <div class="acts">
              {#each KINDS as k (k.k)}<button class="btn2 quiet" onclick={() => (aql = recall(k.k, space?.namespace ?? '', [], 50))}>{k.label}</button>{/each}
              <span class="grow"></span>
              <button class="btn2 primary" disabled={running || !aql.trim() || !isRead(aql)} onclick={runIt}>{running ? 'Running…' : 'Run'}</button>
            </div>
            {#if aql.trim() && !isRead(aql)}<div class="note warn">Only RECALL runs here. To change what it remembers, use the agent's Memory tab.</div>{/if}
            {#if aqlErr}<div class="note bad">{aqlErr}</div>{/if}
          </div>
          {#if aqlRes}
            <div class="card">
              <div class="card-head"><h3>2 · Result</h3><span>{aqlRes.records.length} record{aqlRes.records.length === 1 ? '' : 's'}{aqlRes.ms != null ? ` · ${aqlRes.ms} ms` : ''}</span><span class="grow"></span>
                <button class="btn2 quiet rm" onclick={() => (showRaw = !showRaw)}>{showRaw ? 'List' : 'JSON'}</button></div>
              {#if showRaw}<pre class="code out">{JSON.stringify(aqlRes.records.map((r) => r.data), null, 2)}</pre>{:else}{@render recList(aqlRes.records)}{/if}
            </div>
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
  /* The kinds across the page: each a tab with its count. */
  .mbar { display: flex; gap: 3px; height: 34px; border-radius: 8px; overflow: hidden; }
  .mbar .seg { flex: 1 1 0; min-width: 28px; display: flex; align-items: center; padding: 0 10px; border: 0; background: var(--cb); color: var(--c); cursor: pointer; font: 600 12.5px var(--font-sans); font-variant-numeric: tabular-nums; }
  .mbar .seg span { overflow: hidden; white-space: nowrap; }
  .mbar .seg:hover { filter: brightness(0.96); }
  .mbar .seg.on { box-shadow: inset 0 0 0 2px var(--c); }
  .mkey { display: flex; flex-wrap: wrap; gap: 6px 8px; margin-top: 12px; }
  .mkey button { display: inline-flex; align-items: center; gap: 7px; padding: 5px 10px; border: 1px solid var(--border); border-radius: 999px; background: var(--bg-surface); cursor: pointer; font: inherit; }
  .mkey i { width: 10px; height: 10px; border-radius: 3px; background: var(--cb); box-shadow: inset 0 0 0 1px var(--c); }
  .mkey b { font: 500 13px var(--font-sans); color: var(--text-1); }
  .mkey span { font: 600 13px var(--font-sans); color: var(--text-3); font-variant-numeric: tabular-nums; }
  .mkey button.on { border-color: var(--c); background: var(--cb); }
  .mkey button.on span { color: var(--c); }
  .frow { display: flex; align-items: center; gap: 8px; }
  .fin { height: 34px; padding: 0 10px; border: 1px solid var(--border); border-radius: 8px; background: var(--bg-surface); font: 13px var(--font-sans); color: var(--text-1); min-width: 0; }
  .fin.mono { font-family: var(--font-mono); font-size: 12.5px; }
  .fin.num { width: 76px; margin-left: 6px; }
  .frow > .fin:first-child { flex: 0 0 34%; }
  .frow > .fin.grow { flex: 1 1 auto; }
  .lim { font-size: 12.5px; color: var(--text-3); text-transform: none; letter-spacing: 0; font-weight: 400; display: inline-flex; align-items: center; }
  .aql { width: 100%; box-sizing: border-box; padding: 10px 12px; border: 1px solid var(--border); border-radius: 8px; background: var(--bg-surface); font: 12.5px/1.6 var(--font-mono); color: var(--text-1); resize: vertical; }
  .kvm { margin-top: 6px; grid-template-columns: 120px 1fr; font-size: 12px; }
  details summary { cursor: pointer; list-style: none; }
  details summary::-webkit-details-marker { display: none; }
</style>
