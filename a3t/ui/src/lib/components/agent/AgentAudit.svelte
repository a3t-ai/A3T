<script lang="ts">
  /// Audit — what it did and what changed: every decision about it (allowed
  /// and blocked), its calls and errors, test chats, memory, and the history of
  /// changes to it. Deep debugging is the Studio's.
  import { navigateTo } from '$lib/stores/panels.svelte.ts';
  import { factsOf, ago, dur, type AgentRec, type TemplateRec } from './agentData';
  import type { AgentLive } from './agentLive.svelte';

  let { agent, templates, live, onstudio, ongo }: {
    agent: AgentRec & { created_by?: string; created_at?: string };
    templates: TemplateRec[];
    live: AgentLive;
    onstudio: () => void;
    ongo: (tab: string) => void;
  } = $props();

  const f = $derived(factsOf(agent, templates));
  type View = 'all' | 'deny' | 'allow' | 'run' | 'err' | 'conv';
  import { insight, describeEvent, type AgentEvent, type Turn, type TurnSummary } from './controlPlane';
  /// Its own turns (the turn log) and its history (agent_events), from the control plane.
  let own = $state<{ summary: TurnSummary; turns: Turn[] } | null>(null);
  let history = $state<AgentEvent[] | null>(null);
  let histErr = $state('');
  $effect(() => {
    const id = agent.id;
    own = null; history = null; histErr = '';
    insight.turns(id).then((r) => { if (r.ok && agent.id === id) own = r.data; });
    insight.events(id).then((r) => { if (agent.id !== id) return; if (r.ok) history = r.data; else histErr = r.error ?? 'Its history could not be read.'; });
  });
  let view = $state<View>('all');
  type Item = { kind: 'conv' | 'run' | 'deny' | 'allow'; ts: number; title: string; meta: string; err?: boolean; ic: string };
  /// Its calls: a deployed agent's runs; otherwise its own turns — the serving
  /// flow's runs are every agent's, not its own.
  const deployed = $derived(!!live.placement?.stats);
  const calls = $derived.by(() => deployed
    ? live.runs.map((r) => ({ kind: 'run' as const, ic: 'RUN', ts: Date.parse(r.started), title: `${r.status} · ${dur(r.ms)}`, meta: `${r.flow} · ${r.id}${r.error ? ` · ${r.error.slice(0, 160)}` : ''}`, err: live.failed.includes(r) }))
    : (own?.turns ?? []).map((t) => ({ kind: 'run' as const, ic: 'TURN', ts: Date.parse(t.at), title: `${t.outcome || 'answered'} · ${dur(Number(t.duration_ms) || null)}`,
        meta: `session ${t.session_id}${Number(t.tool_calls) ? ` · ${t.tool_calls} tool calls` : ''}${t.degraded ? ` · degraded: ${t.degraded}` : ''}`, err: !!t.outcome && t.outcome !== 'answered' })));
  const items = $derived.by((): Item[] => {
    const all: Item[] = [
      ...calls,
      ...live.convs.map((c) => ({ kind: 'conv' as const, ic: 'TEST', ts: Date.parse(c.updated_at), title: c.title || 'Test chat', meta: `${c.message_count} messages · ${c.session_id}` })),
      ...live.decisions.map((d) => ({ kind: d.allow ? 'allow' as const : 'deny' as const, ic: d.allow ? 'OK' : 'DENY', ts: d.ts, title: `${d.allow ? 'allowed' : 'blocked'}: ${d.action}`, meta: `${d.resource}${d.obo ? ` · on behalf of ${d.obo}` : ''}${d.reason ? ` · ${d.reason}` : ''}`, err: !d.allow })),
    ];
    const keep = (i: Item) => view === 'all' || (view === 'err' ? i.kind === 'run' && !!i.err : i.kind === view);
    return all.filter(keep).sort((a, b) => b.ts - a.ts).slice(0, 200);
  });
</script>

<section class="agt">
  <div class="chips">
    {#each [['all', 'All'], ['deny', `Blocked (${live.denied.length})`], ['allow', `Allowed (${live.decisions.length - live.denied.length})`], ['run', `Calls (${calls.length})`], ['err', `Errors (${calls.filter((c) => c.err).length})`], ['conv', `Test chats (${live.convs.length})`]] as [k, label] (k)}
      <button class:on={view === k} onclick={() => (view = k as View)}>{label}</button>
    {/each}
    <span class="grow"></span>
    <button class="link" onclick={onstudio}>Debug in Studio →</button>
    <button class="link" onclick={() => navigateTo('executions', 'Executions')}>Executions →</button>
    <button class="link" onclick={() => navigateTo('logs', 'Decision log', { q: agent.id })}>Decision log →</button>
  </div>

  {#if !live.loading}
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
    <div class="card-head"><h3>Changes</h3><span>who changed it and what, from its history</span></div>
    {#if histErr}<p class="err">{histErr}</p>
    {:else if !history}<p class="dim">…</p>
    {:else if history.length}
      <table class="tbl">
        <thead><tr><th>When</th><th>Who</th><th>What</th></tr></thead>
        <tbody>{#each history as e, i (i)}<tr><td class="k">{ago(e.at)}</td><td class="mono">{e.by || '—'}</td><td>{describeEvent(e)}</td></tr>{/each}</tbody>
      </table>
    {:else}
      <p class="dim">Nothing recorded yet. Every change through the control plane is kept here: edits, blocks, shares, merges.</p>
    {/if}
  </div>
</section>

<style>
  .ic { font-size: 9.5px !important; }
</style>
