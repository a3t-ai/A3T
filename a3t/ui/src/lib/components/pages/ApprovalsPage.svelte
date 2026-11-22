<script lang="ts">
  /// Approvals (GOV-7): every question that waits on a person — an agent's
  /// `needs_approval` rule, or a call past its grants — from any chat, flow,
  /// trigger or API call. Items lists what waits (yours to answer; SuperAdmin and
  /// SecurityAdmin see everyone's); the one picked opens here with the agent, the
  /// tool and its arguments, and is answered here. "Answered" lists your answers
  /// from the decision log, where every question, answer and expiry is kept.
  import { untrack } from 'svelte';
  import SlidePanel from '$lib/components/shared/SlidePanel.svelte';
  import PropertiesTree from '$lib/components/shared/PropertiesTree.svelte';
  import WizardHead from '$lib/components/shared/WizardHead.svelte';
  import '$lib/components/agent/agent.css';
  import { approvalsApi, audit, type InboxItem } from '$lib/services/api';
  import { getActivePanel, navigateTo, navigateToDetail } from '$lib/stores/panels.svelte.ts';
  import { bumpContext } from '$lib/stores/apps.svelte.ts';
  import { getProfile } from '$lib/stores/context.svelte.ts';

  let sel = $state<string | null>(null);
  let item = $state<InboxItem | null>(null);
  let gone = $state(false);
  let err = $state('');
  let busy = $state('');
  let done = $state('');
  let note = $state('');
  const me = $derived(getProfile()?.user_id ?? '');

  let shown = '';
  $effect(() => {
    const id = getActivePanel()?.props?.selectedId as string | undefined;
    untrack(() => {
      if (id && id !== shown) { shown = id; sel = id; done = ''; note = ''; err = ''; void load(); }
      else if (!id) { shown = ''; sel = null; }
    });
  });

  async function load() {
    if (!sel) return;
    if (sel === 'answered') { await loadAnswered(); return; }
    const r = await approvalsApi.pending();
    if (!r.ok) { err = r.error ?? 'The approvals could not be read.'; return; }
    item = r.items.find((a) => a.id === sel) ?? null;
    gone = !item;
  }

  // ── Answer ──
  const mine = $derived(!!item && (!item.approver || item.approver === me));
  async function answer(allow: boolean, scope: 'once' | 'session' = 'once') {
    if (!item) return;
    busy = allow ? scope : 'no'; err = '';
    const r = await approvalsApi.answer(item.id, allow, scope, note.trim() || undefined);
    busy = '';
    if (!r.ok) { err = r.status === 404 ? 'It is no longer waiting, or it is not yours to answer.' : (r.error ?? 'The answer was not taken.'); return; }
    done = allow ? (scope === 'session' ? 'Approved for the rest of this conversation.' : 'Approved, for this one call.') : 'Refused.';
    bumpContext();
  }

  // ── Answered (from the decision log) ──
  interface Rec { ts_ms: number; principal_id: string; action: string; resource_id: string; reason?: string | null; project?: string | null; decision: string }
  let answered = $state<Rec[]>([]);
  let asked = $state<Rec[]>([]);
  let scope = $state('');
  async function loadAnswered() {
    err = '';
    const [a, q] = await Promise.all([
      audit.query({ action: 'approval:answer', principal: me || undefined, limit: 100 }),
      audit.query({ action: 'approval:request', limit: 100 }),
    ]);
    if (!a.ok) { err = a.error ?? 'The decision log could not be read.'; return; }
    answered = (a.data?.records ?? []) as Rec[];
    asked = q.ok ? ((q.data?.records ?? []) as Rec[]) : [];
    scope = a.data?.scope ?? '';
  }
  const askedFor = (id: string) => asked.find((r) => r.resource_id === id);
  const said = (r: Rec) => /allow=true/.test(r.reason ?? '') ? 'approved' : /allow=false/.test(r.reason ?? '') ? 'refused' : 'answered';

  const close = () => navigateTo('approvals', 'Approvals');
  const when = (v?: string | number | null) => (v ? new Date(v).toLocaleString(undefined, { month: 'short', day: 'numeric', hour: '2-digit', minute: '2-digit' }) : '—');
  const left = (iso?: string | null) => {
    if (!iso) return '';
    const s = Math.round((Date.parse(iso) - Date.now()) / 1000);
    if (s <= 0) return 'expired';
    return s < 3600 ? `${Math.ceil(s / 60)} min left` : s < 172800 ? `${Math.round(s / 3600)} h left` : `${Math.round(s / 86400)} days left`;
  };
  const steps = $derived(sel === 'answered'
    ? [{ id: 'answered', label: 'Answered', icon: '<path d="M3 8l3 3 7-7"/>', hint: 'Your answers, newest first, from the decision log' }]
    : [{ id: 'ask', label: 'The question', icon: '<circle cx="8" cy="8" r="6.5"/><path d="M6.2 6a1.9 1.9 0 113 1.5c-.7.4-1.2.9-1.2 1.7M8 11.5v.01"/>', hint: 'What the agent wants to do, for whom, and why it asks' }]);
</script>

<SlidePanel open={!!sel} onclose={close}>
  {#if sel}
    <div class="panel-layout">
      <PropertiesTree icon={sel === 'answered' ? '✓' : '?'} color="#b45309" name={sel === 'answered' ? 'Answered' : (item?.tool ?? 'Approval')}
        meta={sel === 'answered' ? 'from the decision log' : item ? `${item.agent || 'an agent'}${item.project ? ` · ${item.project}` : ''}` : ''} nodes={[]} {steps} activeNode={steps[0].id} onselect={() => {}} />
      <div class="panel-col"><div class="panel-content"><div class="pf">
        <WizardHead step={steps[0]} />
        <section class="agt">
          {#if err}<div class="note bad">{err}</div>{/if}

          {#if sel === 'answered'}
            <div class="card">
              <div class="card-head"><h3>1 · Your answers</h3><span>{answered.length} · {scope === 'yours' ? 'yours' : scope}</span><span class="grow"></span>
                <button class="btn2 quiet rm" onclick={() => navigateToDetail('logs', 'Decision log', 'ask')}>Decision log →</button></div>
              {#if !answered.length}<p class="dim">No answers yet.</p>
              {:else}
                <ul class="rows wide">
                  {#each answered as r (r.ts_ms + r.resource_id)}
                    {@const q = askedFor(r.resource_id)}
                    <li><span class="lbl">{when(r.ts_ms)}</span>
                      <span class="val"><b>{said(r)}</b> · {q?.reason?.split(' · ')[0] ?? r.resource_id}{q ? ` · ${q.principal_id.replace(/^agent:/, '')}` : ''}{r.project ? ` · ${r.project}` : ''}</span><span></span></li>
                  {/each}
                </ul>
              {/if}
            </div>
          {:else if gone}
            <div class="note">It is no longer waiting: answered, expired, or not yours to see. The decision log has what happened.</div>
            <div class="acts"><button class="btn2" onclick={() => navigateToDetail('approvals', 'Approvals', 'answered')}>Answered</button></div>
          {:else if !item}
            <p class="dim">Reading…</p>
          {:else}
            <div class="card">
              <div class="card-head"><h3>1 · What it wants to do</h3><span>{item.durable ? 'waits durably' : 'waits in one FlowR'} · {left(item.expiresAt)}</span></div>
              <ul class="rows wide">
                <li><span class="lbl">Agent</span><span class="val mono">{item.agent || '—'}</span><span></span></li>
                <li><span class="lbl">Tool</span><span class="val mono">{item.server ? `${item.server} / ` : ''}{item.tool}</span><span></span></li>
                <li><span class="lbl">For</span><span class="val">{item.onBehalfOf ?? '—'}{item.owner && item.owner !== item.onBehalfOf ? ` · owned by ${item.owner}` : ''}</span><span></span></li>
                {#if item.project}<li><span class="lbl">Project</span><span class="val mono">{item.project}</span><span></span></li>{/if}
                <li><span class="lbl">Why it asks</span><span class="val">{item.reason}</span><span></span></li>
                {#if item.rule}<li><span class="lbl">Rule</span><span class="val mono">{item.rule}</span><span></span></li>{/if}
                <li><span class="lbl">Asked</span><span class="val">{when(item.createdAt)} · answer by {when(item.expiresAt)}</span><span></span></li>
                <li><span class="lbl">Answered by</span><span class="val mono">{item.approver ?? '—'}</span><span></span></li>
              </ul>
            </div>
            <div class="card">
              <div class="card-head"><h3>2 · With these arguments</h3><span>exactly what it would send</span></div>
              <pre class="mono args">{JSON.stringify(item.arguments ?? {}, null, 2)}</pre>
            </div>
            <div class="card">
              <div class="card-head"><h3>3 · Your answer</h3><span>{mine ? 'written to the decision log' : `waiting for ${item.approver}`}</span></div>
              {#if done}
                <div class="note">{done} The agent carries on{item.session ? ' in its conversation' : ''}.</div>
              {:else if mine}
                <label class="fld full">Note <small>optional, kept with your answer</small><input bind:value={note} maxlength="200" placeholder="Checked the PR first" /></label>
                <div class="acts">
                  <button class="btn2 primary" disabled={!!busy} onclick={() => answer(true, 'once')}>{busy === 'once' ? 'Approving…' : 'Approve this call'}</button>
                  {#if item.scopes.includes('session')}<button class="btn2" disabled={!!busy} onclick={() => answer(true, 'session')}>{busy === 'session' ? 'Approving…' : 'Approve for this conversation'}</button>{/if}
                  <button class="btn2 danger" disabled={!!busy} onclick={() => answer(false)}>{busy === 'no' ? 'Refusing…' : 'Refuse'}</button>
                </div>
                {#if !item.scopes.includes('session')}<p class="dim">A rule asks a person for every call, so an answer covers this call only.</p>{/if}
              {:else}
                <p class="dim">Only {item.approver} can answer it. You see it as a security admin.</p>
              {/if}
            </div>
          {/if}
        </section>
      </div></div></div>
    </div>
  {/if}
</SlidePanel>

<style>
  .args { margin: 0; padding: 10px 12px; background: var(--bg-sunken, var(--bg-page)); border: 1px solid var(--border); border-radius: 6px; font-size: 12px; white-space: pre-wrap; word-break: break-word; max-height: 360px; overflow: auto; }
</style>
