<script lang="ts">
  /// Knowledge: the knowledge-graph databases. Items lists them; the one picked
  /// opens here in the agents' frame — Overview (what it holds, where it comes
  /// from, who uses it), Graph (the whole graph, a node's connections), Query
  /// (triples, link prediction, AQL over the facts it is built from), Build
  /// (from a semantic space, train again, add triples).
  import { untrack } from 'svelte';
  import SlidePanel from '$lib/components/shared/SlidePanel.svelte';
  import PropertiesTree from '$lib/components/shared/PropertiesTree.svelte';
  import WizardHead from '$lib/components/shared/WizardHead.svelte';
  import KGPointCloud from '$lib/components/editor/KGPointCloud.svelte';
  import '$lib/components/agent/agent.css';
  import '$lib/components/triggers/triggers.css';
  import { resources as resourcesMcp } from '$lib/services/api';
  import { getActivePanel, navigateTo, navigateToDetail } from '$lib/stores/panels.svelte.ts';
  import { getProject, namespacesFor, canWriteIn } from '$lib/stores/context.svelte.ts';
  import { bumpItems } from '$lib/stores/apps.svelte.ts';
  import { kgApi } from '$lib/components/knowledge/kgApi';
  import { usedBy, type Use } from '$lib/components/resources/resourceUse';
  import { recall, runAql, isRead, type Rec } from '$lib/components/memory/aql';

  type Kg = { ns: string; name: string; ref: string; config: Record<string, any> };
  let db = $state<Kg | null>(null);
  let tab = $state('overview');
  let stats = $state<any>(null);
  let rels = $state<{ name: string; triples: number }[]>([]);
  let uses = $state<Use[] | null>(null);
  let loadErr = $state('');
  let loading = $state(false);

  let shown = '';
  $effect(() => {
    const p = getActivePanel()?.props ?? {};
    const id = p.selectedId as string | undefined;
    const create = p.create;
    untrack(() => {
      if (create && !id) { creating = true; db = null; shown = ''; nName = ''; nErr = ''; return; }
      if (id && id !== shown) { creating = false; shown = id; void open(id); } else if (!id) { shown = ''; db = null; }
    });
  });

  // ── New knowledge graph: a LanceDB store marked kind: kg ──
  let creating = $state(false);
  let nName = $state('');
  let nNs = $state(getProject() || 'a3t');
  let nErr = $state('');
  async function createKg() {
    nErr = '';
    const name = nName.trim();
    const r = await resourcesMcp.create(name, 'lancedb', { db_path: `/data/a3t/${name}.lance`, kind: 'kg' }, nNs);
    if (!r.ok) { nErr = r.error ?? 'Not created.'; return; }
    creating = false; bumpItems();
    navigateToDetail('knowledge', 'Knowledge', `${nNs}~${name}`);
  }
  async function open(id: string) {
    const [ns, ...rest] = id.split('~'); const name = rest.join('~');
    tab = 'overview'; stats = null; rels = []; uses = null; loadErr = ''; selectedNode = null; trip = []; preds = null; aqlRes = null; buildMsg = '';
    const r = await resourcesMcp.get(name, ns);
    const config = (r.ok ? (r.data?.config ?? {}) : {}) as Record<string, any>;
    db = { ns, name, ref: `${ns}/${name}`, config };
    space = config.source_space ?? 'a3t-usage';
    aql = recall('SEMANTIC', space, [], 50);
    void loadStats();
    usedBy(db.ref).then((u) => { if (db?.ref === `${ns}/${name}`) uses = u; });
  }
  async function loadStats() {
    if (!db) return;
    loading = true; loadErr = '';
    const [s, r] = await Promise.all([kgApi.stats(db.ref), kgApi.relations(db.ref)]);
    loading = false;
    if (!s.ok) { loadErr = s.error; return; }
    stats = s.data;
    rels = (r.ok ? r.data.relations ?? [] : []).map((x: any) => ({ name: x.name, triples: x.triples ?? 0 })).sort((a, b) => b.triples - a.triples);
    for (const t of Object.keys(stats?.types ?? {})) colorFor(t);
  }
  const close = () => navigateTo('knowledge', 'Knowledge');
  const empty = $derived(!!stats && !stats.entities);

  // ── Graph ──
  const PALETTE = ['#3b82f6', '#8b5cf6', '#10b981', '#f59e0b', '#ef4444', '#ec4899', '#06b6d4', '#84cc16', '#f97316', '#14b8a6'];
  let typeColors = $state<Record<string, string>>({});
  function colorFor(t: string) { if (t && !typeColors[t]) typeColors = { ...typeColors, [t]: PALETTE[Object.keys(typeColors).length % PALETTE.length] }; return typeColors[t] ?? '#64748b'; }
  const points = $derived((stats?.points ?? []).map((p: any) => ({ id: p.id, name: p.name, type: p.type, x: p.x, y: p.y, z: p.z })));
  const edges = $derived((stats?.edges ?? []).map((e: any) => ({ source: e.source, target: e.target, relation: e.relation ?? 'related_to', opacity: 0.18 })));
  let find = $state('');
  let focusId = $state('');
  const found = $derived(find.trim() ? points.filter((p: any) => p.name.toLowerCase().includes(find.trim().toLowerCase())).slice(0, 8) : []);
  let selectedNode = $state<any>(null);
  async function select(node: any) {
    if (!db) return;
    selectedNode = { ...node, conns: null };
    const [o, i] = await Promise.all([kgApi.lookup(db.ref, { head: node.name, limit: 40 }), kgApi.lookup(db.ref, { tail: node.name, limit: 40 })]);
    const out = (o.ok ? o.data.triples ?? [] : []).map((t: any) => ({ dir: '→', rel: t.relation, name: t.tail }));
    const inc = (i.ok ? i.data.triples ?? [] : []).map((t: any) => ({ dir: '←', rel: t.relation, name: t.head }));
    selectedNode = { ...node, conns: [...out, ...inc] };
  }
  function focus(p: any) { focusId = ''; requestAnimationFrame(() => (focusId = p.id)); void select(p); find = ''; }

  // ── Query ──
  let qh = $state(''); let qr = $state(''); let qt = $state('');
  let trip = $state<any[]>([]);
  let qBusy = $state(''); let qErr = $state('');
  async function lookup() {
    if (!db) return; qBusy = 'lookup'; qErr = '';
    const r = await kgApi.lookup(db.ref, { head: qh || undefined, relation: qr || undefined, tail: qt || undefined, limit: 100 });
    qBusy = ''; if (r.ok) trip = r.data.triples ?? []; else qErr = r.error;
  }
  let ph = $state(''); let pr = $state('');
  let preds = $state<any[] | null>(null);
  async function predict() {
    if (!db || !ph || !pr) return; qBusy = 'predict'; qErr = '';
    const r = await kgApi.predictTail(db.ref, ph, pr, 10);
    qBusy = ''; if (r.ok) preds = r.data?.candidates ?? r.data?.predictions ?? r.data?.results ?? (Array.isArray(r.data) ? r.data : []); else qErr = r.error;
  }
  let space = $state('a3t-usage');
  let aql = $state('');
  let aqlRes = $state<Rec[] | null>(null);
  let aqlErr = $state('');
  async function runIt() { qBusy = 'aql'; aqlErr = ''; const r = await runAql(aql); qBusy = ''; if (r.ok) aqlRes = r.records; else { aqlRes = null; aqlErr = r.error; } }

  // ── Build ──
  let epochs = $state(300);
  let lines = $state('');
  let building = $state(''); let buildMsg = $state(''); let buildErr = $state('');
  async function build() {
    if (!db) return; building = 'semantic'; buildErr = ''; buildMsg = '';
    const r = await kgApi.fromSemantic(db.ref, space, epochs);
    building = '';
    if (!r.ok) { buildErr = r.error; return; }
    const d = r.data;
    buildMsg = `${d?.facts ?? 0} facts from ${space}: ${d?.ingest?.ingested ?? 0} new triples${d?.trained ? ` · trained (MRR ${Number(d.trained.mrr ?? 0).toFixed(3)})` : ''}.`;
    // Remember where it was built from (Overview says so; AQL starts there).
    if (db.config.source_space !== space) { const cfg = { ...db.config, source_space: space }; const u = await resourcesMcp.update(db.ns, db.name, cfg); if (u.ok) db.config = cfg; }
    await loadStats();
  }
  async function retrain() { if (!db) return; building = 'train'; buildErr = ''; const r = await kgApi.train(db.ref, epochs); building = ''; if (r.ok) { buildMsg = `Trained${r.data?.mrr != null ? ` (MRR ${Number(r.data.mrr).toFixed(3)})` : ''}.`; await loadStats(); } else buildErr = r.error; }
  const parsed = $derived(lines.split('\n').map((l) => l.split('|').map((x) => x.trim())).filter((p) => p.length === 3 && p.every(Boolean)).map(([head, relation, tail]) => ({ head, relation, tail })));
  async function add() { if (!db || !parsed.length) return; building = 'ingest'; buildErr = ''; const r = await kgApi.ingest(db.ref, parsed); building = ''; if (r.ok) { buildMsg = `${r.data?.ingested ?? parsed.length} triples added. Train again to place them.`; lines = ''; await loadStats(); } else buildErr = r.error; }

  const steps = [
    { id: 'overview', label: 'Overview', icon: '<rect x="2" y="2" width="12" height="12" rx="2"/><path d="M5 6h6M5 9h4"/>', hint: 'What it holds, where it comes from, and who uses it' },
    { id: 'graph', label: 'Graph', icon: '<circle cx="4" cy="4" r="2"/><circle cx="12" cy="5" r="2"/><circle cx="7" cy="12" r="2"/><path d="M5.7 5l4.4.4M5.2 5.6l1.3 4.6M10.9 6.6l-2.6 4"/>', hint: 'The whole graph — pick a node to see its connections' },
    { id: 'query', label: 'Query', icon: '<circle cx="7" cy="7" r="4.5"/><path d="M10.5 10.5L14 14"/>', hint: 'Find triples, predict missing links, read its source facts with AQL' },
    { id: 'build', label: 'Build', icon: '<path d="M2 14h12M4 14V8l4-5 4 5v6"/>', hint: 'Build it from a semantic space, train it again, add triples' },
  ];
  const n = (x: any) => Number(x ?? 0).toLocaleString();
</script>

<SlidePanel open={!!db || creating} onclose={close}>
  {#if creating}
    <div class="panel-layout">
      <PropertiesTree icon="+" color="#5530a8" name={nName || 'New knowledge graph'} meta="knowledge graph" nodes={[]} steps={[{ id: 'new', label: 'Knowledge graph', hint: 'A LanceDB store for entities, relations and their embeddings', icon: '<circle cx="8" cy="8" r="6"/>' }]} activeNode="new" onselect={() => {}} />
      <div class="panel-col"><div class="panel-content"><div class="pf">
        <WizardHead step={{ id: 'new', label: 'New knowledge graph', hint: 'A LanceDB store for entities, relations and their RotatE embeddings — build it from a semantic space next' }} />
        <section class="agt">
          <div class="card">
            <div class="card-head"><h3>1 · Name</h3><span>flows and agents refer to it as project/name</span></div>
            <div class="fields">
              <label class="fld">Name<input class="mono" bind:value={nName} placeholder="support-kg" /></label>
              <label class="fld">Project<select bind:value={nNs}>{#each [...new Set([nNs, ...namespacesFor('owned')].filter(Boolean))] as ns (ns)}<option value={ns}>{ns}</option>{/each}</select></label>
            </div>
            <p class="dim">Its store: <span class="mono">/data/a3t/{nName.trim() || '<name>'}.lance</span> on FlowR.</p>
            <div class="note warn">Today that store is on each FlowR pod (CA-38): a FlowR restart empties it, and each replica keeps its own copy. Build it again after a restart until that is moved.</div>
          </div>
          {#if nErr}<div class="note bad">{nErr}</div>{/if}
          <div class="acts"><button class="btn2 primary" disabled={!/^[a-z0-9][a-z0-9-]{1,62}$/.test(nName.trim())} onclick={createKg}>Create knowledge graph</button><button class="btn2 quiet" onclick={close}>Cancel</button></div>
        </section>
      </div></div></div>
    </div>
  {:else if db}
    <div class="panel-layout">
      <PropertiesTree icon="KG" color="#5530a8" name={db.name} meta={`knowledge graph · ${db.ns}`} nodes={[]} {steps} activeNode={tab} onselect={(id) => (tab = id)} />
      <div class="panel-col"><div class="panel-content"><div class="pf" class:wide-pf={tab === 'graph'}>
        <WizardHead step={steps.find((s) => s.id === tab)} />
        <section class="agt">
        {#if loadErr}<div class="note bad">{loadErr}</div>{/if}

        {#if tab === 'overview'}
          <div class="card">
            <div class="card-head"><h3>1 · What it holds</h3><span>{loading ? 'reading…' : empty ? 'nothing yet' : `${n(stats?.dim)}-dimensional RotatE embeddings`}</span></div>
            <div class="tiles">
              <div class="tile"><span class="v">{n(stats?.entities)}</span><span class="k">entities</span></div>
              <div class="tile"><span class="v">{n(stats?.relations)}</span><span class="k">relations</span></div>
              <div class="tile"><span class="v">{n(stats?.triples)}</span><span class="k">triples</span></div>
              <div class="tile"><span class="v">{n(Object.keys(stats?.types ?? {}).length)}</span><span class="k">entity types</span></div>
            </div>
            {#if empty}<div class="note warn">It is empty. Build it from a semantic space in Build.</div>{/if}
            {#if rels.length}<ul class="rows">{#each rels.slice(0, 8) as r (r.name)}<li><span class="lbl mono">{r.name}</span><span class="val">{n(r.triples)} triples</span><span></span></li>{/each}</ul>{/if}
          </div>
          <div class="card">
            <div class="card-head"><h3>2 · Where it comes from</h3><span>its store, and the facts it is built from</span></div>
            <ul class="rows">
              <li><span class="lbl">Store</span><span class="val mono">{db.ref} · {db.config.db_path ?? '—'}</span><button class="btn2" onclick={() => navigateToDetail('resources', 'Resources', `database:${db!.ns}/${db!.name}`)}>Open</button></li>
              <li><span class="lbl">Built from</span><span class="val">{db.config.source_space ? `semantic space ${db.config.source_space} on a3t/adb` : 'not recorded'}</span><button class="btn2" onclick={() => (tab = 'build')}>Build</button></li>
            </ul>
          </div>
          <div class="card">
            <div class="card-head"><h3>3 · Used by</h3><span>agents, canvases and triggers that name it</span></div>
            {#if uses === null}<p class="dim">Looking…</p>{:else if uses.length}<ul class="rows wide">{#each uses as u (u.id)}<li><span class="lbl">{u.name}</span><span class="val">{u.kind} · {u.how}</span><span></span></li>{/each}</ul>{:else}<p class="empty">Nothing names it directly.</p>{/if}
          </div>

        {:else if tab === 'graph'}
          <div class="gbar"><input class="fin grow" type="search" bind:value={find} placeholder="Find an entity" />
            <span class="dim">{n(points.length)} of {n(stats?.entities)} entities shown · {n(edges.length)} edges</span>
            <button class="btn2 quiet" onclick={loadStats}>Refresh</button></div>
          {#if found.length}<div class="chips">{#each found as p (p.id)}<button onclick={() => focus(p)}>{p.name}</button>{/each}</div>{/if}
          <div class="gframe">
            {#if points.length}<KGPointCloud {points} {edges} {typeColors} onselect={select} {focusId} />
            {:else}<p class="empty">{loading ? 'Reading the graph…' : 'Nothing to show: build it first.'}</p>{/if}
            {#if Object.keys(typeColors).length}<div class="legend">{#each Object.entries(typeColors) as [t, c] (t)}<span><i style="background:{c}"></i>{t}</span>{/each}</div>{/if}
          </div>
          {#if selectedNode}
            <div class="card">
              <div class="card-head"><h3>{selectedNode.name}</h3><span>{selectedNode.type || 'entity'}</span><span class="grow"></span><button class="btn2 quiet rm" onclick={() => (selectedNode = null)}>Close</button></div>
              {#if selectedNode.conns === null}<p class="dim">Reading its connections…</p>
              {:else if selectedNode.conns.length}<ul class="rows wide">{#each selectedNode.conns as c, i (i)}<li><span class="lbl mono">{c.dir} {c.rel}</span><span class="val">{c.name}</span><span></span></li>{/each}</ul>
              {:else}<p class="empty">No connections.</p>{/if}
            </div>
          {/if}

        {:else if tab === 'query'}
          <div class="card">
            <div class="card-head"><h3>1 · Triples</h3><span>head — relation — tail; leave any empty</span></div>
            <div class="fields three">
              <label class="fld">Head<input class="mono" bind:value={qh} placeholder="agent_a3t_assistant" /></label>
              <label class="fld">Relation<input class="mono" bind:value={qr} placeholder="runs template" list="kg-rels" /></label>
              <label class="fld">Tail<input class="mono" bind:value={qt} /></label>
            </div>
            <datalist id="kg-rels">{#each rels as r (r.name)}<option value={r.name}></option>{/each}</datalist>
            <div class="acts"><button class="btn2 primary" disabled={!!qBusy} onclick={lookup}>{qBusy === 'lookup' ? 'Finding…' : 'Find'}</button><span class="dim">{trip.length ? `${trip.length} found` : ''}</span></div>
            {#if trip.length}<ul class="rows wide">{#each trip as t, i (i)}<li><span class="lbl">{t.head}</span><span class="val"><span class="mono">{t.relation}</span> → {t.tail}</span><span></span></li>{/each}</ul>{/if}
          </div>
          <div class="card">
            <div class="card-head"><h3>2 · Predict</h3><span>the likeliest tails for a head and relation, from its embeddings</span></div>
            <div class="fields">
              <label class="fld">Head<input class="mono" bind:value={ph} /></label>
              <label class="fld">Relation<input class="mono" bind:value={pr} list="kg-rels" /></label>
            </div>
            <div class="acts"><button class="btn2 primary" disabled={!!qBusy || !ph || !pr} onclick={predict}>{qBusy === 'predict' ? 'Ranking…' : 'Predict'}</button></div>
            {#if preds}{#if preds.length}<ul class="rows wide">{#each preds as c, i (i)}<li><span class="lbl">{i + 1} · {c.entity ?? c.tail ?? c.name ?? c}</span><span class="val mono">{c.score != null ? Number(c.score).toFixed(3) : ''}</span><span></span></li>{/each}</ul>{:else}<p class="empty">No candidates.</p>{/if}{/if}
          </div>
          {#if qErr}<div class="note bad">{qErr}</div>{/if}
          <div class="card">
            <div class="card-head"><h3>3 · AQL on its source</h3><span>the facts it is built from, in ADB — RECALL only</span></div>
            <textarea class="aql" rows="3" spellcheck="false" bind:value={aql} onkeydown={(e) => { if ((e.ctrlKey || e.metaKey) && e.key === 'Enter') runIt(); }}></textarea>
            <div class="acts"><button class="btn2 primary" disabled={!!qBusy || !isRead(aql)} onclick={runIt}>{qBusy === 'aql' ? 'Running…' : 'Run'}</button>
              {#if !isRead(aql) && aql.trim()}<span class="err">Only RECALL runs here.</span>{/if}{#if aqlRes}<span class="dim">{aqlRes.length} records</span>{/if}</div>
            {#if aqlErr}<div class="note bad">{aqlErr}</div>{/if}
            {#if aqlRes?.length}<ul class="rows wide">{#each aqlRes as r, i (r.id + i)}<li><span class="lbl">{r.data.subject ?? r.key}</span><span class="val">{r.data.predicate ? `${r.data.predicate} → ${r.data.object}` : JSON.stringify(r.data).slice(0, 160)}</span><span></span></li>{/each}</ul>{/if}
          </div>

        {:else if db && !canWriteIn(db.ns)}
          <div class="note">You can explore and query this graph, but not build or add to it: that needs write access to project <b class="mono">{db.ns}</b>.</div>
        {:else}
          <div class="card">
            <div class="card-head"><h3>1 · From a semantic space</h3><span>its subject → predicate → object facts become triples; then it is trained</span></div>
            <div class="fields">
              <label class="fld">Semantic space <small>on a3t/adb</small><input class="mono" bind:value={space} list="kg-spaces" /></label>
              <label class="fld">Training epochs<input type="number" min="10" max="5000" bind:value={epochs} /></label>
            </div>
            <datalist id="kg-spaces"><option value="a3t-usage"></option><option value="a3t-docs"></option></datalist>
            <div class="acts"><button class="btn2 primary" disabled={!!building || !space.trim()} onclick={build}>{building === 'semantic' ? 'Building…' : 'Build'}</button>
              <button class="btn2" disabled={!!building} onclick={retrain}>{building === 'train' ? 'Training…' : 'Train again'}</button></div>
          </div>
          <div class="card">
            <div class="card-head"><h3>2 · Add triples</h3><span>one per line: head | relation | tail</span></div>
            <textarea class="aql" rows="4" spellcheck="false" bind:value={lines} placeholder="agent_a3t_mapper | runs template | tmpl_a3t_agent"></textarea>
            <div class="acts"><button class="btn2 primary" disabled={!!building || !parsed.length} onclick={add}>{building === 'ingest' ? 'Adding…' : `Add ${parsed.length || ''}`.trim()}</button></div>
          </div>
          {#if buildMsg}<div class="note">{buildMsg}</div>{/if}
          {#if buildErr}<div class="note bad">{buildErr}</div>{/if}
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
  /* The graph is the page: the full width of the pane. */
  .pf.wide-pf { max-width: none; }
  @container details (max-width: 560px) { .pf { padding: 16px 14px; } }
  .fields.three { grid-template-columns: 1fr 1fr 1fr; }
  @container details (max-width: 560px) { .fields.three { grid-template-columns: 1fr; } }
  .gbar { display: flex; align-items: center; gap: 10px; flex-wrap: wrap; }
  .fin { height: 34px; padding: 0 10px; border: 1px solid var(--border); border-radius: 8px; background: var(--bg-surface); font: 13px var(--font-sans); color: var(--text-1); min-width: 0; }
  .gframe { position: relative; height: max(520px, calc(100vh - 280px)); border: 1px solid var(--border); border-radius: 10px; overflow: hidden; background: #0f1115; }
  .gframe > :global(*:first-child) { width: 100%; height: 100%; }
  .gframe .empty { color: #9a9da3; padding: 24px; }
  .legend { position: absolute; left: 10px; bottom: 10px; display: flex; flex-wrap: wrap; gap: 6px 12px; padding: 6px 10px; border-radius: 8px; background: rgba(15, 17, 21, 0.75); font: 11.5px var(--font-sans); color: #d6d8dc; }
  .legend i { display: inline-block; width: 8px; height: 8px; border-radius: 50%; margin-right: 5px; }
  .aql { width: 100%; box-sizing: border-box; padding: 10px 12px; border: 1px solid var(--border); border-radius: 8px; background: var(--bg-surface); font: 12.5px/1.6 var(--font-mono); color: var(--text-1); resize: vertical; }
</style>
