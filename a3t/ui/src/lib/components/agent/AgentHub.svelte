<script lang="ts">
  /// The agent tabs that read it — the same screens for everyone:
  ///   overview  what it is, is it working, where it came from; chat with it,
  ///             or make another from it (clone, fork, branch to dev)
  ///   activity  what it did: conversations, runs, what was blocked, memory
  ///   deploy    where it runs, one-click deploy / pause / undeploy, its health
  /// Configure, Branches and Settings are AgentGuide. Data comes from the APIs
  /// that exist; every gap is named, not faked (.ai/A3T_UI_LIMITATIONS.md).
  import { untrack, type Snippet } from 'svelte';
  import { navigateTo } from '$lib/stores/panels.svelte.ts';
  import { opsApi } from '$lib/services/api';
  import {
    factsOf, servingRuns, decisionsFor, conversationsWith, placementOf, lineageOf, forksOf, ago, dur,
    type AgentRec, type TemplateRec, type Run, type Decision, type Placement,
  } from './agentData';
  import type { ChatSessionSummary } from '$lib/services/api';

  let { agent, templates, allAgents = [], section, ongo, onstudio, onlifecycle, notDeployed, rev = 0, startFilter }: {
    agent: AgentRec & { description?: string };
    templates: TemplateRec[];
    allAgents?: AgentRec[];
    section: 'overview' | 'activity' | 'deploy';
    /// Show another tab of the agent.
    ongo: (tab: string) => void;
    onstudio?: () => void;
    /// Make another agent from it, or branch it.
    onlifecycle?: (kind: 'clone' | 'fork' | 'branch') => void;
    /// What Deploy shows when it is not deployed (the page's deploy form).
    notDeployed?: Snippet;
    /// Bumped by the page after it changes the deployment: reload.
    rev?: number;
    /// Activity's view to open on (a deep link to its memory).
    startFilter?: 'conv' | 'run' | 'deny' | 'memory';
  } = $props();

  const f = $derived(factsOf(agent, templates));
  const lineage = $derived(lineageOf(agent, allAgents));
  const forks = $derived(forksOf(agent, allAgents));
  const purpose = $derived(agent.description || f.purpose);

  // ── Loaded once per agent, shared by every section ──
  let runs = $state<Run[]>([]);
  let runsErr = $state('');
  let decisions = $state<Decision[]>([]);
  let decErr = $state('');
  let scanned = $state(0);
  let convs = $state<ChatSessionSummary[]>([]);
  let placement = $state<Placement | null>(null);
  let placeErr = $state('');
  let loading = $state(true);
  let loadedFor = '';

  async function load() {
    loading = true;
    const facts = factsOf(agent, templates);
    const [r, d, c, p] = await Promise.all([servingRuns(facts), decisionsFor(agent), conversationsWith(agent).catch(() => []), placementOf(agent, facts)]);
    runs = r.runs; runsErr = r.error;
    decisions = d.list; decErr = d.error; scanned = d.scanned;
    convs = c;
    placement = p.placement; placeErr = p.error;
    loading = false;
  }
  $effect(() => {
    const key = `${agent.id}|${templates.length}|${rev}`;
    if (key === loadedFor) return;
    loadedFor = key;
    untrack(() => void load());
  });

  const failed = $derived(runs.filter((r) => ['failed', 'error', 'cancelled'].includes(r.status)));
  const denied = $derived(decisions.filter((d) => !d.allow));
  /// Its serving flow's runs are shared with every agent on that flow (UI-2), so
  /// they say something about this agent only once it is deployed on its own.
  const ownRuns = $derived(!!placement);
  const lastSeen = $derived.by(() => {
    const ts = [...(ownRuns ? runs.map((r) => Date.parse(r.started)) : []), ...convs.map((c) => Date.parse(c.updated_at)), ...decisions.map((d) => d.ts)].filter((x) => x > 0);
    return ts.length ? Math.max(...ts) : 0;
  });
  const health = $derived.by((): { tone: 'ok' | 'warn' | 'err' | 'idle'; word: string } => {
    if (loading) return { tone: 'idle', word: 'Checking…' };
    if (!placement) return convs.length || decisions.length ? { tone: 'ok', word: 'In use' } : { tone: 'idle', word: 'Not used yet' };
    const er = placement.stats?.errorRate;
    if (placement.deploymentState === 'paused') return { tone: 'warn', word: 'Paused' };
    if (er != null && er > 0.2) return { tone: 'err', word: 'Failing' };
    if (er != null && er > 0.02) return { tone: 'warn', word: 'Some failures' };
    return { tone: 'ok', word: 'Working' };
  });
  /// Where it runs, in words.
  const where = $derived(loading ? '…' : placement
    ? placement.deploymentState === 'paused' ? `Paused on ${placement.runtime || 'its runtime'}` : `Deployed on ${placement.runtime || 'a runtime being chosen'}`
    : 'Not deployed — it answers through the shared runtime');

  // ── Activity: one timeline; Memory is one of its views ──
  type Item = { kind: 'conv' | 'run' | 'deny' | 'allow'; ts: number; title: string; meta: string; tone?: string };
  // svelte-ignore state_referenced_locally — where it opens; the chips take over
  let filter = $state<'all' | 'conv' | 'run' | 'deny' | 'memory'>(startFilter ?? 'all');
  const timeline = $derived.by((): Item[] => {
    const items: Item[] = [
      ...convs.map((c) => ({ kind: 'conv' as const, ts: Date.parse(c.updated_at), title: c.title || 'Conversation', meta: `${c.message_count} messages` })),
      ...runs.map((r) => ({ kind: 'run' as const, ts: Date.parse(r.started), title: `Run ${r.status}`, meta: `${r.flow} · ${dur(r.ms)}${r.error ? ` · ${r.error.slice(0, 120)}` : ''}`, tone: failed.includes(r) ? 'err' : r.status === 'completed' ? 'ok' : '' })),
      ...decisions.map((d) => ({ kind: d.allow ? 'allow' as const : 'deny' as const, ts: d.ts, title: `${d.allow ? 'Allowed' : 'Blocked'}: ${d.action}`, meta: `${d.resource}${d.obo ? ` · on behalf of ${d.obo}` : ''}${d.reason ? ` · ${d.reason}` : ''}`, tone: d.allow ? '' : 'err' })),
    ];
    return items.filter((i) => filter === 'all' || i.kind === filter).sort((a, b) => b.ts - a.ts).slice(0, 200);
  });
  const MEM = [
    { k: 'working', t: 'Working', d: 'What it holds during one conversation — the task, recent turns.' },
    { k: 'episodic', t: 'Episodic', d: 'What happened: past conversations and runs.' },
    { k: 'semantic', t: 'Semantic', d: 'What it knows: facts and summaries, searched by meaning.' },
    { k: 'procedural', t: 'Procedural', d: 'How to do things: steps it has learned.' },
  ];
  const remembers = $derived(agent.mode === 'persistent');

  function chat() {
    try { localStorage.setItem('a3t.chat.agent', JSON.stringify(agent.id)); } catch { /* private window */ }
    navigateTo('chat', 'AI Chat');
  }

  // ── Deploy: one click each; details in Runtimes ──
  let busy = $state('');
  let opMsg = $state('');
  let confirmUndeploy = $state(false);
  async function op(action: 'pause' | 'resume' | 'undeploy') {
    if (!placement?.id) return;
    busy = action; opMsg = '';
    const r = await opsApi.deployment(placement.id, action, action === 'pause' ? `Paused from the agent ${agent.name}` : undefined);
    busy = ''; confirmUndeploy = false;
    if (!r.ok) { opMsg = r.error ?? `Could not ${action} it`; return; }
    await load();
  }
</script>

{#if section === 'overview'}
  <section class="hub">
    <header class="hero">
      <p class="purpose" class:none={!purpose}>{purpose || 'No description yet — add one in Settings.'}</p>
      <div class="status">
        <span class="dot {health.tone}"></span><b>{health.word}</b>
        <span class="sep">·</span><span>{where}</span>
        {#if lastSeen}<span class="sep">·</span><span>last active {ago(lastSeen)}</span>{/if}
      </div>
      <div class="lineage">
        {#if lineage.kind === 'template'}
          Built from {#if f.template}<b class="mono">{f.template.name} v{f.template.version}</b>{:else}<span class="mono">{agent.template_id ?? 'no template'}</span>{/if}
        {:else}
          {lineage.kind === 'fork' ? 'Forked from' : 'Cloned from'} <b>{lineage.from?.name}</b>{#if f.template} · template <span class="mono">{f.template.name} v{f.template.version}</span>{/if}
        {/if}
        {#if forks.length}<span class="sep">·</span><button class="lnk" onclick={() => ongo('branches')}>{forks.length} fork{forks.length === 1 ? '' : 's'}</button>{/if}
      </div>
      <div class="acts">
        <button class="btn spec sm" onclick={chat}>Chat with it</button>
        {#if onstudio}<button class="btn ghost sm" onclick={onstudio}>Test in Studio</button>{/if}
      </div>
    </header>

    <div class="tiles">
      <button class="tile" onclick={() => { filter = 'conv'; ongo('activity'); }}><span class="v">{loading ? '…' : convs.length}</span><span class="k">Your conversations</span></button>
      <button class="tile" onclick={() => { filter = 'run'; ongo('activity'); }}><span class="v">{loading ? '…' : runs.length}</span><span class="k">{ownRuns ? 'Recent runs' : 'Runs of its flow'}</span></button>
      <button class="tile" class:bad={failed.length > 0} onclick={() => ongo('deploy')}><span class="v">{loading ? '…' : failed.length}</span><span class="k">{ownRuns ? 'Failed' : 'Failed, its flow'}</span></button>
      <button class="tile" class:bad={denied.length > 0} onclick={() => { filter = 'deny'; ongo('activity'); }}><span class="v">{loading ? '…' : decErr ? '—' : denied.length}</span><span class="k">Blocked actions</span></button>
    </div>

    <div class="glance">
      <div class="glance-head"><h4>How it is set up</h4><button class="lnk" onclick={() => ongo('configure')}>Change →</button></div>
      <dl class="facts">
        <dt>Model</dt><dd class="mono">{f.llm || '—'}</dd>
        <dt>Tools</dt><dd>{f.mcp.length ? f.mcp.map((m) => m.split('/').pop()).join(', ') : 'none'}</dd>
        <dt>Memory</dt><dd>{remembers ? 'remembers across conversations' : agent.mode === 'session' ? 'starts fresh each conversation' : agent.mode}</dd>
        <dt>Called at</dt><dd class="mono">{f.endpoint ? `POST /webhook${f.endpoint}` : '—'}</dd>
        <dt>Project</dt><dd class="mono">{agent.namespace}</dd>
      </dl>
    </div>

    {#if onlifecycle}
      <h4>Make another from it</h4>
      <div class="make">
        <button onclick={() => onlifecycle('clone')}><b>Clone</b><span>A new agent, the same to start with — on its own from then on.</span></button>
        <button onclick={() => onlifecycle('fork')}><b>Fork</b><span>A new agent that keeps its link to this one.</span></button>
        <button onclick={() => onlifecycle('branch')}><b>Branch to dev</b><span>Change and test this agent without touching prod; merge when ready.</span></button>
      </div>
    {/if}
  </section>

{:else if section === 'activity'}
  <section class="hub">
    <div class="chips">
      {#each [['all', 'All'], ['conv', `Conversations (${convs.length})`], ['run', `Runs (${runs.length})`], ['deny', `Blocked (${denied.length})`], ['memory', 'Memory']] as [k, label] (k)}
        <button class:on={filter === k} onclick={() => (filter = k as any)}>{label}</button>
      {/each}
      <span class="grow"></span>
      <button class="lnk" onclick={() => navigateTo('logs', 'Decision log', { q: agent.id })}>Decision log →</button>
      <button class="lnk" onclick={() => navigateTo('executions', 'Executions')}>Executions →</button>
    </div>
    {#if filter === 'memory'}
      <p class="line"><b>{remembers ? 'It remembers across conversations.' : 'It starts fresh each conversation.'}</b>
        <button class="lnk" onclick={() => ongo('configure')}>Change →</button></p>
      <div class="banner">What it has remembered can't be read here yet (UI-1). Below is what each kind of memory holds.</div>
      <div class="mem">
        {#each MEM as m (m.k)}
          <div class="mcard" class:off={!remembers && m.k !== 'working'}><b>{m.t}</b><span>{m.d}</span><em>{!remembers && m.k !== 'working' ? 'not kept' : 'entries: — (UI-1)'}</em></div>
        {/each}
      </div>
    {:else if loading}
      <!-- the header bar shows loading -->
    {:else}
      <ol class="tl">
        {#each timeline as it}
          <li class="ev {it.kind} {it.tone ?? ''}">
            <span class="ic" aria-hidden="true">{it.kind === 'conv' ? '💬' : it.kind === 'run' ? '▶' : it.kind === 'deny' ? '⛔' : '✓'}</span>
            <div class="body"><div class="t">{it.title}</div><div class="m">{it.meta}</div></div>
            <time>{ago(it.ts)}</time>
          </li>
        {:else}
          <li class="empty">Nothing yet{filter !== 'all' ? ' of this kind' : ''}. <button class="lnk" onclick={chat}>Chat with it</button> to start.</li>
        {/each}
      </ol>
      <ul class="notes">
        <li>Conversations are yours only; others' need an API (UI-4).</li>
        <li>Runs are those of its serving flow <span class="mono">{f.target || '—'}</span>, which other agents may share (UI-2).</li>
        <li>{decErr ? decErr : `Decisions are matched in the latest ${scanned} records of the log (UI-3).`}</li>
      </ul>
    {/if}
  </section>

{:else if section === 'deploy'}
  <section class="hub">
    <div class="dstate" class:live={!!placement && placement.deploymentState !== 'paused'} class:paused={placement?.deploymentState === 'paused'}>
      <div class="dword">
        <span class="dot {placement ? (placement.deploymentState === 'paused' ? 'warn' : 'ok') : 'idle'}"></span>
        <b>{loading ? 'Checking…' : placement ? (placement.deploymentState === 'paused' ? 'Paused' : 'Deployed') : 'Not deployed'}</b>
      </div>
      {#if !loading}
        <p>{#if placement}
            {placement.deploymentState === 'paused' ? 'It is not taking new work.' : 'It answers on its own runtime.'}
            Runtime <span class="mono">{placement.runtime || '—'}</span>{placement.canvas ? ` · canvas ${placement.canvas}` : ''}.
          {:else}
            It still answers on its endpoint, through the shared runtime. Deploy it to give it a runtime of its own.
          {/if}</p>
        {#if placement}
          <div class="acts">
            {#if placement.deploymentState === 'paused'}
              <button class="hb-btn primary" disabled={!!busy} onclick={() => op('resume')}>{busy === 'resume' ? 'Resuming…' : 'Resume'}</button>
            {:else}
              <button class="hb-btn" disabled={!!busy} onclick={() => op('pause')}>{busy === 'pause' ? 'Pausing…' : 'Pause'}</button>
            {/if}
            <button class="hb-btn" disabled={!!busy} onclick={() => (confirmUndeploy = true)}>Undeploy</button>
          </div>
          {#if confirmUndeploy}
            <div class="confirm">Undeploy it? Its runtime stops; it goes back to answering through the shared runtime.
              <button class="hb-btn danger" disabled={!!busy} onclick={() => op('undeploy')}>{busy === 'undeploy' ? 'Undeploying…' : 'Undeploy'}</button>
              <button class="lnk" onclick={() => (confirmUndeploy = false)}>Cancel</button></div>
          {/if}
          {#if opMsg}<p class="errline">{opMsg}</p>{/if}
        {:else if notDeployed}
          {@render notDeployed()}
        {/if}
      {/if}
      {#if placeErr}<p class="dim">{placeErr}</p>{/if}
    </div>

    <h4>How it is running</h4>
    {#if placement?.stats}
      {@const s = placement.stats}
      <div class="tiles">
        <div class="tile"><span class="v">{s.runsLastHour}</span><span class="k">Runs, last hour</span></div>
        <div class="tile" class:bad={s.errorRate > 0.02}><span class="v">{(s.errorRate * 100).toFixed(1)}%</span><span class="k">Errors</span></div>
        <div class="tile"><span class="v">{dur(s.p95Ms)}</span><span class="k">Slowest 5% take</span></div>
        <div class="tile" class:bad={s.refused > 0}><span class="v">{s.refused}</span><span class="k">Turned away</span></div>
      </div>
      {#if s.lastError}<div class="banner err"><b>Last error</b> {s.lastError}</div>{/if}
    {:else if !loading}
      <p class="line dim">{runs.length} recent runs of its serving flow · {failed.length} failed{runs.length ? ` · usually ${dur([...runs.map((r) => r.ms ?? 0)].sort((a, b) => a - b)[Math.floor(runs.length / 2)])}` : ''}.</p>
    {/if}
    {#if failed.length}
      <ol class="tl">
        {#each failed.slice(0, 5) as r (r.id)}
          <li class="ev run err"><span class="ic">▶</span><div class="body"><div class="t">{r.status} · {r.flow}</div><div class="m">{r.error || 'no error message'}</div></div><time>{ago(r.started)}</time></li>
        {/each}
      </ol>
    {:else if runsErr}<p class="dim">{runsErr}</p>{/if}
    <div class="deeper">
      <span>Placement, scaling, restarts and logs:</span>
      <button class="lnk" onclick={() => navigateTo('runtime', 'Runtimes')}>Runtimes →</button>
      {#if onstudio}<button class="lnk" onclick={onstudio}>Debug in Studio →</button>{/if}
    </div>
  </section>
{/if}

<style>
  .hub { display: flex; flex-direction: column; gap: 18px; }
  h4 { margin: 8px 0 -6px; font-size: 11px; font-weight: 600; text-transform: uppercase; letter-spacing: 0.06em; color: var(--text-4); }
  .mono { font-family: var(--font-mono); font-size: 12.5px; }
  .dim { color: var(--text-4); font-size: 12.5px; }
  .grow { flex: 1; }
  .lnk { border: 0; background: none; padding: 0; color: var(--spec); cursor: pointer; font-size: 12.5px; }
  .lnk:hover { text-decoration: underline; }

  .hero { padding: 18px 20px; border: 1px solid var(--border); border-radius: 10px; background: var(--bg-surface); display: flex; flex-direction: column; gap: 10px; }
  .purpose { margin: 0; font-size: 15px; line-height: 1.5; color: var(--text-1); }
  .status { display: flex; flex-wrap: wrap; align-items: center; gap: 6px; font-size: 12.5px; color: var(--text-3); }
  .status b { color: var(--text-1); font-weight: 600; }
  .sep { color: var(--text-5, var(--text-4)); }
  .dot { width: 8px; height: 8px; border-radius: 50%; background: var(--text-4); }
  .dot.ok { background: var(--ok); } .dot.warn { background: var(--warn); } .dot.err { background: var(--err); }
  .acts { display: flex; gap: 8px; margin-top: 2px; flex-wrap: wrap; }
  .purpose.none { color: var(--text-4); font-size: 14px; }
  .lineage { font-size: 12.5px; color: var(--text-3); display: flex; flex-wrap: wrap; gap: 6px; align-items: baseline; }
  .lineage b { color: var(--text-1); font-weight: 500; }
  .dot.idle { background: var(--text-4); }
  .glance { display: flex; flex-direction: column; gap: 12px; padding: 14px 18px; border: 1px solid var(--border); border-radius: 10px; background: var(--bg-surface); }
  .glance-head { display: flex; justify-content: space-between; align-items: baseline; }
  .glance-head h4 { margin: 0; }
  .glance .facts { grid-template-columns: 110px 1fr; }
  .make { display: grid; grid-template-columns: repeat(3, 1fr); gap: 10px; }
  .make button { display: flex; flex-direction: column; gap: 4px; padding: 12px 14px; border: 1px solid var(--border); border-radius: 10px; background: var(--bg-surface); text-align: left; cursor: pointer; font: inherit; }
  .make button:hover { border-color: var(--spec); background: var(--spec-bg); }
  .make b { font-size: 13.5px; color: var(--text-1); font-weight: 600; }
  .make span { font-size: 12px; color: var(--text-3); line-height: 1.45; }
  .dstate { display: flex; flex-direction: column; gap: 10px; padding: 16px 18px; border: 1px solid var(--border); border-radius: 10px; background: var(--bg-surface); }
  .dstate.live { border-color: color-mix(in srgb, var(--ok) 40%, var(--border)); }
  .dstate.paused { border-color: color-mix(in srgb, var(--warn) 45%, var(--border)); }
  .dstate p { margin: 0; font-size: 13px; color: var(--text-2); line-height: 1.5; }
  .dword { display: flex; align-items: center; gap: 8px; }
  .dword b { font: 400 19px var(--font-serif); color: var(--text-1); }
  .confirm { display: flex; flex-wrap: wrap; align-items: center; gap: 10px; padding: 10px 12px; border-radius: 8px; background: var(--bg-subtle); font-size: 12.5px; color: var(--text-2); }
  .hb-btn { height: 30px; padding: 0 12px; border-radius: 8px; border: 1px solid var(--border); background: var(--bg-surface); font: 500 12.5px var(--font-sans); color: var(--text-1); cursor: pointer; }
  .hb-btn:hover:not(:disabled) { border-color: var(--spec); }
  .hb-btn.primary { background: var(--green, #1a7158); border-color: var(--green, #1a7158); color: #fff; }
  .hb-btn.danger { background: var(--err); border-color: var(--err); color: #fff; }
  .hb-btn:disabled { opacity: 0.5; cursor: default; }
  .errline { color: var(--err) !important; font-size: 12.5px !important; }
  .deeper { display: flex; flex-wrap: wrap; gap: 12px; align-items: baseline; font-size: 12.5px; color: var(--text-3); }
  .mcard.off { opacity: 0.55; }

  .tiles { display: grid; grid-template-columns: repeat(4, 1fr); gap: 10px; }
  .tile { display: flex; flex-direction: column; gap: 4px; padding: 12px 14px; border: 1px solid var(--border); border-radius: 8px; background: var(--bg-surface); text-align: left; font: inherit; color: inherit; }
  button.tile { cursor: pointer; }
  button.tile:hover { border-color: var(--spec); }
  .tile .v { font-size: 22px; font-weight: 600; font-variant-numeric: tabular-nums; color: var(--text-1); }
  .tile .k { font-size: 11.5px; color: var(--text-3); }
  .tile.bad .v { color: var(--err); }

  .facts { display: grid; grid-template-columns: 150px 1fr; gap: 9px 16px; margin: 0; font-size: 13px; }
  .facts dt { color: var(--text-4); }
  .facts dd { margin: 0; color: var(--text-1); overflow-wrap: anywhere; }


  .chips { display: flex; align-items: center; gap: 6px; flex-wrap: wrap; }
  .chips > button:not(.lnk) { border: 1px solid var(--border); background: var(--bg-surface); border-radius: 999px; padding: 5px 12px; font-size: 12px; color: var(--text-2); cursor: pointer; }
  .chips > button.on { background: var(--text-1); color: var(--bg-page); border-color: var(--text-1); }
  .chips .lnk { margin-left: 10px; }

  .tl { list-style: none; margin: 0; padding: 0; border: 1px solid var(--border); border-radius: 8px; background: var(--bg-surface); }
  .ev { display: grid; grid-template-columns: 26px 1fr auto; gap: 10px; align-items: start; padding: 10px 14px; border-top: 1px solid var(--border); }
  .ev:first-child { border-top: 0; }
  .ev .ic { font-size: 13px; text-align: center; line-height: 20px; }
  .ev .t { font-size: 13px; color: var(--text-1); }
  .ev .m { font-size: 12px; color: var(--text-3); margin-top: 2px; overflow-wrap: anywhere; }
  .ev time { font-size: 11.5px; color: var(--text-4); white-space: nowrap; }
  .ev.err .t { color: var(--err); }
  .empty { padding: 18px 14px; font-size: 12.5px; color: var(--text-4); }

  .notes { margin: 0; padding-left: 18px; font-size: 11.5px; color: var(--text-4); display: flex; flex-direction: column; gap: 3px; }
  .line { margin: 0; font-size: 13px; color: var(--text-2); display: flex; flex-wrap: wrap; gap: 10px; align-items: baseline; }

  .banner { padding: 12px 14px; border-radius: 8px; background: var(--bg-subtle); border: 1px solid var(--border); font-size: 12.5px; color: var(--text-2); line-height: 1.5; }
  .banner.err { background: var(--err-bg); border-color: color-mix(in srgb, var(--err) 30%, transparent); }
  .mem { display: grid; grid-template-columns: repeat(auto-fill, minmax(200px, 1fr)); gap: 10px; }
  .mcard { display: flex; flex-direction: column; gap: 6px; padding: 12px 14px; border: 1px solid var(--border); border-radius: 8px; background: var(--bg-surface); }
  .mcard b { font-size: 13px; }
  .mcard span { font-size: 12px; color: var(--text-3); line-height: 1.45; }
  .mcard em { font-style: normal; font-size: 11px; color: var(--text-4); font-family: var(--font-mono); }

</style>
