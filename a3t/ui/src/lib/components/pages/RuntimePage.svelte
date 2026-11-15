<script lang="ts">
  /// Runtimes — where live systems are operated. Items is the drill-down (host →
  /// runtime → the agents and flows deployed on it); the one picked opens here in
  /// the agents' frame. Configuration lives in Canvas; this is operations:
  /// whether each thing is serving, which pool has a worker, and pause / resume /
  /// restart / undeploy / terminate. A runtime's state is its owner's to change
  /// (a shared runtime is SuperAdmin's; the server says so in `canControl`).
  import { untrack, onDestroy } from 'svelte';
  import SlidePanel from '$lib/components/shared/SlidePanel.svelte';
  import PropertiesTree from '$lib/components/shared/PropertiesTree.svelte';
  import WizardHead from '$lib/components/shared/WizardHead.svelte';
  import '$lib/components/agent/agent.css';
  import '$lib/components/triggers/triggers.css';
  import { opsApi, canvasApi, mcpCall } from '$lib/services/api';
  import { inBackground } from '$lib/core/status/activity';
  import { hasRole } from '$lib/stores/auth';
  import { getActivePanel, navigateTo, navigateToDetail } from '$lib/stores/panels.svelte.ts';
  import { namespacesFor } from '$lib/stores/context.svelte.ts';
  import { bumpItems } from '$lib/stores/apps.svelte.ts';

  interface Stats {
    runs: number; inFlight: number; failed: number; refused: number; completed: number;
    runsLastHour: number; errorRate: number; p50Ms?: number; p95Ms?: number;
    lastRunAt?: string; lastError?: string;
  }
  interface Deployment {
    id: string; kind: 'flow' | 'agent'; target: string; flowRef: string; agentRef: string;
    canvas: string; runtime: string; namespace: string; mode: string; state: 'active' | 'paused';
    pausedBy?: string; pausedAt?: string; pauseReason?: string; deployedBy: string; deployedAt: string;
  }
  interface Row { deployment: Deployment; serving: string; callersGetWhenPaused: number; stats: Stats }
  interface Host {
    host: string; note: string;
    memory: { usedBytes?: number; limitBytes?: number };
    node?: { name?: string; memTotalBytes?: number | null; memAvailableBytes?: number | null; cpus?: number | null; load1?: number | null };
    seenAt?: string;
    cpu: { coresUsed?: number | null; limitCores?: number };
    disk?: { path: string; totalBytes: number; freeBytes: number; usedBytes: number };
  }
  interface Rt {
    runtime: { name: string; namespace: string; labels: Record<string, string>; url?: string | null; default?: boolean };
    state: 'active' | 'hibernated' | 'paused'; pausedResponse: number; deployments: Row[]; canControl?: boolean;
  }
  interface Pool { pool: string; live: boolean; served_here: boolean; queued: number }

  const isAdmin = hasRole('admin');
  const controls = (rt: Rt) => rt.canControl ?? isAdmin;
  function ownerOf(rt: Rt): string {
    const l = rt.runtime.labels ?? {};
    return l['a3t.io/org'] ? `org ${l['a3t.io/org']}` : l['a3t.io/project'] ? `project ${l['a3t.io/project']}` : 'shared by every project';
  }

  // ── What is picked in Items ──
  let sel = $state('');
  let view = $derived<'host' | 'runtime' | 'deployment' | null>(
    sel.startsWith('dep:') ? 'deployment' : sel.startsWith('rt:') ? 'runtime' : sel.startsWith('host:') ? 'host' : null);
  let key = $derived(sel.replace(/^(host|rt|dep):/, ''));
  let tab = $state('overview');
  let creating = $state(false);
  let shown = '';
  $effect(() => {
    const p = getActivePanel()?.props ?? {};
    const id = (p.selectedId as string | undefined) ?? '';
    untrack(() => {
      if (p.create && !id) { creating = true; sel = ''; shown = ''; resetNew(); return; }
      if (id && id !== shown) { creating = false; shown = id; sel = id; tab = 'overview'; resetActs(); void load(); }
      else if (!id) { shown = ''; sel = ''; }
    });
  });
  const close = () => navigateTo('runtime', 'Runtimes');
  const open = (id: string) => navigateToDetail('runtime', 'Runtimes', id);

  // ── Data ──
  let runtimes = $state<Rt[]>([]);
  let unplaced = $state<Row[]>([]);
  let host = $state<Host | null>(null);
  let hostCount = $state(1);
  let controller = $state<'none' | 'reachable' | 'unreachable'>('none');
  let pools = $state<Pool[]>([]);
  let probe = $state<any>(null);
  let loaded = $state(false);
  let err = $state('');
  let busy = $state('');
  let done = $state<{ ok: boolean; text: string } | null>(null);

  async function load() {
    void mcpCall('list_pools', {}).then((r) => { pools = r.ok && Array.isArray(r.data?.pools) ? r.data.pools : []; });
    const res = await opsApi.overview();
    loaded = true;
    if (!res.ok) { err = res.error || 'Runtimes could not be read.'; return; }
    err = '';
    runtimes = res.data?.runtimes ?? [];
    unplaced = res.data?.unplaced ?? [];
    // The FlowR picked in Items (every one reports itself), else the one that answered.
    const all: Host[] = res.data?.hosts ?? [];
    host = all.find((h) => h.host === key) ?? res.data?.host ?? null;
    hostCount = all.length || 1;
    controller = res.data?.controller ?? 'none';
    if (view === 'deployment') { const p = await opsApi.probe(key); probe = p.ok ? p.data : null; }
  }
  // Live while something is open: CPU needs two samples, so the second read fills it in.
  const timer = setInterval(() => { if (sel && !document.hidden && !busy && !confirming) void inBackground(load); }, 5000);
  onDestroy(() => clearInterval(timer));

  const allRows = $derived([...runtimes.flatMap((r) => r.deployments), ...unplaced]);
  const rt = $derived<Rt | null>(view === 'runtime'
    ? key === '__unplaced'
      ? ({ runtime: { name: 'Unplaced', namespace: '', labels: {} }, state: 'active', pausedResponse: 503, deployments: unplaced } as Rt)
      : runtimes.find((r) => r.runtime.name === key) ?? null
    : null);
  const row = $derived(view === 'deployment' ? allRows.find((r) => r.deployment.id === key) ?? null : null);
  const poolOf = (name: string) => pools.find((p) => p.pool === name);
  const notServing = (rows: Row[]) => rows.filter((d) => d.serving !== 'serving' && d.serving !== 'hibernated').length;
  const sum = (rows: Row[], f: (s: Stats) => number) => rows.reduce((n, d) => n + f(d.stats), 0);

  // ── Actions ──
  /// The action waiting for its inline confirmation: `pause` takes a reason,
  /// `terminate-rt` the runtime's name typed in.
  let confirming = $state<'' | 'pause' | 'undeploy' | 'terminate' | 'pause-rt' | 'terminate-rt'>('');
  let reason = $state('');
  let typed = $state('');
  let plan = $state<any>(null);
  let planErr = $state('');
  function resetActs() { confirming = ''; reason = ''; typed = ''; plan = null; planErr = ''; done = null; probe = null; }
  function flash(ok: boolean, text: string) { done = { ok, text }; }

  async function act(r: Row, action: 'pause' | 'resume' | 'restart' | 'undeploy' | 'terminate') {
    busy = action; done = null;
    const res = await opsApi.deployment(r.deployment.id, action, action === 'pause' ? reason.trim() || undefined : undefined);
    busy = ''; confirming = ''; reason = '';
    flash(res.ok, res.ok ? res.data?.note || `${action} done` : res.status === 403 ? `Not allowed: ${res.error}` : res.error || 'It failed.');
    bumpItems();
    if (res.ok && (action === 'undeploy' || action === 'terminate')) { open(`rt:${r.deployment.runtime}`); return; }
    await load();
  }
  async function rtAct(r: Rt, action: 'hibernate' | 'pause' | 'resume') {
    busy = action; done = null;
    const res = await opsApi.runtime(r.runtime.name, action);
    busy = ''; confirming = '';
    flash(res.ok, res.ok ? res.data?.note || 'Done.' : res.status === 403 ? 'Only its owner may change this runtime’s state.' : res.error || 'It failed.');
    bumpItems();
    await load();
  }
  async function setPausedResponse(r: Rt, status: 503 | 404) {
    const res = await opsApi.pausedResponse(r.runtime.name, status);
    if (!res.ok) flash(false, res.error || 'It failed.');
    await load();
  }
  async function planTerminate(r: Rt) {
    confirming = 'terminate-rt'; typed = ''; plan = null; planErr = '';
    const res = await opsApi.terminatePlan(r.runtime.name);
    if (res.ok) plan = res.data?.impact ?? null; else planErr = res.error || 'The plan could not be worked out.';
  }
  async function terminateRt(r: Rt) {
    if (typed !== r.runtime.name) return;
    busy = 'terminate';
    const res = await opsApi.terminate(r.runtime.name);
    busy = ''; confirming = '';
    flash(res.ok, res.ok ? res.data?.note || `Runtime ${r.runtime.name} terminated.` : res.status === 403 ? res.error || 'You may not control this runtime.' : res.error || 'It failed.');
    bumpItems();
    if (res.ok && !res.data?.recreated) { close(); return; }
    await load();
  }
  function editCanvas(d: Deployment) {
    const name = d.canvas.split('/')[1];
    if (name) navigateToDetail('canvas', 'Canvas', name);
  }

  // ── New runtime ──
  let nName = $state(''), nNs = $state('default'), nLabels = $state(''), nUrl = $state(''), nDefault = $state(false), nErr = $state('');
  function resetNew() { nName = ''; nNs = 'default'; nLabels = ''; nUrl = ''; nDefault = false; nErr = ''; }
  async function createRuntime() {
    const name = nName.trim();
    if (!/^[A-Za-z0-9_-]+$/.test(name)) { nErr = "Name: letters, digits, '-' or '_'."; return; }
    const labels: Record<string, string> = {};
    for (const part of nLabels.split(',').map((x) => x.trim()).filter(Boolean)) {
      const [k, v] = part.split('=').map((x) => x?.trim());
      if (!k || !v) { nErr = `Label “${part}” must be key=value.`; return; }
      labels[k] = v;
    }
    busy = 'create';
    const res = await canvasApi.createRuntime(nNs.trim() || 'default', name, { labels, url: nUrl.trim() || undefined, default: nDefault });
    busy = '';
    if (!res.ok) { nErr = res.error || 'The runtime was not created.'; return; }
    creating = false; bumpItems(); open(`rt:${name}`);
  }

  // ── Formatting ──
  const SERVING: Record<string, { label: string; dot: string; hint: string }> = {
    serving: { label: 'Serving', dot: 'ok', hint: 'taking requests' },
    paused: { label: 'Paused', dot: 'err', hint: 'refuses every request until resumed' },
    'runtime-paused': { label: 'Runtime paused', dot: 'err', hint: 'its runtime is paused' },
    hibernated: { label: 'Hibernated', dot: '', hint: 'asleep — the next request wakes it' },
    unplaced: { label: 'Unplaced', dot: 'warn', hint: 'its runtime no longer exists' },
  };
  const RT_DOT: Record<string, string> = { active: 'ok', hibernated: '', paused: 'err' };
  function bytes(n?: number) {
    if (n == null) return '—';
    const u = ['B', 'KB', 'MB', 'GB', 'TB']; let i = 0; let v = n;
    while (v >= 1024 && i < u.length - 1) { v /= 1024; i++; }
    return `${v.toFixed(v >= 100 || i === 0 ? 0 : 1)} ${u[i]}`;
  }
  const pct = (a?: number | null, b?: number) => (a != null && b ? Math.min(100, (a / b) * 100) : null);
  function ago(iso?: string) {
    if (!iso) return '—';
    const s = Math.max(0, (Date.now() - new Date(iso).getTime()) / 1000);
    return s < 60 ? `${Math.round(s)}s ago` : s < 3600 ? `${Math.round(s / 60)}m ago` : s < 86400 ? `${Math.round(s / 3600)}h ago` : `${Math.round(s / 86400)}d ago`;
  }
  const msf = (v?: number) => (v == null ? '—' : v < 1000 ? `${v} ms` : `${(v / 1000).toFixed(1)} s`);
  const labelsOf = (l: Record<string, string>) => Object.entries(l ?? {}).map(([k, v]) => `${k}=${v}`).join(' · ') || 'none';

  const I = {
    overview: '<rect x="2" y="2" width="12" height="12" rx="2"/><path d="M5 6h6M5 9h4"/>',
    pools: '<rect x="2" y="3" width="12" height="4" rx="1"/><rect x="2" y="9" width="12" height="4" rx="1"/><circle cx="5" cy="5" r=".6"/><circle cx="5" cy="11" r=".6"/>',
    deployed: '<rect x="2" y="2" width="5" height="5" rx="1"/><rect x="9" y="2" width="5" height="5" rx="1"/><rect x="2" y="9" width="5" height="5" rx="1"/><rect x="9" y="9" width="5" height="5" rx="1"/>',
    controls: '<circle cx="8" cy="8" r="6"/><path d="M6.5 5.5v5M9.5 5.5v5"/>',
    health: '<path d="M1.5 8h3l2-4 3 8 2-4h3"/>',
    runs: '<path d="M4 3l9 5-9 5z"/>',
  };
  const steps = $derived(
    view === 'host' ? [
      { id: 'overview', label: 'Overview', icon: I.overview, hint: 'What this host has, and the runtimes on it' },
      { id: 'pools', label: 'Work pools', icon: I.pools, hint: 'Which pools have a worker claiming runs, and what is queued on each' },
    ] : view === 'runtime' ? [
      { id: 'overview', label: 'Overview', icon: I.overview, hint: 'Its state, who owns it, and its worker' },
      { id: 'deployed', label: 'Deployed', icon: I.deployed, hint: 'The agents and flows placed on it' },
      ...(key === '__unplaced' ? [] : [{ id: 'controls', label: 'Controls', icon: I.controls, hint: 'Hibernate, pause, resume or terminate it, and what callers get while it is paused' }]),
    ] : [
      { id: 'overview', label: 'Overview', icon: I.overview, hint: 'Whether it is serving, its figures, and where it runs' },
      { id: 'health', label: 'Health', icon: I.health, hint: 'Its checks, and the triggers that feed it' },
      { id: 'runs', label: 'Runs', icon: I.runs, hint: 'Its latest runs — open one in Executions' },
      { id: 'controls', label: 'Controls', icon: I.controls, hint: 'Pause, resume, restart, undeploy or terminate it' },
    ]);
  const title = $derived(view === 'host' ? key : view === 'runtime' ? (rt?.runtime.name ?? key) : (row?.deployment.target ?? key));
  const meta = $derived(view === 'host' ? 'host · shared by every runtime on it'
    : view === 'runtime' ? (rt ? `runtime · ${rt.state}${rt.runtime.namespace ? ` · ${rt.runtime.namespace}` : ''}` : 'runtime')
    : row ? `${row.deployment.kind} · ${SERVING[row.serving]?.label.toLowerCase() ?? row.serving} · on ${row.deployment.runtime}` : 'deployment');
</script>

<SlidePanel open={!!sel || creating} onclose={close}>
  {#if creating}
    <div class="panel-layout">
      <PropertiesTree icon="+" color="#0f766e" name={nName || 'New runtime'} meta="runtime" nodes={[]} steps={[{ id: 'new', label: 'Runtime', hint: 'A pool canvases place work into', icon: I.pools }]} activeNode="new" onselect={() => {}} />
      <div class="panel-col"><div class="panel-content"><div class="pf">
        <WizardHead step={{ id: 'new', label: 'New runtime', hint: 'A pool canvases place work into — its labels are what placement selects on' }} />
        <section class="agt">
          <div class="card">
            <div class="card-head"><h3>1 · Name</h3><span>canvases and the run store call its pool by this name</span></div>
            <div class="fields">
              <label class="fld">Name<input class="mono" bind:value={nName} placeholder="rt-staging" /></label>
              <label class="fld">Project<select bind:value={nNs}>{#each [...new Set([nNs, ...namespacesFor('owned')].filter(Boolean))] as ns (ns)}<option value={ns}>{ns}</option>{/each}</select></label>
            </div>
          </div>
          <div class="card">
            <div class="card-head"><h3>2 · Placement</h3><span>what a canvas selects it by</span></div>
            <div class="fields">
              <label class="fld">Labels<input class="mono" bind:value={nLabels} placeholder="tier=test, class=cpu" /></label>
              <label class="fld">URL <small>optional</small><input class="mono" bind:value={nUrl} placeholder="http://runtime-gpu.a3t.svc:3002" /></label>
            </div>
            <label class="chk"><input type="checkbox" bind:checked={nDefault} /> The default runtime — used when a canvas selects no labels</label>
          </div>
          {#if nErr}<div class="note bad">{nErr}</div>{/if}
          <div class="acts"><button class="btn2 primary" disabled={!nName.trim() || !!busy} onclick={createRuntime}>{busy ? 'Creating…' : 'Create runtime'}</button><button class="btn2 quiet" onclick={close}>Cancel</button></div>
        </section>
      </div></div></div>
    </div>
  {:else if view}
    <div class="panel-layout">
      <PropertiesTree icon={view === 'host' ? 'H' : view === 'runtime' ? 'R' : row?.deployment.kind === 'agent' ? 'A' : 'F'}
        color={view === 'deployment' ? (row?.deployment.kind === 'agent' ? '#b45309' : '#5530a8') : view === 'host' ? '#3a5a85' : '#0f766e'}
        name={title} {meta} nodes={[]} {steps} activeNode={tab} onselect={(id) => { tab = id; confirming = ''; }} />
      <div class="panel-col"><div class="panel-content"><div class="pf">
        <WizardHead step={steps.find((s) => s.id === tab)} />
        <section class="agt">
        {#if err}<div class="note bad">{err}</div>
        {:else if !loaded}<p class="dim">Reading…</p>
        {:else}
          {#if done}<div class="note" class:bad={!done.ok}>{done.text}</div>{/if}

          <!-- ═══ Host ═══ -->
          {#if view === 'host'}
            {#if tab === 'overview'}
              {#if host}
                {@const cpu = pct(host.cpu.coresUsed, host.cpu.limitCores)}
                {@const mem = pct(host.memory.usedBytes, host.memory.limitBytes)}
                {@const disk = host.disk ? pct(host.disk.usedBytes, host.disk.totalBytes) : null}
                {@const nodeUsed = host.node?.memTotalBytes != null && host.node?.memAvailableBytes != null ? host.node.memTotalBytes - host.node.memAvailableBytes : undefined}
                {@const nodeMem = pct(nodeUsed, host.node?.memTotalBytes ?? undefined)}
                <div class="card">
                  <div class="card-head"><h3>1 · Resources</h3><span>{hostCount > 1 ? `one of ${hostCount} FlowRs` : 'the FlowR'}{host.node?.name ? ` on node ${host.node.name}` : ''}</span></div>
                  <ul class="rows meters">
                    {#if host.node?.memTotalBytes}<li><span class="lbl">Node memory</span><span class="val"><span class="bar"><i style="width:{nodeMem ?? 0}%" class:hot={(nodeMem ?? 0) > 85}></i></span>{bytes(nodeUsed)} of {bytes(host.node.memTotalBytes)} used on the node{host.node.cpus ? ` · ${host.node.cpus} CPUs, load ${host.node.load1?.toFixed(2) ?? '?'}` : ''}</span><span></span></li>{/if}
                    <li><span class="lbl">This FlowR, CPU</span><span class="val"><span class="bar"><i style="width:{cpu ?? 0}%" class:hot={(cpu ?? 0) > 80}></i></span>{host.cpu.coresUsed != null ? host.cpu.coresUsed.toFixed(2) : '…'} of {host.cpu.limitCores ?? '—'} cores it may use</span><span></span></li>
                    <li><span class="lbl">This FlowR, memory</span><span class="val"><span class="bar"><i style="width:{mem ?? 0}%" class:hot={(mem ?? 0) > 80}></i></span>{bytes(host.memory.usedBytes)} of {bytes(host.memory.limitBytes)} it may use (its pod's limit)</span><span></span></li>
                    {#if host.disk}<li><span class="lbl">Disk</span><span class="val"><span class="bar"><i style="width:{disk ?? 0}%" class:hot={(disk ?? 0) > 85}></i></span>{bytes(host.disk.usedBytes)} of {bytes(host.disk.totalBytes)} · {bytes(host.disk.freeBytes)} free · <span class="mono">{host.disk.path}</span></span><span></span></li>{/if}
                  </ul>
                </div>
              {/if}
              <div class="card">
                <div class="card-head"><h3>2 · Runtimes on it</h3><span>{runtimes.length} registered · {allRows.length} deployed · {sum(allRows, (s) => s.inFlight)} in flight</span>
                  {#if isAdmin}<span class="grow"></span><button class="btn2 rm" onclick={() => navigateTo('runtime', 'Runtimes', { create: Date.now() })}>New runtime</button>{/if}</div>
                {#if runtimes.length}
                  <ul class="rows">
                    {#each runtimes as r (r.runtime.name)}
                      {@const p = poolOf(r.runtime.name)}
                      <li><span class="lbl"><span class="dot {RT_DOT[r.state]}"></span> <span class="mono">{r.runtime.name}</span></span>
                        <span class="val">{r.state} · {r.deployments.length} deployed · {labelsOf(r.runtime.labels)}{#if p && !p.live} · <span class="err">no worker on its pool</span>{/if}{#if notServing(r.deployments)} · <span class="err">{notServing(r.deployments)} not serving</span>{/if}</span>
                        <button class="btn2" onclick={() => open(`rt:${r.runtime.name}`)}>Open</button></li>
                    {/each}
                  </ul>
                {:else}<p class="empty">No runtimes are registered yet.</p>{/if}
              </div>
              <div class="card">
                <div class="card-head"><h3>3 · Controller</h3><span>what turns a placement into pods</span></div>
                <p class="dim">{controller === 'reachable' ? 'The EE controller answers: each runtime gets its own pods, and it rolls them on restart.' : controller === 'unreachable' ? 'The EE controller is configured but does not answer — placement is logical until it does.' : 'No controller: this is CE, so every runtime runs inside this one FlowR process and placement is logical.'}</p>
              </div>

            {:else}
              <div class="card">
                <div class="card-head"><h3>1 · Work pools</h3><span>runs wait in a pool until a worker claims them</span><span class="grow"></span><button class="btn2 quiet rm" onclick={load}>Refresh</button></div>
                {#if pools.length}
                  <ul class="rows">
                    {#each pools as p (p.pool)}
                      <li><span class="lbl"><span class="dot {p.live ? 'ok' : p.queued ? 'err' : ''}"></span> <span class="mono">{p.pool}</span></span>
                        <span class="val">{p.live ? `a worker is claiming${p.served_here ? ' (this FlowR)' : ''}` : 'no worker'} · {p.queued} queued</span>
                        {#if runtimes.some((r) => r.runtime.name === p.pool)}<button class="btn2" onclick={() => open(`rt:${p.pool}`)}>Open</button>{:else}<span></span>{/if}</li>
                    {/each}
                  </ul>
                {:else}<p class="empty">No pools answered.</p>{/if}
                {#if pools.some((p) => !p.live && p.queued)}<div class="note bad">Work is queued on a pool no worker claims — it waits until one does.</div>{/if}
              </div>
              <div class="card">
                <div class="card-head"><h3>2 · Where pools come from</h3></div>
                <ul class="notes">
                  <li><span class="mono">default</span> — the FlowR servers' own pool: anything not placed elsewhere runs here.</li>
                  <li>A registered runtime's pool has its name; it needs a worker (the EE controller's runtime pod) to claim it.</li>
                  <li>Other live pools are workers that joined through the engine gateway without a registered runtime.</li>
                </ul>
              </div>
            {/if}

          <!-- ═══ Runtime ═══ -->
          {:else if view === 'runtime'}
            {#if !rt}<div class="note warn">Runtime “{key}” is not registered any more.</div>
            {:else if tab === 'overview'}
              {@const p = poolOf(rt.runtime.name)}
              {#if key === '__unplaced'}<div class="note warn">Deployments whose runtime was removed after they were placed. Restart re-places one from its canvas; undeploy removes it.</div>{/if}
              <div class="card">
                <div class="card-head"><h3>1 · State</h3><span>{rt.state === 'hibernated' ? 'asleep — the first request wakes it' : rt.state === 'paused' ? 'paused — everything on it refuses requests until resumed' : 'serving'}</span></div>
                <div class="tiles">
                  <button class="tile" onclick={() => (tab = 'deployed')}><span class="v">{rt.deployments.length}</span><span class="k">deployed</span></button>
                  <div class="tile"><span class="v">{sum(rt.deployments, (s) => s.inFlight)}</span><span class="k">in flight</span></div>
                  <div class="tile"><span class="v">{sum(rt.deployments, (s) => s.runsLastHour)}</span><span class="k">runs in the last hour</span></div>
                  <div class="tile" class:bad={notServing(rt.deployments) > 0}><span class="v">{notServing(rt.deployments)}</span><span class="k">not serving</span></div>
                </div>
              </div>
              {#if key !== '__unplaced'}
                <div class="card">
                  <div class="card-head"><h3>2 · What it is</h3></div>
                  <ul class="rows">
                    <li><span class="lbl">Owner</span><span class="val">{ownerOf(rt)}{controls(rt) ? ' · you may control it' : ''}</span><span></span></li>
                    <li><span class="lbl">Project</span><span class="val mono">{rt.runtime.namespace}</span><span></span></li>
                    <li><span class="lbl">Labels</span><span class="val mono">{labelsOf(rt.runtime.labels)}</span><span></span></li>
                    <li><span class="lbl">Default</span><span class="val">{rt.runtime.default ? 'yes — canvases that select no labels land here' : 'no'}</span><span></span></li>
                    {#if rt.runtime.url}<li><span class="lbl">URL</span><span class="val mono">{rt.runtime.url}</span><span></span></li>{/if}
                  </ul>
                </div>
                <div class="card">
                  <div class="card-head"><h3>3 · Worker</h3><span>pool <span class="mono">{rt.runtime.name}</span></span></div>
                  {#if !p}<p class="dim">The pool did not answer.</p>
                  {:else if p.live}<p class="ok">A worker is claiming its runs{p.served_here ? ' (this FlowR)' : ''} · {p.queued} queued.</p>
                  {:else}<div class="note warn">No worker claims this pool{p.queued ? ` and ${p.queued} run${p.queued === 1 ? ' waits' : 's wait'}` : ''}. {controller === 'reachable' ? 'The controller has not started a pod for it.' : 'On CE, runs placed here are served by FlowR itself.'}</div>{/if}
                </div>
              {/if}

            {:else if tab === 'deployed'}
              <div class="card">
                <div class="card-head"><h3>1 · Deployed here</h3><span>{rt.deployments.length} · each serving, paused or hibernated</span></div>
                {#if rt.deployments.length}
                  <ul class="rows wide">
                    {#each rt.deployments as r (r.deployment.id)}
                      {@const sv = SERVING[r.serving] ?? SERVING.serving}
                      <li><span class="lbl"><span class="dot {sv.dot}"></span> <span class="mono">{r.deployment.target}</span></span>
                        <span class="val">{r.deployment.kind} · {sv.label} · {r.stats.runsLastHour} runs/h · {r.stats.inFlight} in flight</span>
                        <button class="btn2" onclick={() => open(`dep:${r.deployment.id}`)}>Open</button></li>
                    {/each}
                  </ul>
                {:else}<p class="empty">Nothing is deployed here. Deploy from a canvas.</p>
                  <div class="acts"><button class="btn2" onclick={() => navigateTo('canvas', 'Canvas')}>Open Canvas</button></div>{/if}
              </div>

            {:else}
              {#if !controls(rt)}<div class="note">{ownerOf(rt).startsWith('shared') ? 'A shared runtime is controlled by SuperAdmin.' : "It is controlled by its org's admins."} You can see it, not change it.</div>{/if}
              <div class="card">
                <div class="card-head"><h3>1 · State</h3><span>now {rt.state}</span></div>
                <ul class="rows wide">
                  {#if rt.state !== 'active'}<li><span class="lbl">Resume</span><span class="val">everything on it serves again</span><button class="btn2 primary" disabled={!controls(rt) || !!busy} onclick={() => rtAct(rt!, 'resume')}>Resume</button></li>{/if}
                  {#if rt.state === 'active'}<li><span class="lbl">Hibernate</span><span class="val">sleep until the next request wakes it</span><button class="btn2" disabled={!controls(rt) || !!busy} onclick={() => rtAct(rt!, 'hibernate')}>Hibernate</button></li>{/if}
                  {#if rt.state !== 'paused'}<li><span class="lbl">Pause</span><span class="val">all {rt.deployments.length} deployed here refuse requests until resumed</span>
                    {#if confirming === 'pause-rt'}<span class="acts"><button class="btn2 danger" disabled={!!busy} onclick={() => rtAct(rt!, 'pause')}>Pause it</button><button class="btn2 quiet" onclick={() => (confirming = '')}>Cancel</button></span>
                    {:else}<button class="btn2" disabled={!controls(rt) || !!busy} onclick={() => (confirming = 'pause-rt')}>Pause</button>{/if}</li>{/if}
                </ul>
              </div>
              <div class="card">
                <div class="card-head"><h3>2 · While paused</h3><span>what a trigger caller gets — a canvas can override it</span></div>
                <div class="seg" role="tablist">
                  <button role="tab" class:on={rt.pausedResponse === 503} disabled={!controls(rt)} onclick={() => setPausedResponse(rt!, 503)}>503 Unavailable</button>
                  <button role="tab" class:on={rt.pausedResponse === 404} disabled={!controls(rt)} onclick={() => setPausedResponse(rt!, 404)}>404 Not found</button>
                </div>
              </div>
              <div class="card">
                <div class="card-head"><h3>3 · Terminate</h3><span>like killing its container: runs on it are cancelled; what is deployed is rescheduled</span>
                  {#if confirming !== 'terminate-rt'}<span class="grow"></span><button class="btn2 rm" disabled={!controls(rt) || !!busy} onclick={() => planTerminate(rt!)}>Plan it</button>{/if}</div>
                {#if confirming === 'terminate-rt'}
                  {#if planErr}<div class="note bad">{planErr}</div>
                  {:else if !plan}<p class="dim">Working out the plan…</p>
                  {:else}
                    {#if plan.warnings?.length}<div class="note warn"><ul class="notes">{#each plan.warnings as w}<li>{w}</li>{/each}</ul></div>{/if}
                    {#if !plan.plan?.length}<p class="dim">Nothing is deployed here — the runtime is simply removed.</p>
                    {:else}
                      <ul class="rows wide">
                        {#each plan.plan as p}
                          <li><span class="lbl mono">{p.target}</span><span class="val">{p.kind} · {p.mode} → {p.action === 'move' ? `moves to ${p.to}` : `fresh ${p.to}`}{p.runsInFlight ? ` · ${p.runsInFlight} run(s) cancelled` : ''}{p.note ? ` · ${p.note}` : ''}</span><span></span></li>
                        {/each}
                      </ul>
                      {#if plan.recreatesRuntime}<p class="dim">The runtime is recreated fresh (same name, labels and project) because something can only run there.</p>{/if}
                    {/if}
                    <div class="fields"><label class="fld">Type <b class="mono">{rt.runtime.name}</b> to confirm<input class="mono" bind:value={typed} placeholder={rt.runtime.name} /></label></div>
                    <div class="acts"><button class="btn2 danger" disabled={typed !== rt.runtime.name || !!busy} onclick={() => terminateRt(rt!)}>Terminate</button><button class="btn2 quiet" onclick={() => (confirming = '')}>Cancel</button></div>
                  {/if}
                {/if}
              </div>
            {/if}

          <!-- ═══ Deployment ═══ -->
          {:else}
            {#if !row}<div class="note warn">“{key}” is not deployed any more.</div>
            {:else}
              {@const d = row.deployment}
              {@const sv = SERVING[row.serving] ?? SERVING.serving}
              {#if d.state === 'paused'}<div class="note warn">Paused {ago(d.pausedAt)}{d.pausedBy ? ` by ${d.pausedBy}` : ''}{d.pauseReason ? ` — “${d.pauseReason}”` : ''}. Trigger callers get HTTP {row.callersGetWhenPaused}; every other request is refused until it is resumed.</div>{/if}
              {#if tab === 'overview'}
                <div class="card">
                  <div class="card-head"><h3>1 · {sv.label}</h3><span>{sv.hint}</span></div>
                  <div class="tiles">
                    <div class="tile"><span class="v">{row.stats.runsLastHour}</span><span class="k">runs in the last hour</span></div>
                    <div class="tile"><span class="v">{row.stats.inFlight}</span><span class="k">in flight</span></div>
                    <div class="tile" class:bad={row.stats.failed > 0}><span class="v">{Math.round(row.stats.errorRate * 100)}%</span><span class="k">failed</span></div>
                    <div class="tile"><span class="v">{msf(row.stats.p95Ms)}</span><span class="k">p95 · p50 {msf(row.stats.p50Ms)}</span></div>
                  </div>
                  {#if row.stats.lastError}<div class="note bad">Last error: {row.stats.lastError}</div>{/if}
                </div>
                <div class="card">
                  <div class="card-head"><h3>2 · Where it runs</h3></div>
                  <ul class="rows">
                    <li><span class="lbl">Runtime</span><span class="val mono">{d.runtime}</span><button class="btn2" onclick={() => open(`rt:${d.runtime}`)}>Open</button></li>
                    <li><span class="lbl">Mode</span><span class="val">{d.mode}</span><span></span></li>
                    <li><span class="lbl">Canvas</span><span class="val mono">{d.canvas || 'the default canvas'}</span>{#if d.canvas}<button class="btn2" onclick={() => editCanvas(d)}>Open</button>{:else}<span></span>{/if}</li>
                    {#if d.kind === 'agent'}<li><span class="lbl">Runs flow</span><span class="val mono">{d.flowRef}</span><span></span></li>{/if}
                    <li><span class="lbl">Deployed</span><span class="val">{ago(d.deployedAt)}{d.deployedBy ? ` by ${d.deployedBy}` : ''}</span><span></span></li>
                    <li><span class="lbl">Refused</span><span class="val">{row.stats.refused} request{row.stats.refused === 1 ? '' : 's'} refused by policy or pause</span><span></span></li>
                  </ul>
                </div>
              {:else if tab === 'health'}
                <div class="card">
                  <div class="card-head"><h3>1 · Checks</h3><span>{probe ? `${probe.checks.filter((c: any) => c.ok).length} of ${probe.checks.length} pass` : 'probing…'}</span><span class="grow"></span><button class="btn2 quiet rm" onclick={load}>Probe again</button></div>
                  {#if probe}
                    <ul class="rows wide">
                      {#each probe.checks as c}
                        <li><span class="lbl"><span class="dot {c.ok ? 'ok' : 'err'}"></span> {c.check}</span><span class="val">{#if c.target}<span class="mono">{c.target}</span>{/if}{#if c.detail && !c.ok} — <span class="err">{c.detail}</span>{/if}</span><span></span></li>
                      {/each}
                    </ul>
                  {/if}
                </div>
                <div class="card">
                  <div class="card-head"><h3>2 · Triggers</h3><span>what starts it</span></div>
                  {#if probe?.triggers?.length}
                    <ul class="rows">
                      {#each probe.triggers as t}
                        <li><span class="lbl"><span class="dot {t.enabled ? 'ok' : ''}"></span> {t.name}</span><span class="val">{t.config?.type} · <span class="mono">{t.config?.path ?? t.config?.expression ?? t.config?.source ?? ''}</span></span><button class="btn2" onclick={() => navigateToDetail('triggers', 'Triggers', t.id ?? t.name)}>Open</button></li>
                      {/each}
                    </ul>
                  {:else}<p class="empty">No trigger feeds it — it runs on demand, from chat or from other agents.</p>{/if}
                </div>
              {:else if tab === 'runs'}
                <div class="card">
                  <div class="card-head"><h3>1 · Latest runs</h3><span>{probe?.recentRuns?.length ?? 0}</span><span class="grow"></span><button class="btn2 quiet rm" onclick={() => navigateTo('executions', 'Executions')}>All executions</button></div>
                  {#if probe?.recentRuns?.length}
                    <ul class="tl">
                      {#each probe.recentRuns as r (r.instanceId)}
                        {@const refused = (r.error ?? '').startsWith('refused: ')}
                        <li class="ev" class:err={!!r.error && !refused}>
                          <span class="ic">{r.error ? (refused ? '–' : '✕') : '✓'}</span>
                          <button class="evb" onclick={() => navigateToDetail('executions', 'Executions', r.instanceId)}>
                            <div class="t mono">{r.instanceId}</div>
                            <div class="m">{msf(r.durationMs)}{r.error ? ` · ${refused ? r.error.slice(9) : r.error}` : ''}</div>
                          </button>
                          <time>{ago(r.startedAt)}</time>
                        </li>
                      {/each}
                    </ul>
                  {:else}<p class="empty">No runs yet.</p>{/if}
                </div>
              {:else}
                <div class="card">
                  <div class="card-head"><h3>1 · Serving</h3><span>now {sv.label.toLowerCase()}</span></div>
                  <ul class="rows wide">
                    {#if d.state === 'paused'}
                      <li><span class="lbl">Resume</span><span class="val">it takes requests again</span><button class="btn2 primary" disabled={!!busy} onclick={() => act(row!, 'resume')}>Resume</button></li>
                    {:else}
                      <li><span class="lbl">Pause</span><span class="val">every new request is refused — triggers, manual runs, chat, other agents; runs in progress finish. Trigger callers get HTTP {row.callersGetWhenPaused}.</span>
                        {#if confirming !== 'pause'}<button class="btn2" disabled={!!busy} onclick={() => { confirming = 'pause'; reason = ''; }}>Pause</button>{:else}<span></span>{/if}</li>
                    {/if}
                    <li><span class="lbl">Restart</span><span class="val">place it again from the current canvas</span><button class="btn2" disabled={!!busy || d.state === 'paused'} title={d.state === 'paused' ? 'Resume before restarting' : ''} onclick={() => act(row!, 'restart')}>Restart</button></li>
                  </ul>
                  {#if confirming === 'pause'}
                    <div class="fields"><label class="fld">Reason <small>shown to whoever finds it paused</small><input bind:value={reason} placeholder="investigating bad outputs" /></label></div>
                    <div class="acts"><button class="btn2 danger" disabled={!!busy} onclick={() => act(row!, 'pause')}>Pause it</button><button class="btn2 quiet" onclick={() => (confirming = '')}>Cancel</button></div>
                  {/if}
                </div>
                <div class="card">
                  <div class="card-head"><h3>2 · Remove</h3><span>the flow and its canvas are kept</span></div>
                  <ul class="rows wide">
                    <li><span class="lbl">Undeploy</span><span class="val">runs in progress finish, then it is removed</span>
                      {#if confirming === 'undeploy'}<span class="acts"><button class="btn2 danger" disabled={!!busy} onclick={() => act(row!, 'undeploy')}>Undeploy</button><button class="btn2 quiet" onclick={() => (confirming = '')}>Cancel</button></span>
                      {:else}<button class="btn2" disabled={!!busy} onclick={() => (confirming = 'undeploy')}>Undeploy</button>{/if}</li>
                    <li><span class="lbl">Terminate</span><span class="val">cancel the {row.stats.inFlight} run{row.stats.inFlight === 1 ? '' : 's'} in progress now, then undeploy</span>
                      {#if confirming === 'terminate'}<span class="acts"><button class="btn2 danger" disabled={!!busy} onclick={() => act(row!, 'terminate')}>Terminate</button><button class="btn2 quiet" onclick={() => (confirming = '')}>Cancel</button></span>
                      {:else}<button class="btn2" disabled={!!busy} onclick={() => (confirming = 'terminate')}>Terminate</button>{/if}</li>
                  </ul>
                </div>
              {/if}
            {/if}
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
  .meters .val { display: flex; align-items: center; gap: 10px; flex-wrap: wrap; }
  .bar { position: relative; display: inline-block; width: 140px; height: 6px; border-radius: 3px; background: var(--bg-subtle); overflow: hidden; }
  .bar i { position: absolute; inset: 0 auto 0 0; background: var(--runtime, #0f766e); border-radius: 3px; }
  .bar i.hot { background: var(--err, #b42318); }
  .chk { display: flex; align-items: center; gap: 8px; font-size: 12.5px; color: var(--text-2); }
  .evb { all: unset; display: block; cursor: pointer; min-width: 0; }
</style>
