<script lang="ts">
  /// One run, live (console spec §6–§8): its status and what it did step by
  /// step, the journal, a timeline, resume points to re-run from — and, for a
  /// debug run paused at a breakpoint, the controls to continue, step, or edit
  /// the next step's input. Reads the engine's run tools; polls while live.
  import { untrack } from 'svelte';
  import { runsApi, answerApproval, mcpCall, type DebugCommand } from '$lib/services/api';
  import { navigateToDetail } from '$lib/stores/panels.svelte.ts';
  import { nodeKey, type FlowDebug } from './debugState.svelte.ts';

  /// `shared`: the flow's state shared with the editor graph — this view
  /// publishes the run's position to it and shows the step picked there.
  let { runId, flowSteps = [], onrerun, shared = null, crumb = '', title = '', onopen, view = '', failure = null, onrun, onparent, onview }:
    { runId: string; flowSteps?: string[]; onrerun?: (id: string) => void; shared?: FlowDebug | null;
      /// Framed (the Executions app's Details): the frame's tab picks the view
      /// and this renders that one as cards — no header or tab bar of its own.
      view?: '' | 'overview' | 'steps' | 'timeline' | 'journal' | 'checkpoints' | 'io';
      /// The A3T error log's row for this run (admins), classifying its failure.
      failure?: any;
      /// The run as loaded, for the frame's header.
      onrun?: (run: any) => void;
      onparent?: (id: string) => void;
      /// A link inside a view moved to another: tell the frame.
      onview?: (v: string) => void;
      /// Item mode (the shell's Details pane): the run gets the standard item
      /// header (breadcrumb, name with its status, a meta line, the action bar)
      /// with the tabs straight under it.
      crumb?: string; title?: string; onopen?: () => void } = $props();
  import DetailActionsBar from '$lib/shell/DetailActionsBar.svelte';
  import { detailsPane } from '$lib/shell/detailsPane';
  import { setDetailActions } from '$lib/stores/detailActions.svelte.ts';
  const item = !!title && !!detailsPane();
  const framed = $derived(!!view);

  type Step = { step_key: string; status: string; class?: string; duration_us?: number; attempts?: number; edited?: boolean;
                started_at?: string; completed_at?: string; seq?: number; error?: any; fire_at?: string; listening?: any; asking?: any };
  type Entry = { seq: number; at: string; epoch: number; record: any };

  let run = $state<any>(null);

  /// What started it, to name and open: a trigger (or its cron tick), or the agent
  /// whose turn it is (a turn of tmpl_a3t_agent carries `<agent>:<conversation>`).
  const startedBy = $derived.by(() => {
    const t = String(run?.trigger ?? '');
    const m = t.match(/^(trigger|cron):(.+)$/);
    if (m) return { kind: 'trigger' as const, id: m[2], cron: m[1] === 'cron' };
    const sid = run?.input?.session_id;
    if (run?.flow === 'tmpl_a3t_agent' && typeof sid === 'string' && sid.includes(':')) return { kind: 'agent' as const, id: sid.split(':')[0], cron: false };
    return null;
  });
  let triggerNames = $state<Record<string, string>>({});
  $effect(() => {
    const sb = startedBy;
    if (sb?.kind !== 'trigger' || sb.id in triggerNames) return;
    untrack(async () => {
      const r = await mcpCall('list_triggers', {});
      let d: any = r.data; if (typeof d === 'string') { try { d = JSON.parse(d); } catch { d = []; } }
      const list: any[] = Array.isArray(d) ? d : (d?.triggers ?? []);
      triggerNames = { ...triggerNames, ...Object.fromEntries(list.map((x) => [x.id, x.name])), [sb.id]: list.find((x) => x.id === sb.id)?.name ?? '' };
    });
  });
  let entries = $state<Entry[]>([]);
  let error = $state('');
  let message = $state('');
  /// What just worked (shown in green; `message` is for what went wrong).
  let notice = $state('');
  let tab = $state<'steps' | 'timeline' | 'journal' | 'checkpoints' | 'io'>('steps');
  let selected = $state<string | null>(null);
  let detailTab = $state<'input' | 'output' | 'context' | 'records'>('output');
  let editing = $state(false);
  let editText = $state('');
  let busy = $state(false);
  let runTo = $state('');

  const LIVE = ['pending', 'running', 'waiting', 'suspended'];
  const live = $derived(!!run && LIVE.includes(run.phase));

  async function load() {
    const [r, j] = await Promise.all([runsApi.get(runId), runsApi.journal(runId, entries.length ? entries[entries.length - 1].seq : 0)]);
    if (!r.ok) { error = r.error ?? 'Could not load the run'; return; }
    error = '';
    run = r.data;
    onrun?.(r.data);
    shared?.show(r.data);
    if (j.ok && Array.isArray(j.data?.entries) && j.data.entries.length) entries = [...entries, ...j.data.entries];
  }

  let timer: ReturnType<typeof setTimeout> | null = null;
  $effect(() => {
    const id = runId;
    run = null; entries = []; selected = null; message = ''; notice = ''; editing = false;
    let stop = false;
    const tick = async () => {
      if (stop || id !== runId) return;
      await load();
      if (stop) return;
      const wait = !run ? 1500 : run.phase === 'suspended' ? 3000 : LIVE.includes(run.phase) ? 1000 : 0;
      if (wait) timer = setTimeout(tick, wait);
    };
    // Untracked: the poll reads and writes `run`/`entries`, which must not
    // re-trigger this effect — only a different run does.
    untrack(() => void tick());
    return () => { stop = true; if (timer) clearTimeout(timer); };
  });

  // ── Steps ──
  const steps = $derived<Step[]>(run?.steps ?? []);
  const paused = $derived(run?.phase === 'suspended' ? run?.paused : null);
  const rows = $derived.by(() => {
    const out = [...steps];
    if (paused && !out.some((s) => s.step_key === paused.step_key)) out.push({ step_key: paused.step_key, status: 'paused' });
    return out;
  });
  // A step picked on the graph: show its latest iteration here.
  $effect(() => {
    const want = shared?.focus;
    if (!want) return;
    const hit = [...rows].reverse().find((s) => nodeKey(s.step_key) === want);
    untrack(() => {
      tab = 'steps';
      selected = hit?.step_key ?? null;
      message = hit ? '' : `${want} has not run in this run.`;
      shared!.focus = null;
    });
  });

  function recordsOf(key: string): Entry[] {
    return entries.filter((e) => e.record?.step_key === key || e.record?.wait_id === key);
  }
  const last = (key: string) => [...recordsOf(key)].reverse().find((e) => ['step_completed', 'step_failed'].includes(e.record.kind))?.record;
  /// The context in effect after a step: its own, or the last one recorded before it.
  function contextAfter(key: string): any {
    let ctx: any = {};
    for (const e of entries) {
      if (e.record.kind === 'step_completed' && e.record.context !== null && e.record.context !== undefined) ctx = e.record.context;
      if (e.record.step_key === key && e.record.kind === 'step_completed') return ctx;
    }
    return null;
  }
  function icon(s: Step): string {
    if (paused?.step_key === s.step_key) return '⏸';
    return { completed: '✓', failed: '✗', started: '●', waiting: '⏳', paused: '⏸' }[s.status] ?? '·';
  }

  // ── Debug ──
  async function command(c: DebugCommand, stepKey?: string) {
    busy = true; message = ''; notice = '';
    const r = await runsApi.debug(runId, c, stepKey);
    busy = false;
    if (!r.ok) message = r.error ?? 'Command failed';
    await load();
  }
  function openEdit() {
    editText = JSON.stringify(paused?.input ?? {}, null, 2);
    editing = true;
  }
  async function applyEdit() {
    let input: any;
    try { input = JSON.parse(editText); } catch (e) { message = `Not valid JSON: ${(e as Error).message}`; return; }
    busy = true;
    const r = await runsApi.editInput(runId, paused.step_key, input);
    busy = false;
    if (!r.ok) { message = r.error ?? 'Edit failed'; return; }
    editing = false;
    notice = `Input for ${paused.step_key} replaced — Continue or Step to run it.`;
    await load();
  }
  // ── Events: wake a listening run from here ──
  let eventText = $state('');
  let sending = $state(false);
  function openSend(listening: any) {
    const f = { ...(listening?.filters?.[0] ?? { type: 'my.event' }) };
    eventText = JSON.stringify({ ...f, data: f.data ?? {} }, null, 2);
    sending = true;
  }
  async function sendEvent() {
    let event: any;
    try { event = JSON.parse(eventText); } catch (e) { message = `Not valid JSON: ${(e as Error).message}`; return; }
    busy = true;
    const r = await runsApi.sendEvent(run.namespace, event);
    busy = false;
    if (!r.ok) { message = r.error ?? 'Send failed'; return; }
    sending = false;
    const n = r.data?.woke?.length ?? 0;
    notice = n ? `Event sent — woke ${n} run${n > 1 ? 's' : ''}.` : 'Event sent — no waiting run matched it.';
    await load();
  }

  // ── Approvals: answer the question a run waits on ──
  async function answer(requestId: string, allow: boolean) {
    busy = true; message = ''; notice = '';
    const ok = await answerApproval(requestId, allow, 'once');
    busy = false;
    if (!ok) { message = 'Could not answer — only the approver named in the request can.'; return; }
    notice = allow ? 'Allowed once — the run resumes.' : 'Refused — the run resumes and is told.';
    await load();
  }

  async function cancel() {
    busy = true;
    const r = await runsApi.cancel(runId, 'cancelled from the console');
    busy = false;
    if (!r.ok) message = r.error ?? 'Stop failed';
    await load();
  }
  async function rerun(debug: boolean, fromSeq?: number) {
    busy = true; message = '';
    const r = await runsApi.rerun(runId, { debug, ...(fromSeq ? { from_seq: fromSeq } : {}) });
    busy = false;
    if (!r.ok) { message = r.error ?? 'Re-run failed'; return; }
    onrerun?.(r.data.run_id);
  }

  // Item mode: the run's actions go in the shared action bar.
  $effect(() => {
    if (!item || framed || !run) return;
    const can = !live && run.can_steer;
    return setDetailActions({
      onopenineditor: onopen,
      extra: [
        ...(live && run.can_steer ? [{ label: 'Stop', run: cancel }] : []),
        ...(can ? [{ label: 'Re-run in debug', run: () => rerun(true) }, { label: 'Re-run', run: () => rerun(false) }] : []),
      ],
    });
  });
  const initials = (t: string) => t.replace(/[^a-z0-9]+/gi, ' ').trim().split(' ').filter(Boolean).slice(0, 2).map((w) => w[0]).join('').toUpperCase() || 'RN';

  // ── Timeline ──
  // From the moment the run was queued (`created_at`) to its end (or now):
  // the wait for a worker, the run's own life — loading, pauses, timers,
  // each (re)claim by a worker — and every step on the same axis.
  type Seg = { from: number; to: number; kind: 'queued' | 'requeued' | 'loading' | 'paused' | 'timer'; title: string };
  const ms = (t?: string | null) => (t ? Date.parse(t) : NaN);
  /// When a journal record happened: a step's own start (its recorded end less
  /// its duration — pure steps are written in batches, later), else when written.
  const happened = (e: Entry) => (e.record?.at ? ms(e.record.at) - (e.record.duration_us ?? 0) / 1000 : ms(e.at));
  const tl = $derived.by(() => {
    if (!run) return null;
    const created = ms(run.created_at);
    if (isNaN(created)) return null;
    const started = ms(run.started_at);
    const lastAt = entries.length ? ms(entries[entries.length - 1].at) : created;
    const end = !isNaN(ms(run.completed_at)) ? ms(run.completed_at) : live ? Date.now() : Math.max(lastAt, isNaN(started) ? created : started);
    const queue: Seg[] = [], life: Seg[] = [];
    const claims: { at: number; epoch: number }[] = [];
    queue.push({ from: created, to: isNaN(started) ? end : started, kind: 'queued', title: isNaN(started) ? 'Queued — waiting for a worker' : 'Queued until a worker picked it up' });
    let pausedAt: number | null = null, pausedWhy = '';
    let resumeAt: number | null = null, resumeEpoch = 0;
    let epoch = 0;
    const listening = new Map<string, number>(), asking = new Map<string, number>();
    for (const e of entries) {
      const at = happened(e), r = e.record ?? {};
      if (e.epoch > epoch) {
        if (epoch) claims.push({ at, epoch: e.epoch });
        epoch = e.epoch;
      }
      if (resumeAt !== null && e.epoch > resumeEpoch) {
        queue.push({ from: resumeAt, to: at, kind: 'requeued', title: 'Resumed — waiting for a worker to pick it up again' });
        resumeAt = null;
      }
      if (r.kind === 'run_started' && !isNaN(started) && at > started) life.push({ from: started, to: at, kind: 'loading', title: 'Worker loading the flow' });
      if (r.kind === 'paused') { pausedAt = at; pausedWhy = r.reason ?? `paused at ${r.step_key}`; }
      if ((r.kind === 'debug_command' || r.kind === 'resumed') && pausedAt !== null) {
        life.push({ from: pausedAt, to: at, kind: 'paused', title: pausedWhy });
        pausedAt = null;
        resumeAt = at; resumeEpoch = e.epoch;
      }
      if (r.kind === 'wait_for_events') listening.set(r.wait_id, at);
      if (r.kind === 'approval_requested') asking.set(r.wait_id, at);
      if (r.kind === 'approval_decided' && asking.has(r.wait_id)) {
        life.push({ from: asking.get(r.wait_id)!, to: at, kind: 'paused', title: `Waiting for approval — answered by ${r.by}` });
        asking.delete(r.wait_id);
      }
      if (r.kind === 'event_received' && listening.has(r.wait_id)) {
        life.push({ from: listening.get(r.wait_id)!, to: at, kind: 'timer', title: `${r.wait_id} listened until its event came` });
        listening.delete(r.wait_id);
      }
      if (r.kind === 'timer_set' && r.fire_at) life.push({ from: at, to: Math.min(ms(r.fire_at), end), kind: 'timer', title: `Timer ${r.wait_id} until ${new Date(r.fire_at).toLocaleTimeString()}` });
    }
    for (const [k, from] of listening) life.push({ from, to: end, kind: 'timer', title: `${k} listening for events` });
    for (const [, from] of asking) life.push({ from, to: end, kind: 'paused', title: 'Waiting for approval' });
    if (pausedAt !== null) life.push({ from: pausedAt, to: end, kind: 'paused', title: pausedWhy });
    if (resumeAt !== null) queue.push({ from: resumeAt, to: end, kind: 'requeued', title: 'Resumed — waiting for a worker to pick it up again' });
    const total = Math.max(end - created, 1);
    const sum = (k: Seg['kind'][], list: Seg[]) => list.filter((x) => k.includes(x.kind)).reduce((n, x) => n + Math.max(x.to - x.from, 0), 0);
    const queued = sum(['queued', 'requeued'], queue), pausedMs = sum(['paused'], life);
    return { start: created, end, total, started, queue, life, claims, queued, paused: pausedMs, running: Math.max(total - queued - pausedMs, 0) };
  });
  function pos(from: number, to: number) {
    if (!tl) return 'display:none';
    return `left:${((from - tl.start) / tl.total) * 100}%;width:${Math.max(((to - from) / tl.total) * 100, 0.4)}%`;
  }
  function bar(s: Step) {
    if (!tl || !s.started_at) return 'display:none';
    const a = ms(s.started_at);
    const b = s.completed_at ? ms(s.completed_at) : s.fire_at ? Math.min(ms(s.fire_at), tl.end) : tl.end;
    return pos(a, b);
  }
  const msDur = (n: number) => dur(Math.round(n * 1000));
  const ticks = [0, 0.25, 0.5, 0.75, 1];

  // ── Formatting ──
  const pretty = (v: any) => (v === undefined || v === null ? '—' : typeof v === 'string' ? v : JSON.stringify(v, null, 2));
  function dur(us?: number | null): string {
    if (us === undefined || us === null) return '';
    const ms = us / 1000;
    return ms < 1 ? `${us} µs` : ms < 1000 ? `${ms.toFixed(ms < 10 ? 1 : 0)} ms` : `${(ms / 1000).toFixed(2)} s`;
  }
  const when = (t?: string) => (t ? new Date(t).toLocaleTimeString() : '');
  const PHASE: Record<string, string> = { completed: 'Completed', faulted: 'Failed', cancelled: 'Cancelled', running: 'Running', pending: 'Queued', waiting: 'Waiting', suspended: 'Paused' };
  /// The A3T base's failure classes (system/flow_system_error_handler).
  const CLASS: Record<string, { label: string; hint: string }> = {
    policy: { label: 'Not allowed', hint: 'a policy refused it' },
    transient: { label: 'Unavailable', hint: 'something it called was down or timed out — a retry may work' },
    limit: { label: 'Limit', hint: 'a quota, rate or size limit was hit' },
    input: { label: 'Input', hint: 'it was given input it could not use' },
    bug: { label: 'Bug', hint: 'the flow or the engine did something wrong' },
  };
  const stamp = (t?: string) => (t ? new Date(t).toLocaleString(undefined, { month: 'short', day: 'numeric', hour: '2-digit', minute: '2-digit', second: '2-digit' }) : '—');
  const phaseClass = (p: string) => (p === 'completed' ? 'ok' : p === 'faulted' ? 'err' : p === 'cancelled' ? 'mut' : p === 'suspended' ? 'warn' : 'live');
</script>

{#if error}
  <div class="rv-msg">{error}</div>
{:else if !run}
  <!-- the header bar shows loading -->
{:else if framed}
  {@render framedView()}
{:else}
  <div class="rv" class:item>
    {#if item}
      <div class="rv-ihead">
        <span class="rv-icon">{initials(title)}</span>
        <div class="rv-it">
          <div class="rv-crumb">{crumb}</div>
          <div class="rv-name">{title} <span class="rv-st {phaseClass(run.phase)}">{run.phase}</span>{#if run.debug}<span class="rv-st">debug</span>{/if}</div>
          <div class="rv-meta">
            <span class="mono" title="Run id">{run.run_id}</span>
            {#if run.duration_us}<span>· {dur(run.duration_us)}</span>{/if}
            {#if run.principal_id}<span>· by {run.principal_id}</span>{/if}
            {#if run.resume_count > 0}<span title="Resumed after an interruption, a timer or a pause">· resumed {run.resume_count}×</span>{/if}
            {#if run.current_step}<span>· at <span class="mono">{run.current_step}</span></span>{/if}
          </div>
        </div>
        <DetailActionsBar />
      </div>
    {:else}
    <div class="rv-head">
      <span class="pill {phaseClass(run.phase)}">{run.phase}</span>
      {#if run.debug}<span class="pill mut">debug</span>{/if}
      <span class="mono dim" title="Run id">{run.run_id}</span>
      {#if run.duration_us}<span>{dur(run.duration_us)}</span>{/if}
      {#if run.principal_id}<span class="dim">by {run.principal_id}</span>{/if}
      {#if run.resume_count > 0}<span class="dim" title="Resumed after an interruption, a timer or a pause">resumed {run.resume_count}×</span>{/if}
      {#if run.current_step}<span class="dim">at <span class="mono">{run.current_step}</span></span>{/if}
      <span class="grow"></span>
      {#if live && run.can_steer}<button class="lnk" disabled={busy} onclick={cancel} title="Stop this run">■ Stop</button>{/if}
      {#if !live && run.can_steer}
        <span class="tip-wrap">
          <button class="ib" disabled={busy} onclick={() => rerun(false)} aria-label="Re-run">
            <svg width="14" height="14" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><path d="M13.5 8a5.5 5.5 0 1 1-1.6-3.9"/><path d="M13.5 2.5v3h-3"/></svg>
          </button>
          <span class="tip" aria-hidden="true">Re-run</span>
        </span>
        <span class="tip-wrap">
          <button class="ib" disabled={busy} onclick={() => rerun(true)} aria-label="Re-run in debug">
            <svg width="14" height="14" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><rect x="5" y="5" width="6" height="8.5" rx="3"/><path d="M6.5 5a1.5 1.5 0 0 1 3 0M8 8v5.5M2.5 7.5H5M11 7.5h2.5M2.5 11.5H5M11 11.5h2.5M3.5 3.5 5.5 5.5M12.5 3.5 10.5 5.5"/></svg>
          </button>
          <span class="tip" aria-hidden="true">Re-run in debug</span>
        </span>
      {/if}
    </div>
    {/if}

    {@render banners()}

    <div class="rv-tabs">
      {#each [['steps', `Steps (${rows.length})`], ['timeline', 'Timeline'], ['journal', `Journal (${entries.length})`], ['checkpoints', 'Checkpoints'], ['io', 'Input / Output']] as [k, label] (k)}
        <button class:on={tab === k} onclick={() => (tab = k as typeof tab)}>{label}</button>
      {/each}
    </div>

    {#if tab === 'steps'}{@render stepsBody()}
    {:else if tab === 'timeline'}{@render timelineBody()}
    {:else if tab === 'journal'}{@render journalBody()}
    {:else if tab === 'checkpoints'}{@render checkpointsBody()}
    {:else}{@render ioBody()}{/if}
  </div>
{/if}

{#snippet stepsBody()}
      <div class="rv-steps">
        <div class="rv-list">
          {#if rows.length === 0}<div class="rv-empty">No steps recorded{run.durability === 'ephemeral' ? ' — an ephemeral flow keeps no journal' : ''}.</div>{/if}
          {#each rows as s (s.step_key)}
            <button class="rv-step" class:sel={selected === s.step_key} class:paused={paused?.step_key === s.step_key}
              onclick={() => { selected = s.step_key; detailTab = s.status === 'failed' ? 'output' : detailTab; }}>
              <span class="ic {s.status}">{icon(s)}</span>
              <span class="mono key">{s.step_key}</span>
              {#if s.edited}<span title="Input edited while paused">✎</span>{/if}
              {#if (s.attempts ?? 0) > 1}<span class="badge" title="Attempts">×{s.attempts}</span>{/if}
              <span class="dur">{dur(s.duration_us)}</span>
            </button>
          {/each}
        </div>
        <div class="rv-detail">
          {#if selected}
            {@const rec = last(selected)}
            <div class="rv-dtabs">
              {#each ['input', 'output', 'context', 'records'] as k (k)}
                <button class:on={detailTab === k} onclick={() => (detailTab = k as typeof detailTab)}>{k}</button>
              {/each}
              {#if rec?.class}<span class="dim small">class: {rec.class}</span>{/if}
            </div>
            {#if detailTab === 'input'}
              <pre>{pretty(rec?.input ?? (paused?.step_key === selected ? paused?.input : undefined))}</pre>
            {:else if detailTab === 'output'}
              {#if rec?.kind === 'step_failed'}<pre class="err">{pretty(rec.error)}</pre>{:else}<pre>{pretty(rec?.output)}</pre>{/if}
            {:else if detailTab === 'context'}
              <pre>{pretty(contextAfter(selected))}</pre>
            {:else}
              <table class="rv-table"><tbody>
                {#each recordsOf(selected) as e (e.seq)}
                  <tr><td class="mono">{e.seq}</td><td>{when(e.at)}</td><td>{e.record.kind}</td><td class="dim">{e.record.attempt !== undefined ? `attempt ${e.record.attempt}` : ''} {e.record.idem_key ? `· idem ${e.record.idem_key.slice(-12)}` : ''}</td></tr>
                {/each}
              </tbody></table>
            {/if}
          {:else}
            <div class="rv-empty">Select a step to see its input, output and context.</div>
          {/if}
        </div>
      </div>
{/snippet}
{#snippet timelineBody()}
      <div class="rv-timeline">
        {#if tl}
          <div class="tl-sum">
            <span>Queued <b>{new Date(tl.start).toLocaleTimeString()}</b></span>
            <span title="Waiting for a worker, including after each resume">waited <b>{msDur(tl.queued)}</b></span>
            <span>running <b>{msDur(tl.running)}</b></span>
            {#if tl.paused}<span>paused <b>{msDur(tl.paused)}</b></span>{/if}
            <span>total <b>{msDur(tl.total)}</b>{live ? ' so far' : ''}</span>
          </div>
          <div class="tl-row axis">
            <span></span>
            <span class="tl-axis">{#each ticks as f}<span style="left:{f * 100}%">{f === 0 ? '0' : `+${msDur(tl.total * f)}`}</span>{/each}</span>
          </div>
          <div class="tl-row lane">
            <span class="tl-key">Queue</span>
            <span class="tl-track">
              {#each tl.queue as g}<span class="tl-bar seg-{g.kind}" style={pos(g.from, g.to)} title="{g.title} · {msDur(g.to - g.from)}"></span>{/each}
            </span>
          </div>
          <div class="tl-row lane">
            <span class="tl-key">Run</span>
            <span class="tl-track">
              {#if !isNaN(tl.started)}<span class="tl-bar seg-active" style={pos(tl.started, tl.end)} title="Owned by a worker from {new Date(tl.started).toLocaleTimeString()}"></span>{/if}
              {#each tl.life as g}<span class="tl-bar seg-{g.kind}" style={pos(g.from, g.to)} title="{g.title} · {msDur(g.to - g.from)}"></span>{/each}
              {#if !isNaN(tl.started)}<span class="tl-mark" style="left:{((tl.started - tl.start) / tl.total) * 100}%" title="Picked up by a worker at {new Date(tl.started).toLocaleTimeString()}"></span>{/if}
              {#each tl.claims as c}<span class="tl-mark" style="left:{((c.at - tl.start) / tl.total) * 100}%" title="Claimed again by a worker (epoch {c.epoch}) at {new Date(c.at).toLocaleTimeString()}"></span>{/each}
            </span>
          </div>
          {#each steps as s (s.step_key)}
            <div class="tl-row">
              <span class="mono tl-key" title={s.step_key}>{s.step_key}</span>
              <span class="tl-track"><span class="tl-bar {s.status}" style={bar(s)} title="{s.step_key} · {dur(s.duration_us)}{s.started_at ? ` · started +${msDur(ms(s.started_at) - tl.start)}` : ''}"></span></span>
            </div>
          {/each}
          <div class="tl-legend">
            <span class="lg seg-queued">queued</span><span class="lg seg-active">running</span><span class="lg seg-loading">loading</span>
            <span class="lg seg-paused">paused</span><span class="lg seg-timer">timer</span><span class="lg mark">worker claim</span>
          </div>
        {:else}
          <div class="rv-empty">Nothing to show yet.</div>
        {/if}
      </div>
{/snippet}
{#snippet journalBody()}
      <table class="rv-table" class:tbl={framed}><tbody>
        <tr><th>seq</th><th>time</th><th>record</th><th>step</th><th>owner epoch</th></tr>
        {#each entries as e (e.seq)}
          <tr><td class="mono">{e.seq}</td><td>{when(e.at)}</td><td>{e.record.kind}</td><td class="mono">{e.record.step_key ?? e.record.wait_id ?? ''}</td><td>{e.epoch}</td></tr>
        {/each}
      </tbody></table>
{/snippet}
{#snippet checkpointsBody()}
      <p class="dim small">Each completed step is a point the run can resume from. Re-run from one starts a new debug run: the steps up to it replay from this journal, the steps after it run again.</p>
      <table class="rv-table" class:tbl={framed}><tbody>
        {#each entries.filter((e) => e.record.kind === 'step_completed') as e (e.seq)}
          <tr>
            <td class="mono">{e.seq}</td><td class="mono">{e.record.step_key}</td><td>{when(e.at)}</td>
            <td class="dim">{e.record.context === null ? 'context unchanged' : `context ${JSON.stringify(e.record.context).length} B`}</td>
            <td>{#if run.can_steer && !live}<button class={framed ? 'btn2' : 'lnk'} disabled={busy} onclick={() => rerun(true, e.seq)}>Re-run from here</button>{/if}</td>
          </tr>
        {/each}
      </tbody></table>
{/snippet}
{#snippet ioBody()}
      <div class="rv-io">
        <div><h4>Input</h4><pre>{pretty(run.input)}</pre></div>
        <div><h4>Output</h4><pre>{pretty(run.output)}</pre></div>
      </div>
{/snippet}
{#snippet banners()}
    {#if paused}
      <div class="rv-pause">
        <div>
          ⏸ Paused {paused.when === 'before' ? 'before' : paused.when === 'after' ? 'after' : 'on an error in'}
          <span class="mono">{paused.step_key}</span> <span class="dim">({paused.reason})</span>
        </div>
        {#if run.can_steer && run.debug}
          <div class="rv-ctl">
            <button class="btn spec sm" disabled={busy} onclick={() => command('continue')}>▶ Continue</button>
            <button class="btn ghost sm" disabled={busy} onclick={() => command('step_over')} title="Run this step, pause before the next">⤼ Step over</button>
            <button class="btn ghost sm" disabled={busy} onclick={() => command('step_into')} title="Pause before the very next step, inside this one if it has steps">⤵ Step into</button>
            <button class="btn ghost sm" disabled={busy} onclick={() => command('step_out')} title="Finish the enclosing block, pause after it">⤴ Step out</button>
            {#if flowSteps.length}
              <select bind:value={runTo} title="Run until this step">
                <option value="">Run to…</option>
                {#each flowSteps as k (k)}<option value={k}>{k}</option>{/each}
              </select>
              <button class="btn ghost sm" disabled={busy || !runTo} onclick={() => command('run_to', runTo)}>⇥</button>
            {/if}
            {#if paused.when === 'before'}<button class="btn ghost sm" disabled={busy} onclick={openEdit}>✎ Edit input</button>{/if}
            <button class="btn ghost sm" disabled={busy} onclick={() => command('stop')}>■ Stop</button>
          </div>
        {/if}
      </div>
    {/if}
    {#if run.phase === 'waiting'}
      {@const ear = (run.steps ?? []).find((s: any) => s.status === 'waiting' && s.listening)}
      {@const ask = (run.steps ?? []).find((s: any) => s.status === 'waiting' && s.asking)}
      {#if ask}
        <div class="rv-note">⏳ Waiting for {ask.asking.approver ?? 'a person'} to approve <code>{ask.asking.tool}</code>{ask.asking.reason ? ` — ${ask.asking.reason}` : ''}. No worker is held; the answer (here, in the chat, or <code>POST /inputs/{ask.asking.request_id}</code>) resumes it on any worker{ask.asking.expires_at ? `; unanswered, it is refused at ${when(ask.asking.expires_at)}` : ''}.
          <button class="lnk" disabled={busy} onclick={() => answer(ask.asking.request_id, true)}>Allow once</button>
          <button class="lnk" disabled={busy} onclick={() => answer(ask.asking.request_id, false)}>Refuse</button></div>
      {:else if ear}
        <div class="rv-note" title={JSON.stringify(ear.listening.filters)}>⏳ Listening at {ear.step_key} for {ear.listening.mode === 'all' ? 'all of' : ear.listening.mode === 'any' ? 'any of' : ''} {(ear.listening.filters ?? []).map((f: any) => f.type).join(', ') || 'any event'} — no worker is held; a matching event (an <code>emit</code>, or <code>send_event</code>) wakes it on any worker.
          {#if run.can_steer && !sending}<button class="lnk" disabled={busy} onclick={() => openSend(ear.listening)}>Send event…</button>{/if}</div>
        {#if sending}
          <div class="rv-edit">
            <span class="rv-note">CloudEvent sent in project {run.namespace} (<code>type</code> required; <code>id</code>, <code>time</code>, <code>source</code> are filled in):</span>
            <textarea rows="8" bind:value={eventText}></textarea>
            <div class="rv-row">
              <button class="btn sm" onclick={() => (sending = false)}>Cancel</button>
              <button class="btn spec sm" disabled={busy} onclick={sendEvent}>Send</button>
            </div>
          </div>
        {/if}
      {:else}
        <div class="rv-note">⏳ Waiting on a timer — no worker is held; it wakes on time, on any worker.</div>
      {/if}
    {/if}
    {#if message}<div class="rv-msg">{message}</div>{/if}
    {#if notice && !message}<div class="rv-msg ok">{notice}</div>{/if}
    {#if run.error && !framed}<div class="rv-msg">{run.error}</div>{/if}

    {#if editing}
      <div class="rv-edit">
        <div class="dim">Input for <span class="mono">{paused?.step_key}</span> — the step runs with this instead. The change is recorded with your name.</div>
        <textarea rows="8" spellcheck="false" bind:value={editText}></textarea>
        <div class="rv-row">
          <button class="btn ghost sm" onclick={() => (editing = false)}>Cancel</button>
          <button class="btn spec sm" disabled={busy} onclick={applyEdit}>Apply</button>
        </div>
      </div>
    {/if}
{/snippet}

{#snippet framedView()}
  <div class="rv framed">
    {@render banners()}
    {#if view === 'overview'}
      {@const cls = failure?.class ? CLASS[failure.class] : null}
      <div class="card">
        <div class="card-head"><h3>1 · {PHASE[run.phase] ?? run.phase}</h3><span>{run.debug ? 'a debug run · ' : ''}{run.durability === 'ephemeral' ? 'ephemeral — no journal kept' : 'durable — every step journaled'}</span></div>
        <div class="tiles">
          <div class="tile" class:bad={run.phase === 'faulted'}><span class="v">{PHASE[run.phase] ?? run.phase}</span><span class="k">{run.current_step ? `at ${run.current_step}` : 'status'}</span></div>
          <div class="tile"><span class="v">{run.duration_us ? dur(run.duration_us) : live ? 'running' : '—'}</span><span class="k">duration</span></div>
          <div class="tile"><span class="v">{rows.length}</span><span class="k">steps recorded</span></div>
          <div class="tile"><span class="v">{run.resume_count ?? 0}</span><span class="k">times resumed</span></div>
        </div>
      </div>
      {#if run.error || failure}
        <div class="card">
          <div class="card-head"><h3>2 · Why it failed</h3><span>{cls ? `${cls.label} — ${cls.hint}` : 'the engine’s error'}</span></div>
          {#if failure}
            <ul class="rows wide">
              <li><span class="lbl">Class</span><span class="val">{cls?.label ?? failure.class} · <span class="mono">{failure.code}</span>{failure.status ? ` · HTTP ${failure.status}` : ''}</span><span></span></li>
              {#if failure.step}<li><span class="lbl">Step</span><span class="val mono">{failure.step}</span><button class="btn2" onclick={() => { selected = failure.step; tab = 'steps'; onview?.('steps'); }}>See</button></li>{/if}
              <li><span class="lbl">The caller saw</span><span class="val">{failure.user_message || '—'}</span><span></span></li>
            </ul>
          {/if}
          <pre class="code out">{failure?.detail || run.error}</pre>
        </div>
      {/if}
      <div class="card">
        <div class="card-head"><h3>{run.error || failure ? 3 : 2} · What ran</h3><span>who started it, from where, and when</span></div>
        <ul class="rows wide">
          <li><span class="lbl">Flow</span><span class="val mono">{run.namespace}/{run.flow}{run.version ? ` · ${run.version}` : ''}</span>{#if onopen}<button class="btn2" onclick={onopen}>Open</button>{:else}<span></span>{/if}</li>
          {#if startedBy?.kind === 'trigger'}
            <li><span class="lbl">Started by</span><span class="val">{startedBy.cron ? 'Schedule of trigger' : 'Trigger'} <span class="mono">{triggerNames[startedBy.id] || startedBy.id}</span>{triggerNames[startedBy.id] === '' ? ' (deleted)' : ''}{run.principal_id ? ` · as ${run.principal_id}` : ''}</span>
              {#if triggerNames[startedBy.id]}<button class="btn2" onclick={() => navigateToDetail('triggers', 'Triggers', startedBy.id)}>Open</button>{:else}<span></span>{/if}</li>
          {:else if startedBy?.kind === 'agent'}
            <li><span class="lbl">Started by</span><span class="val">A turn of agent <span class="mono">{startedBy.id}</span>{run.principal_id ? ` · for ${run.principal_id}` : ''}</span>
              <button class="btn2" onclick={() => navigateToDetail('agents', 'Agents', startedBy.id)}>Open</button></li>
          {:else}
            <li><span class="lbl">Started by</span><span class="val">{run.trigger ?? '—'}{run.principal_id ? ` · ${run.principal_id}` : ''}</span><span></span></li>
          {/if}
          {#if run.parent_run_id}<li><span class="lbl">Part of</span><span class="val mono">{run.parent_run_id}</span>{#if onparent}<button class="btn2" onclick={() => onparent?.(run.parent_run_id)}>Open</button>{:else}<span></span>{/if}</li>{/if}
          <li><span class="lbl">Queued</span><span class="val">{stamp(run.created_at)}</span><span></span></li>
          <li><span class="lbl">Started</span><span class="val">{stamp(run.started_at)}</span><span></span></li>
          <li><span class="lbl">Finished</span><span class="val">{run.completed_at ? stamp(run.completed_at) : live ? 'still running' : '—'}</span><span></span></li>
          <li><span class="lbl">Run id</span><span class="val mono">{run.run_id}</span><span></span></li>
        </ul>
      </div>
      {#if run.can_steer}
        <div class="card">
          <div class="card-head"><h3>{run.error || failure ? 4 : 3} · Run it again</h3><span>a new run with the same input</span></div>
          <div class="acts">
            {#if live}<button class="btn2 danger" disabled={busy} onclick={cancel}>Stop</button>
            {:else}
              <button class="btn2 primary" disabled={busy} onclick={() => rerun(false)}>Re-run</button>
              <button class="btn2" disabled={busy} onclick={() => rerun(true)}>Re-run in debug</button>
              <button class="btn2 quiet" onclick={() => { tab = 'checkpoints'; onview?.('checkpoints'); }}>From a step…</button>
            {/if}
          </div>
        </div>
      {/if}
    {:else if view === 'steps'}
      <div class="card"><div class="card-head"><h3>1 · Steps</h3><span>{rows.length} recorded · pick one for its input, output and context</span></div>{@render stepsBody()}</div>
    {:else if view === 'timeline'}
      <div class="card"><div class="card-head"><h3>1 · Timeline</h3><span>queued, running, paused and each step on one axis</span></div>{@render timelineBody()}</div>
    {:else if view === 'journal'}
      <div class="card"><div class="card-head"><h3>1 · Journal</h3><span>{entries.length} records — what the engine wrote, in order</span></div>{@render journalBody()}</div>
    {:else if view === 'checkpoints'}
      <div class="card"><div class="card-head"><h3>1 · Resume points</h3><span>each completed step</span></div>{@render checkpointsBody()}</div>
    {:else}
      <div class="card"><div class="card-head"><h3>1 · Input</h3><span>what it was started with</span></div><pre class="code out">{pretty(run.input)}</pre></div>
      <div class="card"><div class="card-head"><h3>2 · Output</h3><span>{run.phase === 'completed' ? 'what it returned' : 'nothing returned yet'}</span></div><pre class="code out">{pretty(run.output)}</pre></div>
    {/if}
  </div>
{/snippet}
<style>
  .rv { display: flex; flex-direction: column; gap: 8px; font-size: 12.5px; }
  /* Item mode: the standard item header, then the tabs and body padded below. */
  .rv.framed { gap: 14px; font-size: 13px; }
  .rv.framed .rv-steps { min-height: 220px; }
  .rv.framed .rv-list { max-height: 520px; }
  .rv.framed pre { max-height: 480px; }
  .rv.item { gap: 0; height: 100%; min-height: 0; overflow-y: auto; }
  .rv.item > :not(.rv-ihead) { margin-left: 20px; margin-right: 20px; }
  .rv.item > :not(.rv-ihead) + * { margin-top: 8px; }
  .rv-ihead { display: flex; align-items: center; gap: 12px; padding: 12px 18px; background: var(--bg-surface); border-bottom: 1px solid var(--border-light, #f0ece3); margin-bottom: 10px; }
  .rv-icon { flex: 0 0 36px; width: 36px; height: 36px; border-radius: 9px; display: inline-flex; align-items: center; justify-content: center; font: 600 12px var(--font-sans); color: #fff; background: #55514a; }
  .rv-it { flex: 1 1 auto; min-width: 0; display: flex; flex-direction: column; gap: 1px; }
  .rv-crumb { font: 400 11.5px var(--font-mono); color: var(--text-3); }
  .rv-name { font: 400 18px/1.25 var(--font-serif); color: var(--text-1); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .rv-st { font: 500 10.5px var(--font-mono); letter-spacing: 0.06em; text-transform: uppercase; color: var(--text-3); margin-left: 6px; vertical-align: middle; }
  .rv-st.ok { color: var(--green, #1a7158); }
  .rv-st.err { color: var(--err, #d15858); }
  .rv-st.live { color: var(--spec); }
  .rv-meta { display: flex; flex-wrap: wrap; gap: 4px; font-size: 12px; color: var(--text-3); }
  .rv-head { display: flex; align-items: center; gap: 10px; flex-wrap: wrap; color: var(--text-2); }
  .grow { flex: 1; }
  .dim { color: var(--text-3); }
  .small { font-size: 11.5px; }
  .mono { font-family: var(--font-mono); font-size: 11.5px; }
  .pill { padding: 2px 8px; border-radius: 10px; font-size: 11px; background: var(--bg-subtle); color: var(--text-2); }
  .pill.ok { background: var(--ok-bg, #e7f6ec); color: var(--ok, #15803d); }
  .pill.err { background: var(--err-bg, #fde8e8); color: var(--err, #b42318); }
  .pill.live { background: var(--spec-bg); color: var(--spec); }
  .pill.warn { background: var(--warn-bg, #fef3c7); color: var(--warn, #b45309); }
  .lnk { border: 0; background: transparent; color: var(--spec); cursor: pointer; font-size: 12px; }
  .lnk:disabled { opacity: 0.5; }
  .ib { width: 24px; height: 24px; display: inline-flex; align-items: center; justify-content: center; border: 0; border-radius: var(--r-sm); background: transparent; color: var(--spec); cursor: pointer; }
  .ib:hover:not(:disabled) { background: var(--spec-bg); }
  .ib:disabled { opacity: 0.5; cursor: default; }
  /* Icon labels, as SlidePanel's: shown under the button on hover or focus, and
     right-aligned so the last one stays inside the panel. */
  .tip-wrap { position: relative; display: inline-flex; }
  .tip { position: absolute; top: calc(100% + 6px); right: 0; z-index: 20;
         padding: 4px 10px; border-radius: 5px; background: var(--text-1); color: var(--bg-page);
         font-size: 11px; font-weight: 500; white-space: nowrap; pointer-events: none;
         opacity: 0; transition: opacity 0.15s 0.3s; }
  .tip::before { content: ''; position: absolute; bottom: 100%; right: 8px; border: 4px solid transparent; border-bottom-color: var(--text-1); }
  .tip-wrap:hover .tip, .tip-wrap:focus-within .tip { opacity: 1; }
  .rv-pause { display: flex; flex-direction: column; gap: 6px; padding: 8px 10px; border: 1px solid var(--warn, #b45309); border-radius: var(--r-sm); background: var(--warn-bg, #fef3c7); }
  .rv-ctl { display: flex; gap: 6px; flex-wrap: wrap; align-items: center; }
  .rv-ctl select { padding: 3px 6px; border: 1px solid var(--border); border-radius: var(--r-sm); font-size: 12px; background: var(--bg-surface); }
  .rv-note { color: var(--text-3); }
  .rv-msg.ok { background: var(--ok-bg, #e7f6ec); color: var(--ok, #15803d); }
  .rv-msg { padding: 6px 10px; border-radius: var(--r-sm); background: var(--err-bg, #fde8e8); color: var(--err, #b42318); white-space: pre-wrap; }
  .rv-edit { display: flex; flex-direction: column; gap: 6px; padding: 8px; border: 1px solid var(--border); border-radius: var(--r-sm); }
  .rv-edit textarea { font-family: var(--font-mono); font-size: 12px; padding: 6px; border: 1px solid var(--border); border-radius: var(--r-sm); background: var(--bg-page); color: var(--text-1); }
  .rv-row { display: flex; justify-content: flex-end; gap: 6px; }
  .rv-tabs, .rv-dtabs { display: flex; gap: 4px; align-items: center; border-bottom: 1px solid var(--border); }
  .rv-tabs button, .rv-dtabs button { border: 0; background: transparent; padding: 5px 8px; font-size: 12px; color: var(--text-3); cursor: pointer; border-bottom: 2px solid transparent; }
  .rv-tabs button.on, .rv-dtabs button.on { color: var(--text-1); border-bottom-color: var(--spec); }
  .rv-steps { display: grid; grid-template-columns: minmax(200px, 38%) 1fr; gap: 10px; min-height: 160px; }
  @media (max-width: 800px) { .rv-steps { grid-template-columns: 1fr; } }
  .rv-list { display: flex; flex-direction: column; overflow-y: auto; max-height: 340px; }
  .rv-step { display: flex; align-items: center; gap: 6px; padding: 4px 6px; border: 0; background: transparent; cursor: pointer; text-align: left; border-radius: var(--r-sm); color: var(--text-2); }
  .rv-step:hover, .rv-step.sel { background: var(--bg-subtle); }
  .rv-step.paused { outline: 1px solid var(--warn, #b45309); }
  .key { flex: 1; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .ic { width: 14px; text-align: center; }
  .ic.completed { color: var(--ok, #15803d); } .ic.failed { color: var(--err, #b42318); } .ic.started { color: var(--spec); }
  .badge { font-size: 10px; padding: 0 4px; border-radius: 6px; background: var(--bg-subtle); }
  .dur { font-family: var(--font-mono); font-size: 11px; color: var(--text-3); }
  .rv-detail { min-width: 0; display: flex; flex-direction: column; gap: 6px; }
  pre { margin: 0; padding: 8px 10px; background: var(--bg-page); border: 1px solid var(--border); border-radius: var(--r-sm); font-family: var(--font-mono); font-size: 11.5px; white-space: pre-wrap; word-break: break-word; max-height: 300px; overflow: auto; }
  pre.err { color: var(--err, #b42318); }
  .rv-empty { padding: 10px; color: var(--text-4); }
  .rv-table { width: 100%; border-collapse: collapse; font-size: 12px; }
  .rv-table th { text-align: left; font-weight: 500; color: var(--text-3); padding: 3px 6px; border-bottom: 1px solid var(--border); }
  .rv-table td { padding: 3px 6px; border-bottom: 1px solid var(--border); }
  .rv-timeline { display: flex; flex-direction: column; gap: 3px; }
  .tl-row { display: grid; grid-template-columns: minmax(120px, 30%) 1fr; gap: 8px; align-items: center; }
  .tl-sum { display: flex; flex-wrap: wrap; gap: 14px; font-size: 11.5px; color: var(--text-3); padding: 2px 0 6px; }
  .tl-sum b { color: var(--text-1); font-weight: 600; font-family: var(--font-mono); }
  .tl-row.axis { height: 14px; }
  .tl-axis { position: relative; height: 14px; font-size: 10px; color: var(--text-4); font-family: var(--font-mono); }
  .tl-axis span { position: absolute; top: 0; transform: translateX(-50%); white-space: nowrap; }
  .tl-axis span:first-child { transform: none; } .tl-axis span:last-child { transform: translateX(-100%); }
  .tl-row.lane .tl-key { font-size: 11px; font-weight: 600; color: var(--text-2); text-transform: uppercase; letter-spacing: 0.04em; }
  .tl-row.lane .tl-track { height: 14px; }
  .tl-bar.seg-queued, .lg.seg-queued::before { background: repeating-linear-gradient(45deg, var(--text-5, #c8c3b6), var(--text-5, #c8c3b6) 3px, transparent 3px, transparent 6px); }
  .tl-bar.seg-requeued { background: repeating-linear-gradient(45deg, var(--text-4), var(--text-4) 3px, transparent 3px, transparent 6px); }
  .tl-bar.seg-active, .lg.seg-active::before { background: rgba(85, 48, 168, 0.22); }
  .tl-bar.seg-loading, .lg.seg-loading::before { background: var(--text-4); }
  .tl-bar.seg-paused, .lg.seg-paused::before { background: var(--warn, #b45309); }
  .tl-bar.seg-timer, .lg.seg-timer::before { background: repeating-linear-gradient(45deg, var(--warn, #b45309), var(--warn, #b45309) 3px, transparent 3px, transparent 6px); }
  .tl-mark { position: absolute; top: -3px; bottom: -3px; width: 2px; margin-left: -1px; background: var(--spec, #5530a8); }
  .tl-legend { display: flex; flex-wrap: wrap; gap: 12px; margin-top: 6px; font-size: 10.5px; color: var(--text-4); }
  .tl-legend .lg::before { content: ''; display: inline-block; width: 14px; height: 8px; border-radius: 2px; margin-right: 4px; vertical-align: -1px; }
  .tl-legend .lg.mark::before { width: 2px; background: var(--spec, #5530a8); }
  .tl-key { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .tl-track { position: relative; height: 12px; background: var(--bg-subtle); border-radius: 3px; }
  .tl-bar { position: absolute; top: 0; bottom: 0; border-radius: 3px; background: var(--ok, #15803d); }
  .tl-bar.failed { background: var(--err, #b42318); } .tl-bar.started { background: var(--spec); }
  .tl-bar.waiting { background: repeating-linear-gradient(45deg, var(--text-4), var(--text-4) 3px, transparent 3px, transparent 6px); }
  .rv-io { display: grid; grid-template-columns: 1fr 1fr; gap: 10px; }
  h4 { margin: 4px 0; font-size: 11px; text-transform: uppercase; letter-spacing: 0.04em; color: var(--text-3); }
</style>
