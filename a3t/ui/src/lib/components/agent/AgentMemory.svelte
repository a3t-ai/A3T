<script lang="ts">
  /// Build › Memory — what it remembers, each kind shown as what it is:
  /// episodic as a timeline (newest first), working by conversation, semantic as
  /// facts, procedural as learned steps. Read from its memory space on the org's
  /// ADB through agent_a3t_agents (tool_a3t_agent_memory). Its memory setting is
  /// changed in Model & tools.
  import { insight, type MemoryType } from './controlPlane';
  import { ago, type AgentRec } from './agentData';

  let { agent, ongo }: { agent: AgentRec; ongo: (tab: string) => void } = $props();

  type Entry = { key: string | null; at: string | null; data: Record<string, any> };
  let mem = $state<{ namespace: string; counts: Record<MemoryType, number>; memory: Record<MemoryType, Entry[]> } | null>(null);
  let err = $state('');
  let open = $state<MemoryType>('episodic');
  let filter = $state('');

  /// Entries as {key, at, data}, newest first (an older control plane sent the data alone).
  const norm = (list: any[]): Entry[] => (list ?? []).map((e) =>
    e && typeof e === 'object' && 'data' in e && ('at' in e || 'key' in e) ? { key: e.key ?? null, at: e.at ?? null, data: e.data ?? {} } : { key: null, at: null, data: e ?? {} })
    .sort((a, b) => (b.at ?? '').localeCompare(a.at ?? ''));

  async function load(id: string) {
    mem = null; err = '';
    const r = await insight.memory(id);
    if (agent.id !== id) return;
    if (!r.ok) { err = r.error ?? 'Its memory could not be read.'; return; }
    const m: any = r.data;
    mem = { namespace: m.namespace, counts: m.counts, memory: Object.fromEntries(Object.entries(m.memory ?? {}).map(([k, v]) => [k, norm(v as any[])])) as any };
  }
  $effect(() => { load(agent.id); });

  const TYPES: { k: MemoryType; t: string; d: string }[] = [
    { k: 'episodic', t: 'Episodic', d: 'what happened: its answers, turn by turn' },
    { k: 'working', t: 'Working', d: 'the conversations it is in now' },
    { k: 'semantic', t: 'Semantic', d: 'facts it knows' },
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

  const list = $derived((mem?.memory?.[open] ?? []).filter((e) => !filter.trim() || JSON.stringify(e.data).toLowerCase().includes(filter.trim().toLowerCase())));
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
</script>

<section class="agt">
  <div class="card">
    <div class="card-head"><h3>Memory</h3><span>{keeps ? 'kept across conversations' : agent.mode === 'ephemeral' ? 'it remembers nothing' : 'only the conversation it is in'}</span><span class="grow"></span>
      <button class="link" onclick={() => load(agent.id)}>Refresh</button>
      <button class="link" onclick={() => ongo('tools')}>Change what it remembers →</button></div>
    <div class="types" role="tablist">
      {#each TYPES as ty (ty.k)}
        <button role="tab" class:on={open === ty.k} aria-selected={open === ty.k} onclick={() => { open = ty.k; filter = ''; }}>
          <b>{ty.t}</b><span class="n">{mem ? mem.counts?.[ty.k] ?? 0 : '…'}</span><span class="dim">{ty.d}</span>
        </button>
      {/each}
    </div>
    {#if err}<p class="err">{err}</p>{/if}
    {#if mem && (mem.memory?.[open] ?? []).length > 5}<input type="search" bind:value={filter} placeholder="Filter {open} memory" />{/if}
  </div>

  {#if !mem}<p class="dim">{err ? '' : 'Reading its memory…'}</p>
  {:else if !list.length}
    <p class="dim">{filter ? 'Nothing matches.' : `Nothing in ${open} memory yet.`} {open === 'episodic' || open === 'working' ? 'Talk to it on Overview.' : open === 'semantic' ? 'Facts are written when it reflects on a conversation, or when told to remember.' : 'Procedures come from merged branches and learned steps.'}</p>
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
              <div class="meta"><span class="chip">{e.data.role === 'user' ? 'they said' : 'it answered'}</span><span class="mono">conversation {conv(e.data.session_id)}</span>
                {#if e.data.tool_calls_made}<span>· {e.data.tool_calls_made} tool call{Number(e.data.tool_calls_made) === 1 ? '' : 's'}</span>{/if}
                {#if e.data.iterations}<span>· {e.data.iterations} step{Number(e.data.iterations) === 1 ? '' : 's'}</span>{/if}
                {#if e.at}<span class="dim">· {ago(e.at)}</span>{/if}</div>
              <p class:clip={!expanded[id] && body.length > 280}>{body}</p>
              {#if body.length > 280}<button class="link" onclick={() => (expanded[id] = !expanded[id])}>{expanded[id] ? 'Less' : 'More'}</button>{/if}
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
          {#each items as e, i (e.key ?? i)}<li><time>{time(e.at)}</time><span class="chip">{e.data.role ?? 'entry'}</span><span>{text(e.data.content ?? e.data)}</span></li>{/each}
        </ol>
      </div>
    {/each}
    <p class="dim">Working memory is the conversation in hand: it expires with the conversation.</p>
  {:else if open === 'semantic'}
    <table class="tbl facts">
      <thead><tr><th>Fact</th><th>Subject</th><th>Predicate</th><th>Object / value</th><th>Confidence</th><th>From</th><th>When</th></tr></thead>
      <tbody>
        {#each list as e, i (e.key ?? i)}
          {@const conf = Number(e.data.confidence ?? NaN)}
          <tr>
            <td>{text(e.data.content ?? '') || '—'}</td>
            <td class="mono">{text(e.data.subject ?? '')}</td>
            <td class="mono">{text(e.data.predicate ?? '')}</td>
            <td class="mono">{text(e.data.object ?? e.data.value ?? '')}</td>
            <td>{#if Number.isFinite(conf)}<span class="bar"><span style="width:{Math.round(conf * 100)}%"></span></span> {Math.round(conf * 100)}%{:else}—{/if}</td>
            <td>{text(e.data.source ?? '') || '—'}{e.data.session_id ? ` · ${conv(e.data.session_id)}` : ''}</td>
            <td class="when">{e.at ? ago(e.at) : ''}</td>
          </tr>
        {/each}
      </tbody>
    </table>
  {:else}
    <ol class="procs">
      {#each list as e, i (e.key ?? i)}
        {@const steps = e.data.steps ?? e.data.actions ?? e.data.action ?? e.data.flow ?? e.data.response ?? e.data.content}
        <li class="card">
          <div class="card-head"><h3>When: <span class="mono">{text(e.data.pattern ?? e.data.when ?? e.key ?? '—')}</span></h3><span>{e.at ? ago(e.at) : ''}{e.data.source ? ` · ${text(e.data.source)}` : ''}</span></div>
          {#if Array.isArray(steps)}<ol class="steps">{#each steps as s, j (j)}<li>{text(s)}</li>{/each}</ol>
          {:else}<p>{text(steps ?? e.data)}</p>{/if}
        </li>
      {/each}
    </ol>
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
  input[type='search'] { margin-top: 8px; }
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
  .work li { display: grid; grid-template-columns: 48px auto 1fr; gap: 8px; align-items: baseline; font-size: 13px; }
  .work time { font: 12px var(--font-mono); color: var(--text-3); }
  .facts td { vertical-align: top; font-size: 12.5px; }
  .bar { display: inline-block; width: 48px; height: 6px; border-radius: 3px; background: var(--bg-subtle); vertical-align: middle; overflow: hidden; }
  .bar span { display: block; height: 100%; background: var(--spec); }
  .when { white-space: nowrap; color: var(--text-4); }
  .procs { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 8px; }
  .steps { margin: 0; padding-left: 20px; font-size: 13px; }
  .mono { font-family: var(--font-mono); font-size: 0.92em; }
</style>
