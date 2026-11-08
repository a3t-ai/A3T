<script lang="ts">
  // The A3T error log (UX ST-1): every failure the A3T base handled, one row
  // each, written by system/a3t-error-handler and read through a3t-error-log.
  // Admin and SuperAdmin depth: class, flow, step and the raw detail beside the
  // message the user was shown.
  import PageBanner from '$lib/components/shared/PageBanner.svelte';
  import { execFlow } from '$lib/services/api';
  import { onMount } from 'svelte';

  interface ErrorRow {
    id: string;
    created_at: string;
    class: string;
    code: string;
    status: string;
    title: string;
    detail: string;
    flow: string;
    step: string;
    run_id: string;
    principal: string;
    user_message: string;
  }

  const CLASSES = ['policy', 'transient', 'limit', 'input', 'bug'] as const;
  const CLASS_LABEL: Record<string, string> = {
    policy: 'Not allowed', transient: 'Unavailable', limit: 'Limit', input: 'Input', bug: 'Bug',
  };

  let error = $state('');
  let rows = $state<ErrorRow[]>([]);
  let counts = $state<Record<string, number>>({});
  let cls = $state('');
  let open = $state<string | null>(null);

  async function load() {
    error = '';
    const res = await execFlow('a3t-error-log', 'a3t', { action: 'list', ...(cls ? { class: cls } : {}), limit: 200 }, { silent: true });
    if (!res.ok) { error = text(res.error) || 'Could not read the error log.'; return; }
    if (res.data?.ok === false) { error = text(res.data.error) || 'Refused'; return; }
    rows = res.data?.errors ?? [];
    if (!cls) counts = res.data?.counts ?? {};
  }

  /// An error can arrive as a string or as an object with a message.
  function text(e: unknown): string {
    if (!e) return '';
    if (typeof e === 'string') return e;
    const o = e as { message?: unknown; error?: unknown; detail?: unknown };
    for (const v of [o.message, o.error, o.detail]) if (typeof v === 'string' && v) return v;
    return '';
  }

  onMount(load);

  function pick(c: string) { cls = cls === c ? '' : c; load(); }

  function fmtDate(iso: string): string {
    const d = new Date(iso);
    if (isNaN(d.getTime())) return iso || '—';
    return d.toLocaleString(undefined, { month: 'short', day: 'numeric', hour: '2-digit', minute: '2-digit', second: '2-digit' });
  }
</script>

<div class="er-page">
  <PageBanner headline="Every failure, once." highlightWord="once"
    subtitle="Errors handled by the A3T base, newest first."
    description="Each row is one failure: its class, where it happened, the raw detail, and the message the user saw. Written by system/a3t-error-handler."
    accent="spec" />

  <div class="er-toolbar">
    <div class="er-classes" role="group" aria-label="Filter by class">
      {#each CLASSES as c}
        <button class="er-chip" class:on={cls === c} data-class={c} onclick={() => pick(c)}>
          {CLASS_LABEL[c]} <span class="n">{counts[c] ?? 0}</span>
        </button>
      {/each}
    </div>
    <span style="flex:1"></span>
    <button class="er-refresh" onclick={load}>Refresh</button>
  </div>

  {#if error}
    <div class="er-empty er-err">{error}</div>
  {:else if rows.length === 0}
    <div class="er-empty">{cls ? `No ${CLASS_LABEL[cls].toLowerCase()} errors.` : 'No errors logged.'}</div>
  {:else}
    <div class="er-table">
      <div class="er-head">
        <span>When</span><span>Class</span><span>Flow · step</span><span>What the user saw</span><span>Run</span>
      </div>
      {#each rows as r (r.id)}
        <button class="er-row" class:open={open === r.id} onclick={() => (open = open === r.id ? null : r.id)}>
          <span class="mono dim">{fmtDate(r.created_at)}</span>
          <span><span class="er-badge" data-class={r.class}>{CLASS_LABEL[r.class] ?? r.class}</span></span>
          <span class="mono">{r.flow}{r.step ? ` · ${r.step}` : ''}</span>
          <span class="msg">{r.user_message}</span>
          <span class="mono dim">{r.run_id.slice(0, 8)}</span>
        </button>
        {#if open === r.id}
          <div class="er-detail">
            <div><b>Code</b> <span class="mono">{r.code}</span> · <b>Status</b> <span class="mono">{r.status}</span> · <b>By</b> <span class="mono">{r.principal || '—'}</span></div>
            <div><b>{r.title}</b></div>
            <pre>{r.detail}</pre>
            <div class="mono dim">Run {r.run_id}</div>
          </div>
        {/if}
      {/each}
    </div>
  {/if}
</div>

<style>
  .er-page { height: 100%; overflow-y: auto; padding: 20px 24px; }
  .er-toolbar { display: flex; align-items: center; gap: 8px; margin-bottom: 12px; flex-wrap: wrap; }
  .er-classes { display: flex; gap: 6px; flex-wrap: wrap; }
  .er-chip { display: flex; align-items: center; gap: 6px; padding: 5px 10px; border: 1px solid var(--border); border-radius: 999px; background: var(--bg-surface); color: var(--text-2); font-size: 12px; cursor: pointer; }
  .er-chip .n { font-family: var(--font-mono); font-size: 11px; color: var(--text-4); }
  .er-chip.on { border-color: var(--spec); color: var(--spec); background: var(--spec-bg); }
  .er-refresh { padding: 6px 14px; border: 1px solid var(--border); border-radius: 6px; background: var(--bg-surface); color: var(--text-2); font-size: 12px; font-weight: 550; cursor: pointer; }
  .er-refresh:hover { border-color: var(--spec); color: var(--spec); }
  .er-empty { padding: 60px 20px; text-align: center; color: var(--text-4); font-size: 14px; }
  .er-err { color: var(--err, #dc2626); }
  .er-table { border: 1px solid var(--border); border-radius: var(--r-sm); overflow: hidden; }
  .er-head, .er-row { display: grid; grid-template-columns: 130px 110px 1.2fr 2fr 80px; gap: 12px; padding: 9px 14px; align-items: center; text-align: left; }
  .er-head { background: var(--bg-subtle); border-bottom: 1px solid var(--border); font-size: 10px; font-weight: 700; text-transform: uppercase; letter-spacing: 0.04em; color: var(--text-4); }
  .er-row { width: 100%; border: 0; border-bottom: 1px solid var(--border); background: none; font: inherit; font-size: 13px; color: var(--text-1); cursor: pointer; }
  .er-row:hover, .er-row.open { background: var(--bg-subtle); }
  .msg { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; color: var(--text-2); }
  .mono { font-family: var(--font-mono); font-size: 12px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .dim { color: var(--text-4); font-size: 11px; }
  .er-badge { font-family: var(--font-mono); font-size: 10px; text-transform: uppercase; padding: 1px 6px; border-radius: 3px; background: var(--bg-subtle); color: var(--text-3); }
  .er-badge[data-class='policy'], .er-chip.on[data-class='policy'] { color: var(--warn, #b45309); }
  .er-badge[data-class='bug'] { color: var(--err, #dc2626); }
  .er-detail { padding: 10px 14px 14px; border-bottom: 1px solid var(--border); background: var(--bg-subtle); font-size: 12px; color: var(--text-2); display: grid; gap: 6px; }
  .er-detail pre { margin: 0; white-space: pre-wrap; word-break: break-word; font-family: var(--font-mono); font-size: 11.5px; color: var(--text-1); }
  @media (max-width: 720px) {
    .er-head { display: none; }
    .er-row { grid-template-columns: 1fr; gap: 2px; }
  }
</style>
