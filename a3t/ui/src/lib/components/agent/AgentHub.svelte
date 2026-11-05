<script lang="ts">
  /// One agent, seen from the questions people bring to it — the same screens
  /// for everyone; they only start on different tabs:
  ///   overview  what is it, is it working, where to look next
  ///   activity  what it did: conversations, runs, what was blocked
  ///   memory    what it keeps and how (live contents: UI-1)
  ///   controls  who it acts as, what it may do, its hooks, what is logged
  ///   health    how its deployment is doing: runs, errors, latency, host
  ///   deploy    where it runs (the canvas editor follows in the page)
  /// Data comes from the APIs that exist; every gap is named, not faked
  /// (.ai/A3T_UI_LIMITATIONS.md).
  import { untrack } from 'svelte';
  import { navigateTo } from '$lib/stores/panels.svelte.ts';
  import { hasRole } from '$lib/stores/auth';
  import {
    factsOf, servingRuns, decisionsFor, conversationsWith, placementOf, ago, dur, bytes, STATUS_WORDS,
    type AgentRec, type TemplateRec, type Run, type Decision, type Placement,
  } from './agentData';
  import type { ChatSessionSummary } from '$lib/services/api';

  let { agent, templates, section, ongo, onstudio }: {
    agent: AgentRec; templates: TemplateRec[];
    section: 'overview' | 'activity' | 'memory' | 'controls' | 'health' | 'deploy';
    /// Show another tab of the agent.
    ongo: (tab: string) => void;
    onstudio?: () => void;
  } = $props();

  const f = $derived(factsOf(agent, templates));

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
    const key = `${agent.id}|${templates.length}`;
    if (key === loadedFor) return;
    loadedFor = key;
    untrack(() => void load());
  });

  const failed = $derived(runs.filter((r) => ['failed', 'error', 'cancelled'].includes(r.status)));
  const denied = $derived(decisions.filter((d) => !d.allow));
  const lastSeen = $derived.by(() => {
    const ts = [...runs.map((r) => Date.parse(r.started)), ...convs.map((c) => Date.parse(c.updated_at)), ...decisions.map((d) => d.ts)].filter((x) => x > 0);
    return ts.length ? Math.max(...ts) : 0;
  });
  const health = $derived.by((): { tone: 'ok' | 'warn' | 'err' | 'idle'; word: string } => {
    const er = placement?.stats?.errorRate;
    if (placement?.deploymentState === 'paused') return { tone: 'warn', word: 'Paused' };
    if (er != null && er > 0.2) return { tone: 'err', word: 'Failing' };
    if ((er != null && er > 0.02) || failed.length > 0) return { tone: 'warn', word: 'Some failures' };
    if (!runs.length && !convs.length) return { tone: 'idle', word: 'No activity yet' };
    return { tone: 'ok', word: 'Working' };
  });

  // ── Activity: one timeline ──
  type Item = { kind: 'conv' | 'run' | 'deny' | 'allow'; ts: number; title: string; meta: string; tone?: string };
  let filter = $state<'all' | 'conv' | 'run' | 'deny'>('all');
  const timeline = $derived.by((): Item[] => {
    const items: Item[] = [
      ...convs.map((c) => ({ kind: 'conv' as const, ts: Date.parse(c.updated_at), title: c.title || 'Conversation', meta: `${c.message_count} messages` })),
      ...runs.map((r) => ({ kind: 'run' as const, ts: Date.parse(r.started), title: `Run ${r.status}`, meta: `${r.flow} · ${dur(r.ms)}${r.error ? ` · ${r.error.slice(0, 120)}` : ''}`, tone: failed.includes(r) ? 'err' : r.status === 'completed' ? 'ok' : '' })),
      ...decisions.map((d) => ({ kind: d.allow ? 'allow' as const : 'deny' as const, ts: d.ts, title: `${d.allow ? 'Allowed' : 'Blocked'}: ${d.action}`, meta: `${d.resource}${d.obo ? ` · on behalf of ${d.obo}` : ''}${d.reason ? ` · ${d.reason}` : ''}`, tone: d.allow ? '' : 'err' })),
    ];
    return items.filter((i) => filter === 'all' || i.kind === filter || (filter === 'deny' && i.kind === 'deny')).sort((a, b) => b.ts - a.ts).slice(0, 200);
  });

  const isSecurity = hasRole('admin');
  function chat() {
    try { localStorage.setItem('a3t.chat.agent', JSON.stringify(agent.id)); } catch { /* private window */ }
    navigateTo('chat', 'AI Chat');
  }
  const MEM = [
    { k: 'working', t: 'Working', d: 'What it holds during one conversation or run — the current task, recent turns.' },
    { k: 'episodic', t: 'Episodic', d: 'What happened: past conversations and runs, as events it can recall.' },
    { k: 'semantic', t: 'Semantic', d: 'What it knows: facts and summaries, searched by meaning.' },
    { k: 'procedural', t: 'Procedural', d: 'How to do things: steps and preferences it has learned.' },
    { k: 'tools', t: 'Tools', d: 'What its tools returned, kept for reuse.' },
  ];
  const memoryMcp = $derived(f.mcp.find((m) => /memory/i.test(m)) ?? '');
</script>

{#if section === 'overview'}
  <section class="hub">
    <header class="hero">
      <p class="purpose">{f.purpose || 'Its template has no description yet.'}</p>
      <div class="status">
        <span class="dot {health.tone}"></span><b>{health.word}</b>
        <span class="sep">·</span><span>{STATUS_WORDS[agent.status] ?? agent.status}</span>
        {#if lastSeen}<span class="sep">·</span><span>last active {ago(lastSeen)}</span>{/if}
      </div>
      <div class="acts">
        <button class="btn spec sm" onclick={chat}>Chat with it</button>
        {#if onstudio}<button class="btn ghost sm" onclick={onstudio}>Open in Studio</button>{/if}
      </div>
    </header>

    <div class="tiles">
      <button class="tile" onclick={() => { filter = 'conv'; ongo('activity'); }}><span class="v">{loading ? '…' : convs.length}</span><span class="k">Your conversations</span></button>
      <button class="tile" onclick={() => { filter = 'run'; ongo('activity'); }}><span class="v">{loading ? '…' : runs.length}</span><span class="k">Recent runs</span></button>
      <button class="tile" class:bad={failed.length > 0} onclick={() => ongo('health')}><span class="v">{loading ? '…' : failed.length}</span><span class="k">Failed</span></button>
      <button class="tile" class:bad={denied.length > 0} onclick={() => { filter = 'deny'; ongo('activity'); }}><span class="v">{loading ? '…' : decErr ? '—' : denied.length}</span><span class="k">Blocked actions</span></button>
    </div>

    <dl class="facts">
      <dt>Project</dt><dd class="mono">{agent.namespace}</dd>
      <dt>Template</dt><dd>{#if f.template}<button class="lnk" onclick={() => ongo('template')}>{f.template.name} v{f.template.version}</button>{:else}<span class="mono">{agent.template_id ?? '—'}</span>{/if}</dd>
      <dt>Model</dt><dd class="mono">{f.llm || '—'}</dd>
      <dt>Tools</dt><dd>{f.mcp.length ? f.mcp.join(', ') : 'no MCP servers'}{f.tools ? (f.tools.length ? ` · ${f.tools.length} pre-approved` : ' · each call needs approval') : ''}</dd>
      <dt>Mode</dt><dd>{agent.mode}</dd>
      <dt>Endpoint</dt><dd class="mono">{f.endpoint ? `POST /webhook${f.endpoint}` : '—'}</dd>
      <dt>Served by</dt><dd class="mono">{f.target || '—'}</dd>
    </dl>

    <h4>Look closer</h4>
    <div class="next">
      <button onclick={() => ongo('activity')}><b>What it did</b><span>Conversations, runs and what was blocked</span></button>
      <button onclick={() => ongo('memory')}><b>What it remembers</b><span>Its memory tiers and how they are kept</span></button>
      <button onclick={() => ongo('controls')}><b>What it may do</b><span>Identity, tools, hooks, policies, logging</span></button>
      <button onclick={() => ongo('health')}><b>How it is running</b><span>Errors, latency, its runtime</span></button>
      <button onclick={() => ongo('designer')}><b>How it is built</b><span>Its flow and hooks — open in Studio</span></button>
      <button onclick={() => ongo('canvas')}><b>Where it is deployed</b><span>Canvas, runtime and state</span></button>
    </div>
  </section>

{:else if section === 'activity'}
  <section class="hub">
    <div class="chips">
      {#each [['all', 'All'], ['conv', `Conversations (${convs.length})`], ['run', `Runs (${runs.length})`], ['deny', `Blocked (${denied.length})`]] as [k, label] (k)}
        <button class:on={filter === k} onclick={() => (filter = k as any)}>{label}</button>
      {/each}
      <span class="grow"></span>
      <button class="lnk" onclick={() => navigateTo('logs', 'Decision log', { q: agent.id })}>Decision log →</button>
      <button class="lnk" onclick={() => navigateTo('executions', 'Executions')}>Executions →</button>
    </div>
    {#if loading}
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
          <li class="empty">Nothing yet{filter !== 'all' ? ' of this kind' : ''}. Chat with it to start.</li>
        {/each}
      </ol>
    {/if}
    <ul class="notes">
      <li>Conversations are yours only; others' need an API (UI-4).</li>
      <li>Runs are those of its serving flow <span class="mono">{f.target || '—'}</span>, which other agents may share (UI-2).</li>
      <li>{decErr ? decErr : `Decisions are matched in the latest ${scanned} records of the log (UI-3).`}</li>
    </ul>
  </section>

{:else if section === 'memory'}
  <section class="hub">
    <div class="banner">
      <b>Live memory isn't readable here yet.</b> The console has no API to read one agent's memory — its entries, what it learned and when (UI-1, UI-5).
      Below is how its memory is set up.
    </div>
    <div class="mem">
      {#each MEM as m (m.k)}
        <div class="mcard"><b>{m.t}</b><span>{m.d}</span><em>entries: — (UI-1)</em></div>
      {/each}
    </div>
    <dl class="facts">
      <dt>Memory service</dt><dd>{memoryMcp ? `${memoryMcp} (MCP) — ADB memory` : 'none bound — it keeps only working memory'}</dd>
      <dt>Mode</dt><dd>{agent.mode === 'persistent' ? 'persistent — memory carries across sessions' : agent.mode === 'session' ? 'session — memory lasts for one conversation' : agent.mode}</dd>
      <dt>Settings</dt><dd class="mono">{f.memory ? JSON.stringify(f.memory) : 'template defaults'}</dd>
      <dt>Recall / store hooks</dt><dd>{f.hooks.filter((h) => /recall|store/.test(h.name)).map((h) => `${h.name}${h.flows.length ? ` → ${h.flows.join(', ')}` : ''}`).join(' · ') || '—'}</dd>
    </dl>
    {#if isSecurity}<p class="dim">Admins can query memory tables in Settings → ADB SQL in the meantime.</p>{/if}
  </section>

{:else if section === 'controls'}
  <section class="hub">
    <h4>Identity</h4>
    <dl class="facts">
      <dt>Acts as</dt><dd class="mono">agent:{agent.id}</dd>
      <dt>On behalf of</dt><dd>the person or service that calls it — every tool call is decided for both (Cedar)</dd>
      <dt>Called through</dt><dd class="mono">{f.endpoint ? `POST /webhook${f.endpoint}` : '—'}</dd>
      <dt>Egress</dt><dd>through the platform's policy choke points; per-agent gateway tokens arrive with the gateway (UI-6)</dd>
    </dl>
    <h4>Tools</h4>
    <p class="line">{!f.tools ? 'Not restricted — every tool of its MCP servers may run; policies still decide each call.' : f.tools.length === 0 ? 'No tool is pre-approved — every tool call needs approval before it runs; policies still decide each call.' : `${f.tools.length} tool${f.tools.length === 1 ? ' runs' : 's run'} without asking — any other tool needs approval first; policies still decide each call.`}
      {#if f.mcp.length}<span class="dim"> Servers: {f.mcp.join(', ')}.</span>{/if}</p>
    <h4>Hooks</h4>
    <table class="tbl">
      <thead><tr><th>Hook</th><th>Applies to</th><th>Runs</th></tr></thead>
      <tbody>
        {#each f.hooks as h (h.name)}
          <tr><td class="mono">{h.name}</td><td>{h.extend}{h.when ? ` · when ${h.when}` : ''}</td><td>{h.flows.length ? h.flows.join(', ') : '— nothing attached'}</td></tr>
        {:else}
          <tr><td colspan="3" class="dim">Its template defines no hooks.</td></tr>
        {/each}
      </tbody>
    </table>
    <h4>Decisions about it</h4>
    <p class="line">{decErr ? decErr : `${decisions.length - denied.length} allowed, ${denied.length} blocked in the latest ${scanned} log records.`}
      <button class="lnk" onclick={() => navigateTo('logs', 'Decision log', { q: agent.id })}>Open in the decision log →</button>
      {#if isSecurity}<button class="lnk" onclick={() => navigateTo('security', 'Security', { selectedId: 'policies' })}>Policies →</button>{/if}</p>
    <h4>What is logged</h4>
    <ul class="plain">
      <li>Every allow and deny for its tool calls and data access — the decision log.</li>
      <li>Its conversations — transcripts, kept with the caller.</li>
      <li>Its runs — input, output and errors, in the run store.</li>
      <li class="dim">Retention, redaction and per-agent audit level have no setting to show yet (UI-7).</li>
    </ul>
    <h4>Tool permissions</h4>
  </section>

{:else if section === 'health'}
  <section class="hub">
    {#if placement?.stats}
      {@const s = placement.stats}
      <div class="tiles six">
        <div class="tile"><span class="v">{s.runs}</span><span class="k">Runs</span></div>
        <div class="tile"><span class="v">{s.runsLastHour}</span><span class="k">Last hour</span></div>
        <div class="tile" class:bad={s.errorRate > 0.02}><span class="v">{(s.errorRate * 100).toFixed(1)}%</span><span class="k">Error rate</span></div>
        <div class="tile"><span class="v">{dur(s.p50Ms)}</span><span class="k">p50</span></div>
        <div class="tile"><span class="v">{dur(s.p95Ms)}</span><span class="k">p95</span></div>
        <div class="tile" class:bad={s.refused > 0}><span class="v">{s.refused}</span><span class="k">Refused</span></div>
      </div>
      {#if s.lastError}<div class="banner err"><b>Last error</b> {s.lastError}</div>{/if}
    {:else}
      <div class="banner">{placeErr || 'Not deployed through a canvas, so there are no deployment figures. Below: the runs of its serving flow.'}</div>
    {/if}
    <dl class="facts">
      <dt>Deployment</dt><dd>{placement ? `${placement.serving}${placement.deploymentState === 'paused' ? ' (paused)' : ''} on ${placement.runtime || 'no runtime'}${placement.canvas ? ` · canvas ${placement.canvas}` : ''}` : 'not deployed'}</dd>
      {#if placement?.host}
        <dt>Host CPU</dt><dd>{placement.host.cpu != null ? `${placement.host.cpu.toFixed(2)} / ${placement.host.cpuLimit ?? '—'} cores` : '—'}</dd>
        <dt>Host memory</dt><dd>{bytes(placement.host.memUsed)} / {bytes(placement.host.memLimit)}</dd>
      {/if}
      <dt>Serving flow runs</dt><dd>{runs.length} recent · {failed.length} failed{runs.length ? ` · median ${dur([...runs.map((r) => r.ms ?? 0)].sort((a, b) => a - b)[Math.floor(runs.length / 2)])}` : ''}</dd>
      <dt>Restart policy</dt><dd><button class="lnk" onclick={() => ongo('canvas')}>in its canvas →</button></dd>
    </dl>
    <h4>Recent failures</h4>
    <ol class="tl">
      {#each failed.slice(0, 20) as r (r.id)}
        <li class="ev run err"><span class="ic">▶</span><div class="body"><div class="t">{r.status} · {r.flow}</div><div class="m">{r.error || 'no error message'}</div></div><time>{ago(r.started)}</time></li>
      {:else}
        <li class="empty">No failed runs{runsErr ? ` (${runsErr})` : ''}.</li>
      {/each}
    </ol>
    <ul class="notes">
      <li>Token use, cost and latency over time per agent need a metrics API (UI-8).</li>
      <li>Restarts, crashes and out-of-memory events need runtime events (UI-9); its application log needs a log API (UI-10).</li>
    </ul>
  </section>

{:else if section === 'deploy'}
  <section class="hub">
    <dl class="facts">
      <dt>Deployed</dt><dd>{placement ? `yes — ${placement.serving} on ${placement.runtime || 'no runtime yet'}` : loading ? '…' : 'no — it answers through the shared runtime'}</dd>
      {#if placement}<dt>State</dt><dd>{placement.deploymentState}{placement.runtimeState ? ` · runtime ${placement.runtimeState}` : ''}</dd>
      <dt>Canvas</dt><dd class="mono">{placement.canvas || '—'}</dd>{/if}
      <dt>Operate</dt><dd><button class="lnk" onclick={() => navigateTo('runtime', 'Runtimes')}>Runtimes → pause, resume, restart</button></dd>
    </dl>
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
  .acts { display: flex; gap: 8px; margin-top: 2px; }

  .tiles { display: grid; grid-template-columns: repeat(4, 1fr); gap: 10px; }
  .tiles.six { grid-template-columns: repeat(6, 1fr); }
  .tile { display: flex; flex-direction: column; gap: 4px; padding: 12px 14px; border: 1px solid var(--border); border-radius: 8px; background: var(--bg-surface); text-align: left; font: inherit; color: inherit; }
  button.tile { cursor: pointer; }
  button.tile:hover { border-color: var(--spec); }
  .tile .v { font-size: 22px; font-weight: 600; font-variant-numeric: tabular-nums; color: var(--text-1); }
  .tile .k { font-size: 11.5px; color: var(--text-3); }
  .tile.bad .v { color: var(--err); }

  .facts { display: grid; grid-template-columns: 150px 1fr; gap: 9px 16px; margin: 0; font-size: 13px; }
  .facts dt { color: var(--text-4); }
  .facts dd { margin: 0; color: var(--text-1); overflow-wrap: anywhere; }

  .next { display: grid; grid-template-columns: repeat(3, 1fr); gap: 10px; }
  .next button { display: flex; flex-direction: column; gap: 4px; padding: 12px 14px; border: 1px solid var(--border); border-radius: 8px; background: var(--bg-surface); text-align: left; cursor: pointer; font: inherit; }
  .next button:hover { border-color: var(--spec); background: var(--spec-bg); }
  .next b { font-size: 13px; color: var(--text-1); font-weight: 600; }
  .next span { font-size: 11.5px; color: var(--text-3); }

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
  .plain { margin: 0; padding-left: 18px; font-size: 13px; color: var(--text-2); display: flex; flex-direction: column; gap: 5px; }
  .line { margin: 0; font-size: 13px; color: var(--text-2); display: flex; flex-wrap: wrap; gap: 10px; align-items: baseline; }

  .banner { padding: 12px 14px; border-radius: 8px; background: var(--bg-subtle); border: 1px solid var(--border); font-size: 12.5px; color: var(--text-2); line-height: 1.5; }
  .banner.err { background: var(--err-bg); border-color: color-mix(in srgb, var(--err) 30%, transparent); }
  .mem { display: grid; grid-template-columns: repeat(auto-fill, minmax(200px, 1fr)); gap: 10px; }
  .mcard { display: flex; flex-direction: column; gap: 6px; padding: 12px 14px; border: 1px solid var(--border); border-radius: 8px; background: var(--bg-surface); }
  .mcard b { font-size: 13px; }
  .mcard span { font-size: 12px; color: var(--text-3); line-height: 1.45; }
  .mcard em { font-style: normal; font-size: 11px; color: var(--text-4); font-family: var(--font-mono); }

  .tbl { width: 100%; border-collapse: collapse; font-size: 12.5px; background: var(--bg-surface); border: 1px solid var(--border); border-radius: 8px; overflow: hidden; }
  .tbl th { text-align: left; font-size: 10.5px; font-weight: 600; text-transform: uppercase; letter-spacing: 0.05em; color: var(--text-4); padding: 8px 12px; border-bottom: 1px solid var(--border); }
  .tbl td { padding: 8px 12px; border-top: 1px solid var(--divider, var(--border)); color: var(--text-2); }
</style>
