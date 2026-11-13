<script lang="ts">
  /// Build › Memory — what it remembers, each kind shown as what it is:
  /// episodic as a timeline (newest first), working by conversation, semantic as a
  /// graph that turns plus its facts, procedural as learned steps. Read from its
  /// memory space on the org's ADB through agent_a3t_agents (tool_a3t_agent_memory).
  /// Every entry can be edited or forgotten here (tool_a3t_agent_memory_put /
  /// _forget), or with AQL in the Studio. Semantic spaces (shared facts, e.g.
  /// a3t-usage) attach to the agent: the core recalls from them each turn.
  import { insight, type MemoryType } from './controlPlane';
  import { ago, type AgentRec } from './agentData';
  import { openInStudio } from '$lib/stores/panels.svelte';
  import SemanticGraph, { type Fact } from './SemanticGraph.svelte';

  let { agent }: { agent: AgentRec } = $props();

  type Entry = { key: string | null; at: string | null; data: Record<string, any>; space?: string };
  type Space = { namespace: string; entries: Entry[]; error?: string };
  let mem = $state<{ namespace: string; counts: Record<MemoryType, number>; memory: Record<MemoryType, Entry[]>; attached: Space[] } | null>(null);
  let err = $state('');
  let open = $state<MemoryType>('episodic');
  let filter = $state('');

  /// Entries as {key, at, data}, newest first (an older control plane sent the data alone).
  const norm = (list: any[]): Entry[] => (list ?? []).map((e) =>
    e && typeof e === 'object' && 'data' in e && ('at' in e || 'key' in e) ? { key: e.key ?? null, at: e.at ?? null, data: e.data ?? {} } : { key: null, at: null, data: e ?? {} })
    .sort((a, b) => (b.at ?? '').localeCompare(a.at ?? ''));

  async function load(id: string) {
    err = '';
    const r = await insight.memory(id);
    if (agent.id !== id) return;
    if (!r.ok) { err = r.error ?? 'Its memory could not be read.'; return; }
    const m: any = r.data;
    mem = {
      namespace: m.namespace, counts: m.counts,
      memory: Object.fromEntries(Object.entries(m.memory ?? {}).map(([k, v]) => [k, norm(v as any[])])) as any,
      attached: (m.attached ?? []).map((s: any) => ({ namespace: s.namespace, error: s.error, entries: norm(s.entries).map((e) => ({ ...e, space: s.namespace })) })),
    };
  }
  $effect(() => { mem = null; editing = null; adding = null; picked = ''; space = 'all'; load(agent.id); });

  const TYPES: { k: MemoryType; t: string; d: string }[] = [
    { k: 'episodic', t: 'Episodic', d: 'what happened: its answers, turn by turn' },
    { k: 'working', t: 'Working', d: 'the conversations it is in now' },
    { k: 'semantic', t: 'Semantic', d: 'facts it knows, and spaces attached' },
    { k: 'procedural', t: 'Procedural', d: 'steps it has learned' },
  ];
  const keeps = $derived(agent.mode === 'persistent');
  const conv = (sid: unknown) => { const s = String(sid ?? ''); return s.includes(':') ? s.split(':').slice(1).join(':') : s || '—'; };
  const text = (v: unknown) => (typeof v === 'string' ? v : v == null ? '' : JSON.stringify(v));
  const time = (at: string | null) => (at ? new Date(at).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' }) : '');
  const day = (at: string | null) => {
    if (!at) return 'Undated';
    const d = new Date(at), today = new Date();
    const y = new Date(); y.setDate(today.getDate() - 1);
    if (d.toDateString() === today.toDateString()) return 'Today';
    if (d.toDateString() === y.toDateString()) return 'Yesterday';
    return d.toLocaleDateString([], { weekday: 'short', day: 'numeric', month: 'short', year: d.getFullYear() === today.getFullYear() ? undefined : 'numeric' });
  };
  const match = (e: Entry) => !filter.trim() || JSON.stringify(e.data).toLowerCase().includes(filter.trim().toLowerCase());

  const list = $derived((mem?.memory?.[open] ?? []).filter(match));
  /// Episodic, by day (newest day first; newest first within it).
  const days = $derived.by(() => {
    const out: { day: string; items: Entry[] }[] = [];
    for (const e of list) { const d = day(e.at); const last = out[out.length - 1]; if (last?.day === d) last.items.push(e); else out.push({ day: d, items: [e] }); }
    return out;
  });
  /// Working, by conversation (the most recent conversation first).
  const convs = $derived.by(() => {
    const m = new Map<string, Entry[]>();
    for (const e of list) { const c = conv(e.data.session_id); (m.get(c) ?? m.set(c, []).get(c)!).push(e); }
    return [...m.entries()];
  });
  let expanded = $state<Record<string, boolean>>({});

  // ── Semantic: its own facts and the attached spaces ──
  let space = $state<string>('all'); // 'all' | 'own' | <attached namespace>
  let picked = $state('');
  const own = $derived(mem?.namespace ?? agent.id);
  const allFacts = $derived.by((): Entry[] => {
    if (!mem) return [];
    const mine = (mem.memory.semantic ?? []).map((e) => ({ ...e, space: own }));
    const att = mem.attached.flatMap((s) => s.entries);
    return space === 'all' ? [...mine, ...att] : space === 'own' ? mine : att.filter((e) => e.space === space);
  });
  const triples = $derived(allFacts.filter((e) => e.data.subject && e.data.object)
    .map((e): Fact => ({ key: e.key, subject: text(e.data.subject), predicate: text(e.data.predicate), object: text(e.data.object), space: e.space ?? own })));
  const factRows = $derived(allFacts.filter(match).filter((e) => !picked || text(e.data.subject) === picked || text(e.data.object) === picked));
  const semCount = $derived((mem?.counts?.semantic ?? 0) + (mem?.attached ?? []).reduce((n, s) => n + s.entries.length, 0));

  let attachName = $state('a3t-usage');
  let busy = $state(false);
  async function attach() {
    const ns = attachName.trim(); if (!ns) return;
    busy = true; err = '';
    const r = await insight.attach(agent.id, ns);
    busy = false;
    if (!r.ok) { err = r.error ?? `${ns} could not be attached.`; return; }
    attachName = ''; space = ns; await load(agent.id);
  }
  async function detach(ns: string) {
    if (!confirm(`Detach ${ns}? Its facts stay in ADB; this agent stops recalling them.`)) return;
    busy = true; err = '';
    const r = await insight.detach(agent.id, ns);
    busy = false;
    if (!r.ok) { err = r.error ?? `${ns} could not be detached.`; return; }
    if (space === ns) space = 'all';
    await load(agent.id);
  }

  // ── Editing: one entry at a time, as fields (semantic) or JSON ──
  type Edit = { type: MemoryType; key: string | null; space: string; json: string; orig: Record<string, any>; f?: { subject: string; predicate: string; object: string; content: string; confidence: string } };
  let editing = $state<Edit | null>(null);
  let adding = $state<Edit | null>(null);
  let editErr = $state('');
  const editId = (e: Entry) => `${e.space ?? own}|${e.key}`;

  function startEdit(type: MemoryType, e: Entry) {
    adding = null; editErr = '';
    const d = e.data ?? {};
    if (type === 'semantic') queueMicrotask(() => document.querySelector('.agt .card.edit')?.scrollIntoView({ block: 'nearest', behavior: 'smooth' }));
    editing = { type, key: e.key, space: e.space ?? own, orig: d, json: JSON.stringify(d, null, 2),
      f: type === 'semantic' ? { subject: text(d.subject), predicate: text(d.predicate), object: text(d.object), content: text(d.content), confidence: d.confidence != null ? String(d.confidence) : '' } : undefined };
  }
  function startAdd(type: MemoryType) {
    editing = null; editErr = '';
    const sp = type === 'semantic' && space !== 'all' && space !== 'own' ? space : own;
    adding = { type, key: null, space: sp, orig: {}, json: type === 'procedural' ? '{\n  "pattern": "",\n  "steps": []\n}' : '{\n  "content": ""\n}',
      f: type === 'semantic' ? { subject: picked, predicate: '', object: '', content: '', confidence: '0.9' } : undefined };
  }
  /// The fields to write: from the form (semantic) or the JSON. Fields removed from an
  /// entry are set empty, since UPDATE sets only the fields it is given.
  function fields(ed: Edit): Record<string, unknown> | null {
    let data: Record<string, unknown>;
    if (ed.f) {
      const c = ed.f.confidence.trim() === '' ? undefined : Number(ed.f.confidence);
      if (c !== undefined && !(c >= 0 && c <= 1)) { editErr = 'Confidence is a number from 0 to 1.'; return null; }
      if (!ed.f.subject.trim() || !ed.f.predicate.trim() || !ed.f.object.trim()) { editErr = 'A fact needs a subject, a predicate and an object.'; return null; }
      data = { ...ed.orig, subject: ed.f.subject.trim(), predicate: ed.f.predicate.trim(), object: ed.f.object.trim(),
        content: ed.f.content.trim() || `${ed.f.subject.trim()} ${ed.f.predicate.trim()} ${ed.f.object.trim()}.`, ...(c !== undefined ? { confidence: c } : {}), source: ed.orig.source ?? 'console' };
    } else {
      try { const v = JSON.parse(ed.json); if (!v || typeof v !== 'object' || Array.isArray(v)) throw 0; data = v; }
      catch { editErr = 'That is not a JSON object.'; return null; }
    }
    delete data.key; delete data.id;
    for (const k of Object.keys(ed.orig)) if (!(k in data) && k !== 'key' && k !== 'id') data[k] = '';
    return data;
  }
  async function save(ed: Edit) {
    editErr = '';
    const data = fields(ed); if (!data) return;
    busy = true;
    const r = await insight.memoryPut(agent.id, ed.type, data, ed.key, ed.space === own ? undefined : ed.space);
    busy = false;
    if (!r.ok) { editErr = r.error ?? 'It could not be saved.'; return; }
    editing = null; adding = null; await load(agent.id);
  }
  async function forget(type: MemoryType, e: Entry) {
    if (!e.key) return;
    if (!confirm(`Forget this ${type} entry (${e.key})${e.space && e.space !== own ? ` in ${e.space}` : ''}? Reads stop returning it at once.`)) return;
    busy = true; err = '';
    const r = await insight.memoryForget(agent.id, type, e.key, e.space && e.space !== own ? e.space : undefined);
    busy = false;
    if (!r.ok) { err = r.error ?? 'It could not be forgotten.'; return; }
    await load(agent.id);
  }

  /// The same memory as AQL in the Studio's query view on the org's ADB.
  function studio() {
    const ns = open === 'semantic' && space !== 'all' && space !== 'own' ? space : own;
    const aql = `RECALL FROM ${open.toUpperCase()} ALL NAMESPACE "${ns}" LIMIT 100\n\n-- Change one entry:\n-- UPDATE INTO ${open.toUpperCase()} WHERE key = "<key>" (field = "value") NAMESPACE "${ns}"\n-- FORGET FROM ${open.toUpperCase()} WHERE key = "<key>" NAMESPACE "${ns}"`;
    try { localStorage.setItem('a3t.studio.db.draft.a3t://a3t/resource/adb', JSON.stringify(aql)); } catch { /* private window: opens without it */ }
    openInStudio('resource', 'a3t', 'adb');
  }
</script>

{#snippet actions(type: MemoryType, e: Entry)}
  {#if e.key}<span class="acts"><button class="link" disabled={busy} onclick={() => startEdit(type, e)}>Edit</button><button class="link danger" disabled={busy} onclick={() => forget(type, e)}>Forget</button></span>{/if}
{/snippet}

{#snippet editor(ed: Edit, title: string)}
  <div class="card edit">
    <div class="card-head"><h3>{title}</h3><span class="mono">{ed.type} · {ed.space}{ed.key ? ` · ${ed.key}` : ''}</span></div>
    {#if ed.f}
      <div class="form">
        <label>Subject<input type="text" bind:value={ed.f.subject} placeholder="support-bot" /></label>
        <label>Predicate<input type="text" bind:value={ed.f.predicate} placeholder="uses model" /></label>
        <label>Object<input type="text" bind:value={ed.f.object} placeholder="claude-sonnet" /></label>
        <label>Confidence<input type="text" bind:value={ed.f.confidence} inputmode="decimal" placeholder="0.9" /></label>
        <label class="wide">Sentence<input type="text" bind:value={ed.f.content} placeholder="Written from the three above when empty" /></label>
      </div>
    {:else}
      <textarea rows="8" spellcheck="false" bind:value={ed.json}></textarea>
      {#if ed.key}<p class="dim">Fields you remove are set empty.</p>{/if}
    {/if}
    {#if editErr}<p class="err">{editErr}</p>{/if}
    <div class="row"><button class="btn2 primary" disabled={busy} onclick={() => save(ed)}>{busy ? 'Saving…' : ed.key ? 'Save' : 'Add'}</button>
      <button class="btn2 quiet" onclick={() => { editing = null; adding = null; editErr = ''; }}>Cancel</button></div>
  </div>
{/snippet}

<section class="agt">
  <div class="card">
    <div class="card-head"><h3>Memory</h3><span>{keeps ? 'kept across conversations' : agent.mode === 'ephemeral' ? 'it remembers nothing' : 'only the conversation it is in'}</span><span class="grow"></span>
      <button class="link" onclick={() => load(agent.id)}>Refresh</button>
      <button class="link" onclick={studio} title="Open the org's ADB in the Studio with this memory as AQL">Edit with AQL in Studio →</button></div>
    <div class="types" role="tablist">
      {#each TYPES as ty (ty.k)}
        <button role="tab" class:on={open === ty.k} aria-selected={open === ty.k} onclick={() => { open = ty.k; filter = ''; editing = null; adding = null; }}>
          <b>{ty.t}</b><span class="n">{mem ? (ty.k === 'semantic' ? semCount : mem.counts?.[ty.k] ?? 0) : '…'}</span><span class="dim">{ty.d}</span>
        </button>
      {/each}
    </div>
    {#if err}<p class="err">{err}</p>{/if}
    {#if mem && ((open === 'semantic' ? allFacts : mem.memory?.[open]) ?? []).length > 5}<input type="search" bind:value={filter} placeholder="Filter {open} memory" />{/if}
  </div>

  {#if !mem}<p class="dim">{err ? '' : 'Reading its memory…'}</p>
  {:else if open === 'semantic'}
    <div class="card">
      <div class="card-head"><h3>Spaces</h3><span>its own facts, and shared spaces it also knows</span></div>
      <div class="spaces">
        <button class="sp" class:on={space === 'all'} onclick={() => { space = 'all'; picked = ''; }}>All <span class="n">{semCount}</span></button>
        <button class="sp" class:on={space === 'own'} onclick={() => { space = 'own'; picked = ''; }}>Its own <span class="mono">{own}</span> <span class="n">{mem.counts?.semantic ?? 0}</span></button>
        {#each mem.attached as s (s.namespace)}
          <span class="sp att" class:on={space === s.namespace}>
            <button class="plain" onclick={() => { space = s.namespace; picked = ''; }}><span class="mono">{s.namespace}</span> <span class="n">{s.entries.length}</span></button>
            <button class="plain x" title="Detach {s.namespace}" aria-label="Detach {s.namespace}" disabled={busy} onclick={() => detach(s.namespace)}>×</button>
          </span>
        {/each}
        <form class="attach" onsubmit={(e) => { e.preventDefault(); attach(); }}>
          <input type="text" bind:value={attachName} placeholder="space, e.g. a3t-usage" list="sem-spaces" aria-label="Semantic space to attach" />
          <datalist id="sem-spaces"><option value="a3t-usage"></option></datalist>
          <button class="btn2" disabled={busy || !attachName.trim()}>Attach</button>
        </form>
      </div>
      {#each mem.attached.filter((s) => s.error) as s (s.namespace)}<p class="err">{s.namespace}: {s.error}</p>{/each}
    </div>

    {#if triples.length}
      <SemanticGraph facts={triples} selected={picked} onselect={(n) => (picked = n)} />
    {/if}

    {#if adding?.type === 'semantic'}{@render editor(adding, 'Add a fact')}{/if}
    {#if editing?.type === 'semantic'}{@render editor(editing, 'Edit the fact')}{/if}
    <div class="card">
      <div class="card-head"><h3>{picked ? `Facts about ${picked}` : 'Facts'}</h3><span>{factRows.length} shown</span><span class="grow"></span>
        <button class="btn2" disabled={busy} onclick={() => startAdd('semantic')}>Add a fact</button></div>
      {#if !factRows.length}
        <p class="dim">{filter || picked ? 'Nothing matches.' : 'No facts yet. Facts are written when it reflects on a conversation or is told to remember; attach a space (a3t-usage) to share one.'}</p>
      {:else}
        <div class="scroll">
          <table class="tbl facts">
            <thead><tr><th>Subject</th><th>Predicate</th><th>Object / value</th><th>Confidence</th><th>Space</th><th>From</th><th>When</th><th></th></tr></thead>
            <tbody>
              {#each factRows as e, i (editId(e) + i)}
                {@const conf = Number(e.data.confidence ?? NaN)}
                  <tr title={text(e.data.content ?? '')} class:sel={editing?.type === 'semantic' && editing.key === e.key && editing.space === (e.space ?? own)}>
                    <td class="mono"><button class="link" onclick={() => (picked = text(e.data.subject))}>{text(e.data.subject ?? '') || '—'}</button></td>
                    <td>{text(e.data.predicate ?? '') || (e.data.subject ? '—' : text(e.data.content ?? e.data).slice(0, 120))}</td>
                    <td class="mono">{#if e.data.object}<button class="link" onclick={() => (picked = text(e.data.object))}>{text(e.data.object)}</button>{:else}{text(e.data.value ?? '')}{/if}</td>
                    <td class="nowrap">{#if Number.isFinite(conf)}<span class="bar"><span style="width:{Math.round(conf * 100)}%"></span></span> {Math.round(conf * 100)}%{:else}—{/if}</td>
                    <td class="mono nowrap">{e.space === own ? 'own' : e.space}</td>
                    <td>{text(e.data.source ?? '') || '—'}{e.data.session_id ? ` · ${conv(e.data.session_id)}` : ''}</td>
                    <td class="when">{e.at ? ago(e.at) : ''}</td>
                    <td class="nowrap">{@render actions('semantic', e)}</td>
                  </tr>
              {/each}
            </tbody>
          </table>
        </div>
      {/if}
    </div>
  {:else}
    <div class="addrow"><button class="btn2" disabled={busy} onclick={() => startAdd(open)}>Add {open === 'episodic' ? 'an' : 'a'} {open} entry</button></div>
    {#if adding && adding.type === open}{@render editor(adding, `Add ${open} memory`)}{/if}
    {#if !list.length}
      <p class="dim">{filter ? 'Nothing matches.' : `Nothing in ${open} memory yet.`} {open === 'episodic' || open === 'working' ? 'Talk to it on Overview.' : 'Procedures come from merged branches and learned steps.'}</p>
    {:else if open === 'episodic'}
      <ol class="timeline">
        {#each days as g (g.day)}
          <li class="dayhead">{g.day}</li>
          {#each g.items as e, i (e.key ?? `${g.day}-${i}`)}
            {@const id = e.key ?? `${g.day}-${i}`}
            {@const body = text(e.data.content ?? e.data.reply ?? e.data.text ?? e.data)}
            <li class="ev">
              <time title={e.at ?? ''}>{time(e.at)}</time>
              <div class="what">
                {#if editing && editing.key === e.key && e.key}{@render editor(editing, 'Edit the entry')}{:else}
                <div class="meta"><span class="chip">{e.data.role === 'user' ? 'they said' : 'it answered'}</span><span class="mono">conversation {conv(e.data.session_id)}</span>
                  {#if e.data.tool_calls_made}<span>· {e.data.tool_calls_made} tool call{Number(e.data.tool_calls_made) === 1 ? '' : 's'}</span>{/if}
                  {#if e.data.iterations}<span>· {e.data.iterations} step{Number(e.data.iterations) === 1 ? '' : 's'}</span>{/if}
                  {#if e.at}<span class="dim">· {ago(e.at)}</span>{/if}<span class="grow"></span>{@render actions('episodic', e)}</div>
                <p class:clip={!expanded[id] && body.length > 280}>{body}</p>
                {#if body.length > 280}<button class="link" onclick={() => (expanded[id] = !expanded[id])}>{expanded[id] ? 'Less' : 'More'}</button>{/if}
                {/if}
              </div>
            </li>
          {/each}
        {/each}
      </ol>
    {:else if open === 'working'}
      {#each convs as [c, items] (c)}
        <div class="card">
          <div class="card-head"><h3 class="mono">conversation {c}</h3><span>{items.length} held{items[0]?.at ? ` · latest ${ago(items[0].at)}` : ''}</span></div>
          <ol class="work">
            {#each items as e, i (e.key ?? i)}
              {#if editing && editing.key === e.key && e.key}<li class="full">{@render editor(editing, 'Edit the entry')}</li>
              {:else}<li><time>{time(e.at)}</time><span class="chip">{e.data.role ?? 'entry'}</span><span>{text(e.data.content ?? e.data)}</span>{@render actions('working', e)}</li>{/if}
            {/each}
          </ol>
        </div>
      {/each}
      <p class="dim">Working memory is the conversation in hand: it expires with the conversation.</p>
    {:else}
      <ol class="procs">
        {#each list as e, i (e.key ?? i)}
          {@const steps = e.data.steps ?? e.data.actions ?? e.data.action ?? e.data.flow ?? e.data.response ?? e.data.content}
          {#if editing && editing.key === e.key && e.key}<li>{@render editor(editing, 'Edit the procedure')}</li>
          {:else}
          <li class="card">
            <div class="card-head"><h3>When: <span class="mono">{text(e.data.pattern ?? e.data.when ?? e.key ?? '—')}</span></h3><span>{e.at ? ago(e.at) : ''}{e.data.source ? ` · ${text(e.data.source)}` : ''}</span><span class="grow"></span>{@render actions('procedural', e)}</div>
            {#if Array.isArray(steps)}<ol class="steps">{#each steps as s, j (j)}<li>{text(s)}</li>{/each}</ol>
            {:else}<p>{text(steps ?? e.data)}</p>{/if}
          </li>
          {/if}
        {/each}
      </ol>
    {/if}
  {/if}
  {#if mem}<p class="dim">Its memory space: <span class="mono">{mem.namespace}</span> on the org's ADB. Versions and branches of it are in Versions &amp; deploy.</p>{/if}
</section>

<style>
  .grow { flex: 1 1 auto; }
  .types { display: grid; grid-template-columns: repeat(4, 1fr); gap: 8px; }
  .types button { display: grid; grid-template-columns: 1fr auto; gap: 2px 8px; text-align: left; padding: 10px 12px; border: 1px solid var(--border); border-radius: 9px; background: var(--bg-surface); font: inherit; cursor: pointer; }
  .types button.on { border-color: var(--spec); background: var(--spec-bg); }
  .types b { font-size: 13px; color: var(--text-1); }
  .types .n { font: 600 13px var(--font-mono); color: var(--text-1); }
  .types .dim { grid-column: 1 / 3; font-size: 11.5px; }
  @container details (max-width: 700px) { .types { grid-template-columns: repeat(2, 1fr); } }
  input[type='search'] { margin-top: 8px; }
  .spaces { display: flex; flex-wrap: wrap; gap: 6px; align-items: center; }
  .sp { display: inline-flex; align-items: center; gap: 6px; height: 28px; padding: 0 10px; border: 1px solid var(--border); border-radius: 14px; background: var(--bg-surface); font: 12.5px var(--font-sans); color: var(--text-1); cursor: pointer; }
  .sp.on { border-color: var(--spec); background: var(--spec-bg); }
  .sp.att { padding: 0 4px 0 10px; }
  .sp .n { font: 600 11.5px var(--font-mono); color: var(--text-3); }
  .plain { border: 0; background: none; padding: 0; font: inherit; color: inherit; cursor: pointer; display: inline-flex; gap: 6px; align-items: center; }
  .plain.x { width: 20px; height: 20px; justify-content: center; border-radius: 50%; color: var(--text-3); font-size: 15px; }
  .plain.x:hover { background: var(--bg-subtle); color: var(--err); }
  .attach { display: inline-flex; gap: 6px; margin-left: auto; }
  .attach input { height: 30px; width: 190px; }
  .scroll { overflow-x: auto; }
  .acts { display: inline-flex; flex-wrap: nowrap; gap: 10px; white-space: nowrap; }
  .nowrap { white-space: nowrap; }
  .facts tr.sel td { background: var(--spec-bg); }
  .link.danger { color: var(--err); }
  .edit { border-color: var(--spec); }
  .form { display: grid; grid-template-columns: repeat(4, minmax(0, 1fr)); gap: 8px; }
  .form label { display: flex; flex-direction: column; gap: 3px; font-size: 11.5px; color: var(--text-3); }
  .form .wide { grid-column: 1 / -1; }
  @container details (max-width: 700px) { .form { grid-template-columns: 1fr 1fr; } .attach { margin-left: 0; width: 100%; } .attach input { flex: 1; } }
  textarea { width: 100%; box-sizing: border-box; font: 12.5px var(--font-mono); padding: 8px; border: 1px solid var(--border); border-radius: 7px; background: var(--bg-surface); color: var(--text-1); }
  .row { display: flex; gap: 8px; margin-top: 8px; }
  .addrow { display: flex; justify-content: flex-end; }
  .timeline { list-style: none; margin: 0; padding: 0 0 0 4px; display: flex; flex-direction: column; }
  .dayhead { margin: 14px 0 6px; font: 600 11px var(--font-mono); letter-spacing: 0.06em; text-transform: uppercase; color: var(--text-4); }
  .dayhead:first-child { margin-top: 0; }
  .ev { display: grid; grid-template-columns: 54px 1fr; gap: 12px; padding: 0 0 12px; position: relative; }
  .ev time { font: 12px var(--font-mono); color: var(--text-3); padding-top: 2px; text-align: right; }
  .ev .what { border-left: 2px solid var(--border); padding: 0 0 0 14px; display: flex; flex-direction: column; gap: 4px; position: relative; }
  .ev .what::before { content: ''; position: absolute; left: -6px; top: 5px; width: 10px; height: 10px; border-radius: 50%; background: var(--bg-surface); border: 2px solid var(--spec); }
  .ev .meta { display: flex; flex-wrap: wrap; gap: 6px; align-items: baseline; font-size: 12px; color: var(--text-3); }
  .ev p { margin: 0; font-size: 13px; line-height: 1.5; color: var(--text-1); white-space: pre-wrap; }
  .ev p.clip { max-height: 4.5em; overflow: hidden; }
  .chip { font: 600 10.5px var(--font-mono); text-transform: uppercase; letter-spacing: 0.04em; padding: 1px 6px; border-radius: 4px; background: var(--bg-subtle); color: var(--text-3); }
  .work { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 6px; }
  .work li { display: grid; grid-template-columns: 48px auto 1fr auto; gap: 8px; align-items: baseline; font-size: 13px; }
  .work li.full { display: block; }
  .work time { font: 12px var(--font-mono); color: var(--text-3); }
  .facts td { vertical-align: top; font-size: 12.5px; }
  .bar { display: inline-block; width: 48px; height: 6px; border-radius: 3px; background: var(--bg-subtle); vertical-align: middle; overflow: hidden; }
  .bar span { display: block; height: 100%; background: var(--spec); }
  .when { white-space: nowrap; color: var(--text-4); }
  .procs { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 8px; }
  .steps { margin: 0; padding-left: 20px; font-size: 13px; }
  .mono { font-family: var(--font-mono); font-size: 0.92em; }
</style>
