<script lang="ts">
  /// Metrics — how it is doing, from its own turns (the core's turn log,
  /// a3t_agent_turns, through agent_a3t_agents's tool_a3t_agent_turns): the
  /// headline figures for a period, turns over time (answered and failed), how
  /// long each turn took against the average, and the turns themselves.
  import { insight, type Turn } from './controlPlane';
  import { ago, dur } from './agentData';

  let { id }: { id: string } = $props();
  let turns = $state<Turn[] | null>(null);
  let error = $state('');
  async function load(agentId: string) {
    error = '';
    const r = await insight.turns(agentId);
    if (id !== agentId) return;
    if (r.ok) turns = [...(r.data.turns ?? [])].sort((a, b) => a.at.localeCompare(b.at)); else error = r.error ?? 'Its figures could not be read.';
  }
  $effect(() => { turns = null; load(id); });

  type Range = '24h' | '7d' | '30d' | 'all';
  let range = $state<Range>('7d');
  const HOUR = 3600e3, DAY = 24 * HOUR;
  const since = $derived(range === '24h' ? Date.now() - DAY : range === '7d' ? Date.now() - 7 * DAY : range === '30d' ? Date.now() - 30 * DAY : 0);
  const inRange = $derived((turns ?? []).filter((t) => Date.parse(t.at) >= since));
  const failed = (t: Turn) => !!t.outcome && t.outcome !== 'answered' && t.outcome !== 'null';
  const ms = (t: Turn) => Number(t.duration_ms) || 0;

  const stats = $derived.by(() => {
    const n = inRange.length;
    const times = inRange.map(ms).sort((a, b) => a - b);
    const f = inRange.filter(failed).length;
    return {
      n, failed: f, failRate: n ? f / n : 0,
      avg: n ? times.reduce((a, b) => a + b, 0) / n : 0,
      p95: n ? times[Math.min(n - 1, Math.floor(n * 0.95))] : 0,
      tools: inRange.reduce((a, t) => a + (Number(t.tool_calls) || 0), 0),
      steps: n ? inRange.reduce((a, t) => a + (Number(t.iterations) || 0), 0) / n : 0,
      degraded: inRange.filter((t) => t.degraded).length,
      convs: new Set(inRange.map((t) => t.session_id)).size,
      last: inRange.length ? inRange[inRange.length - 1].at : null,
    };
  });

  // ── Turns over time: hourly for a day, else daily ──
  const buckets = $derived.by(() => {
    const hourly = range === '24h';
    const step = hourly ? HOUR : DAY;
    const floor = (t: number) => (hourly ? Math.floor(t / HOUR) * HOUR : new Date(new Date(t).toDateString()).getTime());
    const first = range === 'all' ? (inRange[0] ? floor(Date.parse(inRange[0].at)) : floor(Date.now())) : floor(since + step);
    const last = floor(Date.now());
    const count = Math.min(90, Math.max(1, Math.round((last - first) / step) + 1));
    const start = last - (count - 1) * step;
    const out = Array.from({ length: count }, (_, i) => ({ t: start + i * step, ok: 0, bad: 0 }));
    for (const t of inRange) {
      const i = Math.round((floor(Date.parse(t.at)) - start) / step);
      if (i >= 0 && i < count) failed(t) ? out[i].bad++ : out[i].ok++;
    }
    return { hourly, list: out };
  });
  const W = 720, H = 180, PAD = { l: 34, r: 8, t: 10, b: 22 };
  const yMax = $derived(Math.max(1, ...buckets.list.map((b) => b.ok + b.bad)));
  const niceMax = $derived(yMax <= 4 ? yMax : Math.ceil(yMax / 5) * 5);
  const bw = $derived((W - PAD.l - PAD.r) / buckets.list.length);
  const yOf = (v: number, max: number) => PAD.t + (H - PAD.t - PAD.b) * (1 - v / max);
  const label = (t: number) => (buckets.hourly
    ? new Date(t).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' })
    : new Date(t).toLocaleDateString([], { day: 'numeric', month: 'short' }));
  const xTicks = $derived.by(() => { const n = buckets.list.length; return n <= 1 ? [0] : [0, Math.floor((n - 1) / 2), n - 1]; });

  // ── Time per turn ──
  const D_H = 160;
  const dMax = $derived(Math.max(1000, ...inRange.map(ms)) * 1.1);
  const dx = (i: number) => PAD.l + (inRange.length <= 1 ? (W - PAD.l - PAD.r) / 2 : (i * (W - PAD.l - PAD.r)) / (inRange.length - 1));
  const dy = (v: number) => PAD.t + (D_H - PAD.t - PAD.b) * (1 - v / dMax);

  let tip = $state<{ x: number; y: number; lines: string[] } | null>(null);
  function show(e: MouseEvent, lines: string[]) {
    const host = (e.currentTarget as Element).closest('.chart') as HTMLElement;
    const r = host.getBoundingClientRect();
    tip = { x: e.clientX - r.left, y: e.clientY - r.top, lines };
  }
  const recent = $derived([...inRange].reverse().slice(0, 50));
  const conv = (s: string) => s.split(':').slice(1).join(':') || s;
</script>

<section class="agt">
  <div class="row filters">
    <div class="seg">{#each [['24h', 'Last 24 hours'], ['7d', '7 days'], ['30d', '30 days'], ['all', 'All']] as [k, l] (k)}<button class:on={range === k} onclick={() => (range = k as Range)}>{l}</button>{/each}</div>
    <span class="grow"></span>
    <button class="link" onclick={() => load(id)}>Refresh</button>
  </div>

  {#if error}<p class="err">{error}</p>
  {:else if !turns}<p class="dim">Reading its turns…</p>
  {:else}
    <div class="heroes">
      <div class="hero"><span class="v">{stats.n}</span><span class="k">turns</span><span class="s">{stats.convs} conversation{stats.convs === 1 ? '' : 's'}</span></div>
      <div class="hero" class:bad={stats.failed > 0}><span class="v">{(stats.failRate * 100).toFixed(stats.failRate && stats.failRate < 0.1 ? 1 : 0)}%</span><span class="k">failed</span><span class="s">{stats.failed} of {stats.n}{stats.degraded ? ` · ${stats.degraded} degraded` : ''}</span></div>
      <div class="hero"><span class="v">{stats.n ? dur(stats.avg) : '—'}</span><span class="k">average turn</span><span class="s">p95 {stats.n ? dur(stats.p95) : '—'}</span></div>
      <div class="hero"><span class="v">{stats.tools}</span><span class="k">tool calls</span><span class="s">{stats.steps.toFixed(1)} steps a turn</span></div>
      <div class="hero"><span class="v">{stats.last ? ago(stats.last) : '—'}</span><span class="k">last answer</span><span class="s">{stats.last ? new Date(stats.last).toLocaleString() : 'none in this period'}</span></div>
    </div>

    {#if !inRange.length}
      <p class="dim">No turns in this period{turns.length ? ` — its last was ${ago(turns[turns.length - 1].at)}.` : ' — talk to it on Overview.'}</p>
    {:else}
      <div class="card">
        <div class="card-head"><h3>Turns {buckets.hourly ? 'per hour' : 'per day'}</h3><span class="grow"></span>
          <span class="legend"><i class="sw ok"></i>answered <i class="sw bad"></i>failed</span></div>
        <div class="chart" role="img" aria-label="Turns per {buckets.hourly ? 'hour' : 'day'}, answered and failed" onmouseleave={() => (tip = null)}>
          <svg viewBox="0 0 {W} {H}" preserveAspectRatio="xMidYMid meet">
            {#each [0, niceMax / 2, niceMax] as g (g)}
              <line class="grid" x1={PAD.l} x2={W - PAD.r} y1={yOf(g, niceMax)} y2={yOf(g, niceMax)} />
              <text class="ax" x={PAD.l - 6} y={yOf(g, niceMax) + 3} text-anchor="end">{Number.isInteger(g) ? g : g.toFixed(1)}</text>
            {/each}
            {#each buckets.list as b, i (b.t)}
              {@const x = PAD.l + i * bw + Math.max(1, bw * 0.15)}
              {@const w = Math.max(2, bw * 0.7)}
              {#if b.ok}<rect class="ok" x={x} y={yOf(b.ok, niceMax)} width={w} height={yOf(0, niceMax) - yOf(b.ok, niceMax)} rx={Math.min(4, w / 2)} />{/if}
              {#if b.bad}<rect class="bad" x={x} y={yOf(b.ok + b.bad, niceMax)} width={w} height={Math.max(1, yOf(b.ok, niceMax) - yOf(b.ok + b.bad, niceMax) - (b.ok ? 2 : 0))} rx={Math.min(4, w / 2)} />{/if}
              <rect class="hit" x={PAD.l + i * bw} y={PAD.t} width={bw} height={H - PAD.t - PAD.b}
                onmousemove={(e) => show(e, [label(b.t), `${b.ok + b.bad} turn${b.ok + b.bad === 1 ? '' : 's'}`, `${b.bad} failed`])} />
            {/each}
            <line class="base" x1={PAD.l} x2={W - PAD.r} y1={yOf(0, niceMax)} y2={yOf(0, niceMax)} />
            {#each xTicks as i (i)}
              <text class="ax" x={PAD.l + i * bw + bw / 2} y={H - 6} text-anchor="middle">{label(buckets.list[i].t)}</text>
            {/each}
          </svg>
          {#if tip}<div class="tip" style="left:{tip.x}px;top:{tip.y}px">{#each tip.lines as l, j (j)}<div class:b={j === 0}>{l}</div>{/each}</div>{/if}
        </div>
      </div>

      <div class="card">
        <div class="card-head"><h3>Time per turn</h3><span>each turn in order, against the average</span><span class="grow"></span>
          <span class="legend"><i class="dot ok"></i>answered <i class="dot bad"></i>failed <i class="dash"></i>average</span></div>
        <div class="chart" role="img" aria-label="Time per turn" onmouseleave={() => (tip = null)}>
          <svg viewBox="0 0 {W} {D_H}" preserveAspectRatio="xMidYMid meet">
            {#each [0, dMax / 2, dMax] as g (g)}
              <line class="grid" x1={PAD.l} x2={W - PAD.r} y1={dy(g)} y2={dy(g)} />
              <text class="ax" x={PAD.l - 6} y={dy(g) + 3} text-anchor="end">{(g / 1000).toFixed(g < 10000 ? 1 : 0)}s</text>
            {/each}
            <line class="avg" x1={PAD.l} x2={W - PAD.r} y1={dy(stats.avg)} y2={dy(stats.avg)} />
            <text class="ax avgl" x={W - PAD.r} y={dy(stats.avg) - 5} text-anchor="end">avg {dur(stats.avg)}</text>
            {#if inRange.length > 1}<polyline class="trace" points={inRange.map((t, i) => `${dx(i)},${dy(ms(t))}`).join(' ')} />{/if}
            {#each inRange as t, i (t.at + i)}
              <circle class:bad={failed(t)} class="pt" cx={dx(i)} cy={dy(ms(t))} r="4" />
              <circle class="hit" cx={dx(i)} cy={dy(ms(t))} r="10"
                onmousemove={(e) => show(e, [new Date(t.at).toLocaleString(), `${dur(ms(t))} · ${t.outcome || 'answered'}`, `conversation ${conv(t.session_id)}`, `${t.iterations || 0} steps · ${t.tool_calls || 0} tool calls`])} />
            {/each}
            <line class="base" x1={PAD.l} x2={W - PAD.r} y1={dy(0)} y2={dy(0)} />
          </svg>
          {#if tip}<div class="tip" style="left:{tip.x}px;top:{tip.y}px">{#each tip.lines as l, j (j)}<div class:b={j === 0}>{l}</div>{/each}</div>{/if}
        </div>
      </div>

      <div class="card">
        <div class="card-head"><h3>Turns</h3><span>newest first{inRange.length > 50 ? ', latest 50' : ''}</span></div>
        <table class="tbl">
          <thead><tr><th>When</th><th>Conversation</th><th>Outcome</th><th class="num">Steps</th><th class="num">Tool calls</th><th class="num">Time</th></tr></thead>
          <tbody>
            {#each recent as t, i (t.at + i)}
              <tr>
                <td title={new Date(t.at).toLocaleString()}>{ago(t.at)}</td>
                <td class="mono">{conv(t.session_id)}</td>
                <td><span class="out" class:bad={failed(t)}>{failed(t) ? '✕' : '✓'} {t.outcome || 'answered'}</span>{#if t.degraded}<span class="dim"> · degraded: {t.degraded}</span>{/if}</td>
                <td class="num">{t.iterations || 0}</td>
                <td class="num">{t.tool_calls || 0}</td>
                <td class="num">{dur(ms(t))}</td>
              </tr>
            {/each}
          </tbody>
        </table>
      </div>
    {/if}
  {/if}
</section>

<style>
  .filters { align-items: center; }
  .grow { flex: 1 1 auto; }
  .heroes { display: grid; grid-template-columns: repeat(auto-fit, minmax(140px, 1fr)); gap: 10px; }
  .hero { display: flex; flex-direction: column; gap: 2px; padding: 14px 16px; border: 1px solid var(--border); border-radius: 12px; background: var(--bg-surface); }
  .hero .v { font: 600 24px var(--font-sans); color: var(--text-1); letter-spacing: -0.01em; }
  .hero .k { font-size: 12.5px; color: var(--text-2); }
  .hero .s { font-size: 11.5px; color: var(--text-4); }
  .hero.bad .v { color: var(--err); }
  .legend { display: inline-flex; align-items: center; gap: 6px; font-size: 12px; color: var(--text-3); }
  .legend .sw { display: inline-block; width: 10px; height: 10px; border-radius: 3px; margin-left: 8px; }
  .legend .dot { display: inline-block; width: 8px; height: 8px; border-radius: 50%; margin-left: 8px; }
  .legend .dash { display: inline-block; width: 16px; border-top: 2px dashed var(--text-4); margin-left: 8px; }
  .legend .ok { background: var(--spec); }
  .legend .bad { background: var(--err); }
  .chart { position: relative; }
  .chart svg { width: 100%; height: auto; display: block; overflow: visible; }
  .grid { stroke: var(--divider, var(--border)); stroke-width: 1; }
  .base { stroke: var(--border-strong, var(--border)); stroke-width: 1; }
  .ax { font: 10.5px var(--font-mono); fill: var(--text-4); }
  rect.ok { fill: var(--spec); }
  rect.bad { fill: var(--err); }
  .hit { fill: transparent; cursor: default; }
  .hit:hover { fill: color-mix(in srgb, var(--text-1) 5%, transparent); }
  .trace { fill: none; stroke: color-mix(in srgb, var(--spec) 35%, transparent); stroke-width: 2; }
  .pt { fill: var(--spec); stroke: var(--bg-surface); stroke-width: 2; }
  .pt.bad { fill: var(--err); }
  .avg { stroke: var(--text-4); stroke-width: 1.5; stroke-dasharray: 4 4; }
  .tip { position: absolute; transform: translate(-50%, calc(-100% - 12px)); pointer-events: none; padding: 6px 10px; border-radius: 8px; background: var(--text-1); color: var(--bg-surface); font-size: 12px; white-space: nowrap; box-shadow: 0 4px 14px rgba(0,0,0,0.15); z-index: 2; }
  .tip .b { font-weight: 600; }
  .num { text-align: right; font-variant-numeric: tabular-nums; }
  .out { font-size: 12.5px; color: var(--text-2); }
  .out.bad { color: var(--err); font-weight: 600; }
  .mono { font-family: var(--font-mono); font-size: 0.92em; }
</style>
