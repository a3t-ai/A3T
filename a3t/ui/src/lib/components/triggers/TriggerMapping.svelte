<script lang="ts" module>
  import type { MappingRecord } from './mapperApi';
  import type { SourceKind } from '$lib/core/triggers/triggerSpec';
  /// What every mapping field of one trigger shares: its records, its samples,
  /// and how to save the trigger (approving writes the mapping into it).
  export interface MapCtx {
    triggerId: string | null;
    trigger: string;
    source: SourceKind;
    /// Sample bodies, JSON text each.
    samples: string[];
    records: MappingRecord[];
    reload: () => Promise<void>;
    /// Save the trigger if it is new; its id, or null with the reason shown.
    ensureSaved: () => Promise<string | null>;
    /// Save the trigger as it is in the form.
    persist: () => Promise<boolean>;
  }
</script>

<script lang="ts">
  /// One mapping of a trigger target (a flow's input, an agent's message or
  /// conversation), and how it came to be: written by hand or proposed by
  /// agent_a3t_mapper, checked on sample events by FlowR, approved by a person.
  /// Only what is approved is written into the trigger; only that runs.
  import { tick } from 'svelte';
  import { BODY_OF } from '$lib/core/triggers/triggerSpec';
  import { MAPPER, askMapper, checkMapping, putRecord, standing, nextVersion, wrapJq, type Field, type Proposal, type CheckResult } from './mapperApi';

  let { value = $bindable(), field, label, hint, placeholder, target, flowRef, targetName, ctx }: {
    value: string | null | undefined;
    field: Field;
    label: string;
    hint: string;
    placeholder: string;
    /// The record key of the target: flow:<ns>/<name> or agent:<id>.
    target: string;
    /// The flow, for its input schema.
    flowRef?: string;
    /// The target as a person reads it.
    targetName: string;
    ctx: MapCtx;
  } = $props();

  const st = $derived(standing(ctx.records, target, field, value ?? ''));
  const history = $derived(ctx.records.filter((r) => r.target === target && r.field === field));

  type Panel = 'none' | 'ask' | 'asking' | 'proposal' | 'history';
  let panel = $state<Panel>('none');
  let wants = $state('');
  let proposal = $state<Proposal | null>(null);
  let check = $state<CheckResult | null>(null);
  /// What `check` was run on: the proposal, or the text in the field.
  let checkedText = $state('');
  let busy = $state('');
  let err = $state('');
  let rejecting = $state(false);
  let reason = $state('');
  let ctl: AbortController | null = null;

  function samples(): unknown[] | null {
    try { return ctx.samples.filter((s) => s.trim()).map((s) => JSON.parse(s)); } catch { err = 'A sample is not JSON: fix it in Sample events above.'; return null; }
  }

  async function runCheck(text: string): Promise<CheckResult | null> {
    const s = samples(); if (!s) return null;
    if (!s.length) { err = 'Add a sample event above to check against.'; return null; }
    const r = await checkMapping({ expr: wrapJq(text), samples: s, field, target: flowRef, trigger: ctx.trigger || 'trigger', source: ctx.source });
    if (!r.ok) { err = r.error; return null; }
    checkedText = text; check = r.check; return r.check;
  }

  async function checkCurrent() {
    err = ''; busy = 'check'; proposal = null; panel = 'none';
    await runCheck(value ?? '');
    busy = '';
  }

  async function ask() {
    err = ''; check = null; proposal = null;
    const s = samples(); if (!s) return;
    if (!s.length) { err = 'Add at least one sample event above: the mapper writes from real examples.'; return; }
    panel = 'asking'; ctl = new AbortController();
    // The flow's input schema, read by FlowR (the check returns it).
    let schema: unknown = field === 'input' ? null : { type: 'string' };
    if (field === 'input' && flowRef) { const c = await checkMapping({ expr: '${ .body }', samples: s.slice(0, 1), field, target: flowRef, trigger: ctx.trigger, source: ctx.source }); if (c.ok) schema = c.check.schema; }
    const r = await askMapper({
      task: 'Write the mapping for this trigger target.',
      trigger: { name: ctx.trigger, source: ctx.source },
      event: `{ body, message, source, trigger, time } — body is ${BODY_OF[ctx.source]}`,
      target: field === 'input' ? { kind: 'flow', flow: flowRef, field, input_schema: schema ?? 'none declared' } : { kind: 'agent', agent: targetName, field, type: 'text' },
      wants: wants.trim() || undefined,
      samples: s,
      current: value?.trim() || undefined,
      approved_before: history.filter((h) => h.event === 'approved').slice(0, 3).map((h) => ({ version: h.version, jq: h.jq })),
    }, ctl.signal);
    ctl = null;
    if (!r.ok) { err = r.error; panel = 'ask'; return; }
    proposal = r.proposal; panel = 'proposal';
    await runCheck(r.proposal.jq);
    if (ctx.triggerId) {
      await putRecord({ event: 'proposed', trigger_id: ctx.triggerId, target, field, jq: r.proposal.jq, version: 0, source: 'agent',
        explanation: r.proposal.explanation, assumptions: r.proposal.assumptions, drops: r.proposal.drops, samples: s.length, wants: wants.trim() || undefined });
      await ctx.reload();
    }
  }

  /// Approve a checked mapping: write it into the trigger, save, keep the record.
  async function approve(text: string, from: 'agent' | 'hand', p: Proposal | null) {
    err = '';
    if (!check?.all_ok || checkedText !== text) { err = 'Check it on the samples first: only a mapping that passes every sample can be approved.'; return; }
    busy = 'approve';
    const id = await ctx.ensureSaved();
    if (!id) { busy = ''; return; }
    value = wrapJq(text);
    await tick();
    if (!(await ctx.persist())) { busy = ''; err = 'The trigger was not saved, so the mapping is not in use.'; return; }
    const w = await putRecord({ event: 'approved', trigger_id: id, target, field, jq: wrapJq(text), version: nextVersion(ctx.records, target, field), source: from,
      explanation: p?.explanation, assumptions: p?.assumptions, drops: p?.drops, samples: ctx.samples.filter((x) => x.trim()).length });
    if (!w.ok) err = `In use, but its approval was not recorded: ${w.error}`;
    await ctx.reload();
    busy = ''; proposal = null; check = null; panel = 'none';
  }

  async function reject() {
    if (!proposal) return;
    busy = 'reject';
    if (ctx.triggerId) {
      await putRecord({ event: 'rejected', trigger_id: ctx.triggerId, target, field, jq: proposal.jq, version: 0, source: 'agent', reason: reason.trim() || undefined, samples: ctx.samples.length });
      await ctx.reload();
    }
    busy = ''; proposal = null; check = null; rejecting = false; reason = ''; panel = 'none';
  }

  /// Take the proposal into the field to change it; it then needs a check and an approval of its own.
  function editFirst() { if (!proposal) return; value = proposal.jq; proposal = null; check = null; panel = 'none'; }

  const when = (iso: string) => { const d = new Date(iso); return isNaN(+d) ? iso : d.toLocaleString(undefined, { day: 'numeric', month: 'short', hour: '2-digit', minute: '2-digit' }); };
</script>

<div class="map">
  <div class="map-head">
    <span class="map-l">{label}</span><small>{hint}</small><span class="grow"></span>
    {#if st.kind === 'approved'}<span class="tag prod" title="{st.rec.jq}">Approved v{st.rec.version} · {st.rec.by} · {when(st.rec.at)}</span>
    {:else if st.kind === 'changed'}<span class="tag sample" title="Approved: {st.rec.jq}">Changed since approval v{st.rec.version}</span>
    {:else if st.kind === 'proposed'}<span class="tag dev">Proposal waiting for review</span>
    {:else if st.kind === 'hand'}<span class="tag">Hand-written · not approved</span>{/if}
  </div>
  <textarea rows="2" bind:value {placeholder} spellcheck="false"></textarea>
  <div class="acts">
    <button class="btn2" onclick={() => { panel = panel === 'ask' ? 'none' : 'ask'; err = ''; }} disabled={panel === 'asking'}>Ask {MAPPER}</button>
    {#if value?.trim()}<button class="btn2" onclick={checkCurrent} disabled={!!busy}>{busy === 'check' ? 'Checking…' : 'Check on samples'}</button>{/if}
    {#if history.length}<button class="btn2 quiet" onclick={() => (panel = panel === 'history' ? 'none' : 'history')}>History ({history.length})</button>{/if}
  </div>
  {#if err}<div class="note bad">{err}</div>{/if}

  {#if panel === 'ask'}
    <div class="note">
      <label class="fld">What should it get? <small>optional — e.g. "the order id and the amount in cents"</small>
        <input type="text" bind:value={wants} placeholder="Leave empty to let it read the target's schema" /></label>
      <p class="dim">{MAPPER} writes the jq from the sample events above and {field === 'input' ? "the flow's input schema" : 'the target'}. <b>The samples are sent to its model</b> — leave out personal data you don't want there. You review and approve what it writes; nothing changes until you do.</p>
      <div class="acts"><button class="btn2 primary" onclick={ask}>Write the mapping</button><button class="btn2 quiet" onclick={() => (panel = 'none')}>Cancel</button></div>
    </div>
  {:else if panel === 'asking'}
    <div class="note"><span class="dot warn"></span> {MAPPER} is writing it — about 20 seconds. <button class="link" onclick={() => ctl?.abort()}>Stop</button></div>
  {/if}

  {#if proposal}
    <div class="review">
      <div class="card-head"><h3>Proposed by {MAPPER}</h3><span>review it, then approve, change or reject it</span></div>
      <pre class="code">{proposal.jq}</pre>
      {#if proposal.explanation}<p>{proposal.explanation}</p>{/if}
      {#if proposal.assumptions.length}<div><b class="sm">It assumes</b><ul class="notes">{#each proposal.assumptions as a (a)}<li>{a}</li>{/each}</ul></div>{/if}
      {#if proposal.drops.length}<div><b class="sm">It leaves out</b> <span class="chips">{#each proposal.drops as d (d)}<span class="tag">{d}</span>{/each}</span></div>{/if}
      {@render checks()}
      <div class="acts">
        <button class="btn2 primary" disabled={!check?.all_ok || !!busy} onclick={() => approve(proposal!.jq, 'agent', proposal)}>{busy === 'approve' ? 'Approving…' : 'Approve and use'}</button>
        <button class="btn2" onclick={editFirst}>Change it first</button>
        <button class="btn2 quiet" onclick={() => (rejecting = !rejecting)}>Reject</button>
        {#if check && !check.all_ok}<span class="err">It fails on a sample: change it or ask again.</span>{/if}
      </div>
      {#if rejecting}
        <div class="row"><input class="grow rs" type="text" bind:value={reason} placeholder="Why (kept with the record)" />
          <button class="btn2" disabled={busy === 'reject'} onclick={reject}>Reject it</button></div>
      {/if}
    </div>
  {:else if check}
    <div class="review">
      <div class="card-head"><h3>Checked on {check.results.length} sample{check.results.length === 1 ? '' : 's'}</h3><span>{check.all_ok ? 'it passes every one' : 'it fails on at least one'}</span></div>
      {@render checks()}
      <div class="acts">
        {#if check.all_ok && checkedText === (value ?? '') && st.kind !== 'approved'}
          <button class="btn2 primary" disabled={!!busy} onclick={() => approve(value ?? '', 'hand', null)}>{busy === 'approve' ? 'Approving…' : 'Approve this mapping'}</button>
        {/if}
        <button class="btn2 quiet" onclick={() => (check = null)}>Close</button>
      </div>
    </div>
  {/if}

  {#if panel === 'history'}
    <ul class="tl">
      {#each history as h (h.key)}
        <li class="ev" class:err={h.event === 'rejected'}>
          <span class="ic">{h.event === 'approved' ? `v${h.version}` : h.event === 'rejected' ? '✕' : '?'}</span>
          <div>
            <div class="t">{h.event === 'approved' ? `Approved by ${h.by}` : h.event === 'rejected' ? `Rejected by ${h.by}` : `Proposed by ${MAPPER}, asked by ${h.by}`}{h.source === 'hand' && h.event === 'approved' ? ' (hand-written)' : ''}</div>
            <div class="m mono">{h.jq}</div>
            {#if h.reason}<div class="m">“{h.reason}”</div>{/if}
            {#if h.event === 'approved' && h.jq !== value}<button class="link" onclick={() => { value = h.jq; panel = 'none'; check = null; }}>Use this version</button>{/if}
          </div>
          <time>{when(h.at)}</time>
        </li>
      {/each}
    </ul>
  {/if}
</div>

{#snippet checks()}
  {#if check}
    <ul class="checks">
      {#each check.results as r, i (i)}
        <li class:bad={!r.ok}>
          <span class="dot" class:ok={r.ok} class:err={!r.ok}></span>
          <div class="cr">
            <b class="sm">Sample {i + 1}</b>
            {#if r.error}<span class="err">{r.error}</span>
            {:else}<pre class="code out">{typeof r.value === 'string' ? r.value : JSON.stringify(r.value, null, 2)}</pre>{/if}
            {#each r.schema_errors ?? [] as e (e)}<span class="err">Schema: {e}</span>{/each}
          </div>
        </li>
      {/each}
    </ul>
    <p class="dim">{check.schema ? "Each result is checked against the flow's input schema." : check.schema_note ?? (field === 'input' ? '' : 'The result is the text the agent gets.')}</p>
  {/if}
{/snippet}

<style>
  .map { display: flex; flex-direction: column; gap: 8px; }
  .map-head { display: flex; align-items: baseline; gap: 8px; flex-wrap: wrap; }
  .map-l { font-size: 12.5px; font-weight: 500; color: var(--text-2); }
  .map-head small { font-size: 11.5px; color: var(--text-4); }
  textarea { width: 100%; box-sizing: border-box; padding: 8px 11px; border: 1px solid var(--border); border-radius: 8px; background: var(--bg-surface); color: var(--text-1); font: 12.5px/1.55 var(--font-mono); resize: vertical; outline: none; }
  textarea:focus { border-color: var(--spec); }
  .review { display: flex; flex-direction: column; gap: 10px; padding: 12px 14px; border: 1px solid var(--border); border-radius: 10px; background: var(--bg-page); }
  .review p { margin: 0; font-size: 13px; color: var(--text-2); line-height: 1.5; }
  .sm { font-size: 12px; font-weight: 600; color: var(--text-2); margin-right: 6px; }
  .checks { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 8px; }
  .checks li { display: grid; grid-template-columns: 14px 1fr; gap: 8px; align-items: start; }
  .checks .dot { margin-top: 5px; }
  .cr { display: flex; flex-direction: column; gap: 4px; min-width: 0; }
  .rs { padding: 7px 10px; border: 1px solid var(--border); border-radius: 7px; background: var(--bg-surface); font: 13px var(--font-sans); }
  .note .fld { margin-bottom: 8px; }
  .note .dim { margin: 0 0 8px; }
</style>
