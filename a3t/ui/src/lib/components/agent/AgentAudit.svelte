<script lang="ts">
  /// Audit — what it did and what changed: every decision about it (allowed
  /// and blocked), its calls and errors, test chats, memory, and the history of
  /// changes to it. Deep debugging is the Studio's.
  import { navigateTo } from '$lib/stores/panels.svelte.ts';
  import { factsOf, ago, dur, type AgentRec, type TemplateRec } from './agentData';
  import type { AgentLive } from './agentLive.svelte';

  let { agent, templates, live, start, onstudio, ongo }: {
    agent: AgentRec & { created_by?: string; created_at?: string };
    templates: TemplateRec[];
    live: AgentLive;
    /// The view to open on (a deep link to its memory).
    start?: 'memory';
    onstudio: () => void;
    ongo: (tab: string) => void;
  } = $props();

  const f = $derived(factsOf(agent, templates));
  type View = 'all' | 'deny' | 'allow' | 'run' | 'err' | 'conv' | 'memory';
  import { insight, type MemoryType } from './controlPlane';
  let mem = $state<{ namespace: string; counts: Record<MemoryType, number>; memory: Record<MemoryType, any[]> } | null>(null);
  let memErr = $state('');
  let memOpen = $state<MemoryType>('working');
  async function loadMemory(id: string) {
    mem = null; memErr = '';
    const r = await insight.memory(id);
    if (r.ok) mem = r.data; else memErr = r.error ?? 'Memory could not be read.';
  }
  const memText = (e: any) => {
    const t = e?.content ?? e?.text ?? e?.reply ?? e?.message ?? e?.pattern ?? e?.object ?? e;
    const s = typeof t === 'string' ? t : JSON.stringify(t);
    return s.length > 240 ? s.slice(0, 240) + '…' : s;
  };
  // svelte-ignore state_referenced_locally — where it opens; the chips take over
  let view = $state<View>(start ?? 'all');
  $effect(() => { if (view === 'memory') loadMemory(agent.id); });
  type Item = { kind: 'conv' | 'run' | 'deny' | 'allow'; ts: number; title: string; meta: string; err?: boolean; ic: string };
  const items = $derived.by((): Item[] => {
    const all: Item[] = [
      ...live.runs.map((r) => ({ kind: 'run' as const, ic: 'RUN', ts: Date.parse(r.started), title: `${r.status} · ${dur(r.ms)}`, meta: `${r.flow} · ${r.id}${r.error ? ` · ${r.error.slice(0, 160)}` : ''}`, err: live.failed.includes(r) })),
      ...live.convs.map((c) => ({ kind: 'conv' as const, ic: 'TEST', ts: Date.parse(c.updated_at), title: c.title || 'Test chat', meta: `${c.message_count} messages · ${c.session_id}` })),
      ...live.decisions.map((d) => ({ kind: d.allow ? 'allow' as const : 'deny' as const, ic: d.allow ? 'OK' : 'DENY', ts: d.ts, title: `${d.allow ? 'allowed' : 'blocked'}: ${d.action}`, meta: `${d.resource}${d.obo ? ` · on behalf of ${d.obo}` : ''}${d.reason ? ` · ${d.reason}` : ''}`, err: !d.allow })),
    ];
    const keep = (i: Item) => view === 'all' || (view === 'err' ? i.kind === 'run' && i.err : i.kind === view);
    return all.filter(keep).sort((a, b) => b.ts - a.ts).slice(0, 200);
  });
  const MEM = [
    { k: 'working', t: 'Working', d: 'The current conversation: task, recent turns.' },
    { k: 'episodic', t: 'Episodic', d: 'Past conversations and runs, as events.' },
    { k: 'semantic', t: 'Semantic', d: 'Facts and summaries, searched by meaning.' },
    { k: 'procedural', t: 'Procedural', d: 'Steps and preferences it has learned.' },
  ];
  const keeps = $derived(agent.mode === 'persistent');
</script>

<section class="agt">
  {#if live.placement?.stats}
    {@const s = live.placement.stats}
    <div class="tiles six">
      <div class="tile"><span class="v">{s.runsLastHour}</span><span class="k">Calls, last hour</span></div>
      <div class="tile"><span class="v">{s.runs}</span><span class="k">Calls, total</span></div>
      <div class="tile" class:bad={s.errorRate > 0.02}><span class="v">{(s.errorRate * 100).toFixed(1)}%</span><span class="k">Errors</span></div>
      <div class="tile"><span class="v">{dur(s.p50Ms)}</span><span class="k">p50</span></div>
      <div class="tile"><span class="v">{dur(s.p95Ms)}</span><span class="k">p95</span></div>
      <div class="tile" class:bad={s.refused > 0}><span class="v">{s.refused}</span><span class="k">Refused</span></div>
    </div>
    {#if s.lastError}<div class="note bad"><b>Last error</b> {s.lastError}</div>{/if}
  {:else if !live.loading}
    <div class="note">Not deployed, so it has no figures of its own. Below are the runs of its serving flow <span class="mono">{f.target || '—'}</span>, which other agents share (UI-2). <button class="link" onclick={() => ongo('releases')}>Deploy it →</button></div>
  {/if}

  <div class="chips">
    {#each [['all', 'All'], ['deny', `Blocked (${live.denied.length})`], ['allow', `Allowed (${live.decisions.length - live.denied.length})`], ['run', `Calls (${live.runs.length})`], ['err', `Errors (${live.failed.length})`], ['conv', `Test chats (${live.convs.length})`], ['memory', 'Memory']] as [k, label] (k)}
      <button class:on={view === k} onclick={() => (view = k as View)}>{label}</button>
    {/each}
    <span class="grow"></span>
    <button class="link" onclick={onstudio}>Debug in Studio →</button>
    <button class="link" onclick={() => navigateTo('executions', 'Executions')}>Executions →</button>
    <button class="link" onclick={() => navigateTo('logs', 'Decision log', { q: agent.id })}>Decision log →</button>
  </div>

  {#if view === 'memory'}
    <p class="dim"><b>memory: {agent.mode}</b> — {keeps ? 'it keeps what happened and what it learned across conversations.' : 'it keeps only the conversation it is in.'} <button class="link" onclick={() => ongo('config')}>Change in Config →</button></p>
    {#if memErr}<p class="err">{memErr}</p>{/if}
    <div class="mem">
      {#each MEM as m (m.k)}
        <button class="card" class:off={!keeps && m.k !== 'working'} class:sel={memOpen === m.k} onclick={() => (memOpen = m.k)}>
          <b>{m.t}</b><span class="dim">{m.d}</span>
          <em class="mono">{mem ? `${mem.counts?.[m.k as MemoryType] ?? 0} entries` : 'reading…'}</em>
        </button>
      {/each}
    </div>
    {#if mem}
      <p class="dim">Namespace <span class="mono">{mem.namespace}</span> on the org's ADB.</p>
      <ol class="memlist">
        {#each (mem.memory?.[memOpen] ?? []).slice(0, 20) as e, i (i)}
          <li class="mono">{memText(e)}</li>
        {:else}
          <li class="dim">Nothing in {memOpen} memory yet.</li>
        {/each}
      </ol>
    {/if}
  {:else if !live.loading}
    <ol class="tl">
      {#each items as it}
        <li class="ev" class:err={it.err}><span class="ic">{it.ic}</span><div><div class="t">{it.title}</div><div class="m mono">{it.meta}</div></div><time>{ago(it.ts)}</time></li>
      {:else}
        <li class="empty">Nothing{view !== 'all' ? ' of this kind' : ''} yet.</li>
      {/each}
    </ol>
    <ul class="notes">
      <li>Which app or MCP client made each call needs app keys (SRV-15). Test chats are yours only (UI-4).</li>
      <li>{live.decErr ? live.decErr : `Decisions are matched in the latest ${live.scanned} records of the log (UI-3).`}</li>
    </ul>
  {/if}

  <div class="card">
    <div class="card-head"><h3>Changes</h3><span class="tag sample">sample</span><span>who changed it, what, and who approved</span></div>
    <table class="tbl">
      <thead><tr><th>When</th><th>Who</th><th>What</th><th>Approved by</th></tr></thead>
      <tbody>
        <tr><td class="k">2 d ago</td><td class="mono">{agent.created_by || '—'}</td><td>tools: + acme/jira</td><td class="dim">— (no rule then)</td></tr>
        <tr><td class="k">5 d ago</td><td class="mono">{agent.created_by || '—'}</td><td>instructions edited (3 lines)</td><td class="dim">—</td></tr>
        <tr><td class="k">{agent.created_at ? ago(agent.created_at) : '—'}</td><td class="mono">{agent.created_by || '—'}</td><td>created{f.template ? ` from ${f.template.name}@${f.template.version}` : ''}</td><td class="dim">—</td></tr>
      </tbody>
    </table>
    <p class="dim">Only its creation is recorded today; edits are not (SRV-20). The rows above it show how changes will read.</p>
  </div>
</section>

<style>
  .tiles.six { grid-template-columns: repeat(6, 1fr); }
  .mem { display: grid; grid-template-columns: repeat(auto-fill, minmax(190px, 1fr)); gap: 10px; }
  .mem .card { gap: 6px; padding: 12px 14px; }
  .mem .card.off { opacity: 0.55; }
  .mem button.card { text-align: left; font: inherit; cursor: pointer; }
  .mem .card.sel { border-color: var(--spec); background: var(--signal-bg); }
  .memlist { margin: 0; padding: 0 0 0 18px; display: flex; flex-direction: column; gap: 6px; font-size: 12px; color: var(--text-2); }
  .mem em { font-style: normal; font-size: 11px; color: var(--text-4); }
  .ic { font-size: 9.5px !important; }
</style>
