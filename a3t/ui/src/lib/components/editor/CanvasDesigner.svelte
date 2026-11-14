<script lang="ts">
  /// A canvas: one flow's deployment — where it loads, how it runs, what it
  /// depends on — in the agents' and triggers' frame: header and steps
  /// (PropertiesTree), each step a page of numbered cards (.agt); the YAML in the
  /// flow designer (Graph | Code); Deploy and its live state last. Embedded (the
  /// chat pane) shows the sections one under another, saved through the caller.
  import type { WizardStep } from '$lib/components/shared/WizardSteps.svelte';
  import SlidePanel from '$lib/components/shared/SlidePanel.svelte';
  import PropertiesTree from '$lib/components/shared/PropertiesTree.svelte';
  import WizardHead from '$lib/components/shared/WizardHead.svelte';
  import WizardNav from '$lib/components/shared/WizardNav.svelte';
  import FlowEditorPane from '$lib/components/editor/FlowEditorPane.svelte';
  import '$lib/components/agent/agent.css';
  import '$lib/components/triggers/triggers.css';
  import { getProject } from '$lib/stores/context.svelte.ts';
  import { openInStudio, navigateTo } from '$lib/stores/panels.svelte.ts';
  import { trackActiveItem } from '$lib/stores/activeItem.svelte.ts';
  import { canvasApi, mcpCall, opsApi } from '$lib/services/api';
  import { untrack } from 'svelte';
  import * as YAML from 'yaml';
  import { fullSpec, specYaml, selectorText, textSelector, verdicts, placedOn, problems as problemsOf, around, servingLabel, triggerLabel,
    type CanvasSpec, type Around, type OnDown } from '$lib/components/canvas/canvasModel';

  let { open = false, canvas = null, onclose, onsaved, embedded = false, readonly = false, readonlyNote = '', onsavespec }:
    {
      open?: boolean; canvas?: any; onclose: () => void; onsaved?: () => void;
      /// Inline (the chat pane) instead of a slide-in panel.
      embedded?: boolean;
      /// View only — a shared preset is copied, not edited in place.
      readonly?: boolean;
      /// Why it is read-only, when not because it is a shared preset.
      readonlyNote?: string;
      /// Save through the caller (the chat pane: update_canvas, Cedar-gated).
      onsavespec?: (spec: any) => Promise<{ ok: boolean; error?: string }>;
    } = $props();

  // ── The canvas being edited ──
  /// A canvas made in this editor: kept here, not written into the prop, so a
  /// save (Deploy saves first) does not re-open the editor.
  let created = $state<any>(null);
  const cur = $derived(canvas ?? created);
  let name = $state('');
  let namespace = $state(getProject() || 'default');
  let spec = $state<CanvasSpec>(fullSpec());
  /// The runtime labels as typed ("class=gpu, tier=prod"); the spec follows it.
  let selText = $state('');
  $effect(() => { const t = selText; untrack(() => { spec.placement.runtimeSelector = textSelector(t); }); });
  let saved = $state('');
  const dirty = $derived(JSON.stringify(spec) !== saved);

  let flows = $state<string[]>([]);
  let runtimes = $state<any[]>([]);
  let resources = $state<{ ref: string; type: string }[]>([]);
  let ctx = $state<Around>({ flowExists: null, triggers: [], calls: [], live: null });
  let saving = $state(false);
  let msg = $state('');

  let step = $state('details');
  const ON_DOWN: { v: OnDown; t: string }[] = [
    { v: 'queue', t: 'hold new runs until it is back' },
    { v: 'cordon', t: 'take no new runs here; runs elsewhere go on' },
    { v: 'circuit-break', t: 'fail fast for a while, then try again' },
    { v: 'fail', t: 'fail the runs that need it' },
  ];

  $effect(() => { void canvas; if (open || embedded) untrack(() => void init()); });
  async function init() {
    msg = ''; step = 'details';
    const [rt, rs] = await Promise.all([canvasApi.runtimes(), canvasApi.resources()]);
    runtimes = rt.ok ? (rt.data?.runtimes ?? []) : [];
    const rows: any[] = rs.ok ? (Array.isArray(rs.data) ? rs.data : (rs.data?.resources ?? [])) : [];
    resources = rows.filter((r) => !['canvas', 'runtime', 'workflow', 'cedar'].includes(String(r.type ?? r.type_ref ?? '').toLowerCase()))
      .map((r) => ({ ref: `${r.namespace}/${r.name}`, type: String(r.type ?? r.type_ref ?? '') })).sort((a, b) => a.ref.localeCompare(b.ref));
    const seen = new Set<string>();
    for (const ns of ['a3t', 'default', 'system', namespace]) {
      const f = await mcpCall('list_flows', { namespace: ns });
      for (const w of (f.data?.workflows ?? (Array.isArray(f.data) ? f.data : []))) seen.add(`${w.namespace ?? ns}/${w.name}`);
    }
    flows = [...seen].sort();
    created = null;
    name = canvas?.name ?? ''; namespace = canvas?.namespace ?? (getProject() || 'default');
    spec = fullSpec(canvas?.config ?? {});
    selText = selectorText(spec.placement.runtimeSelector);
    saved = JSON.stringify(spec);
  }
  // What is around it follows the flow.
  $effect(() => { const f = spec.flowRef, a = spec.agentRef; if (open || embedded) void refreshAround(f, a); });
  async function refreshAround(f = spec.flowRef, a = spec.agentRef) { ctx = await around(f, a); }

  const placed = $derived(placedOn(spec, runtimes));
  const verdictList = $derived(verdicts(spec, runtimes));
  const pinKnown = $derived(!spec.placement.runtime || runtimes.some((r) => r.name === spec.placement.runtime));
  const problems = $derived(problemsOf(spec, { requireFlow: !embedded, flowExists: ctx.flowExists, runtimes, pinKnown }));
  const resourceKnown = (ref: string) => resources.some((r) => r.ref === ref);

  const steps = $derived.by((): WizardStep[] => [
    { id: 'details', icon: '<rect x="2" y="2" width="12" height="12" rx="2"/><path d="M5 6h6M5 9h4"/>', label: 'Details', hint: 'The canvas, the flow it deploys, and how it stands', ok: !!name.trim() && !!spec.flowRef && ctx.flowExists !== false, need: !name.trim() ? 'Name the canvas.' : 'Pick a flow that exists.' },
    { id: 'placement', icon: '<rect x="1" y="3" width="14" height="10" rx="1.5"/><path d="M1 7h14"/>', label: 'Placement', hint: 'Where it loads: a shared runtime pool or its own sandbox, and which runtime takes it', ok: !runtimes.length || !!placed, need: 'No runtime takes it yet.' },
    { id: 'lifecycle', icon: '<path d="M13.5 8A5.5 5.5 0 113 4.5"/><path d="M3 1.5v3h3"/>', label: 'Lifecycle', hint: 'How it runs: state, session, scaling and restarts', ok: spec.lifecycle.scaling.max >= spec.lifecycle.scaling.min, need: 'Scaling max must be at least min.' },
    { id: 'deps', icon: '<circle cx="4" cy="4" r="2"/><circle cx="12" cy="4" r="2"/><circle cx="8" cy="12" r="2"/><path d="M5.5 5.5L7 10.5M10.5 5.5L9 10.5"/>', label: 'Dependencies', hint: 'What feeds it, what it calls, what it depends on — and what happens when one is down', optional: true },
    { id: 'yaml', icon: '<path d="M5 3L1 8l4 5M11 3l4 5-4 5"/>', label: 'YAML', hint: 'The canvas as code: a graph of where it loads and what it depends on, and the YAML the Studio edits' },
    { id: 'deploy', icon: '<path d="M8 1v9M4 6l4 4 4-4"/><path d="M2 14h12"/>', label: 'Deploy', hint: 'Put it on its runtime, and see how it is running', optional: true },
  ]);

  // ── YAML: the form and the text are one canvas ──
  let yamlDraft = $state('');
  let yamlRev = $state(0);
  let yamlErr = $state('');
  $effect(() => { if (step === 'yaml') untrack(() => { yamlDraft = specYaml(spec); yamlRev++; yamlErr = ''; }); });
  function applyYaml(text: string) {
    yamlDraft = text;
    let d: any;
    try { d = YAML.parse(text); } catch (e) { yamlErr = `Not YAML: ${(e as Error).message}`; return; }
    if (!d || typeof d !== 'object') { yamlErr = 'Empty.'; return; }
    yamlErr = '';
    spec = fullSpec(d);
    selText = selectorText(spec.placement.runtimeSelector);
  }

  // The chat's context: this canvas, the step on screen, its spec as edited.
  let specRev = $state(0);
  if (!embedded) trackActiveItem(() => open ? {
    kind: 'canvas', namespace, name: name || 'new canvas', id: cur ? undefined : '(new)',
    section: step, text: JSON.stringify(spec, null, 2), rev: specRev,
    applyEdit: readonly ? undefined : (t: string) => applyYaml(t),
  } : null);

  // ── Dependencies ──
  let depPick = $state('');
  function addDep() {
    const r = depPick.trim();
    if (r && r.includes('/') && !spec.health.dependsOn.some((d) => d.resource === r)) spec.health.dependsOn = [...spec.health.dependsOn, { resource: r }];
    depPick = '';
  }
  const removeDep = (r: string) => (spec.health.dependsOn = spec.health.dependsOn.filter((d) => d.resource !== r));

  // ── Save ──
  async function save(stay = false): Promise<boolean> {
    if (!name.trim()) { msg = 'Name the canvas.'; step = 'details'; return false; }
    const blocking = problems.filter((p) => !/No runtime takes it/.test(p));
    if (blocking.length) { msg = blocking[0]; return false; }
    saving = true; msg = '';
    const body = JSON.parse(JSON.stringify(spec));
    let ok = false;
    if (onsavespec) {
      const r = await onsavespec(body); ok = r.ok; if (!ok) msg = r.error || 'Not saved.';
    } else {
      const res = cur ? await canvasApi.update(namespace, name.trim(), body) : await canvasApi.create(namespace, name.trim(), body);
      ok = res.ok; if (!ok) msg = res.error || `Not saved (HTTP ${res.status}).`;
      if (ok && !cur) created = { name: name.trim(), namespace, config: body };
    }
    saving = false;
    if (ok) { saved = JSON.stringify(spec); specRev++; onsaved?.(); if (!stay && !embedded) onclose(); }
    return ok;
  }

  // ── Deploy and live ──
  let deploying = $state(false);
  let deployMsg = $state('');
  let acting = $state('');
  async function deploy() {
    deployMsg = '';
    if (dirty || !cur) { if (!(await save(true))) return; }
    const [ns, nm] = spec.flowRef.split('/');
    if (!ns || !nm) { deployMsg = 'Pick the flow first.'; return; }
    deploying = true;
    const res = spec.agentRef ? await canvasApi.deployResource(namespace, name) : await canvasApi.deploy(ns, nm);
    deploying = false;
    if (res.ok && res.data?.deployed !== false) {
      deployMsg = res.data?.edition === 'ee' ? `Deployed: ${res.data?.placedRuntime ? `on ${res.data.placedRuntime}` : 'sandbox created'}.` : `Placed on ${res.data?.placedRuntime ?? placed?.name ?? 'its runtime'}.`;
      setTimeout(() => void refreshAround(), 1500);
    } else deployMsg = `Not deployed: ${res.data?.error ?? res.error ?? 'refused'}`;
  }
  const depId = $derived(ctx.live?.deployment?.id ?? ctx.live?.id ?? '');
  async function act(a: 'pause' | 'resume' | 'restart' | 'undeploy') {
    if (!depId) return;
    if (a === 'undeploy' && !confirm(`Undeploy ${spec.flowRef}? Its runs stop and it leaves ${ctx.live?.deployment?.runtime ?? 'its runtime'}.`)) return;
    acting = a;
    const r = await opsApi.deployment(depId, a);
    acting = '';
    const DONE = { pause: 'Paused.', resume: 'Resumed.', restart: 'Restarted.', undeploy: 'Undeployed.' };
    deployMsg = r.ok ? DONE[a] : `Not ${a}d: ${r.error ?? 'refused'}`;
    await refreshAround();
  }

  const liveState = $derived(ctx.live?.serving as string | undefined);
  const howRuns = $derived(`${spec.lifecycle.state} · ${spec.lifecycle.session} · ${spec.lifecycle.scaling.min}–${spec.lifecycle.scaling.max} ${spec.lifecycle.scaling.max === 1 ? 'instance' : 'instances'}`);
</script>

{#snippet details()}
  <div class="card">
    <div class="card-head"><h3>1 · Canvas</h3><span>its name, the project it belongs to, what it is for</span></div>
    <div class="fields">
      <label class="fld">Name<input bind:value={name} placeholder="orders-canvas" readonly={!!cur} /></label>
      <label class="fld">Project<input bind:value={namespace} readonly={!!cur} /></label>
      <label class="fld full">Description<input bind:value={spec.description} placeholder="What it deploys and why, e.g. refunds, kept warm for the shop" /></label>
    </div>
  </div>
  <div class="card">
    <div class="card-head"><h3>2 · The flow it deploys</h3><span>one canvas, one flow</span></div>
    <div class="fields">
      <label class="fld full">Flow
        <select bind:value={spec.flowRef}>
          <option value="">— pick the flow —</option>
          {#if spec.flowRef && !flows.includes(spec.flowRef)}<option value={spec.flowRef}>{spec.flowRef}</option>{/if}
          {#each flows as f (f)}<option value={f}>{f}</option>{/each}
        </select></label>
    </div>
    {#if spec.flowRef && ctx.flowExists === false}<div class="note bad">The flow <span class="mono">{spec.flowRef}</span> does not exist (any more). Pick the one it should deploy.</div>
    {:else if spec.flowRef}<div class="acts"><button class="link" onclick={() => openInStudio('flow', spec.flowRef.split('/')[0], spec.flowRef.split('/').slice(1).join('/'))}>Open the flow in Studio →</button>
      {#if spec.agentRef}<span class="dim">for agent <span class="mono">{spec.agentRef}</span></span>{/if}</div>{/if}
  </div>
{/snippet}

{#snippet status()}
  <div class="card">
    <div class="card-head"><h3>3 · Status</h3><span>where it lands, how it runs, what it needs, whether it is live</span></div>
    <ul class="rows">
      <li><span class="lbl">Runs on</span>
        <span class="val">{#if placed}<span class="dot ok"></span> {placed.name} <span class="dim">· {spec.placement.mode === 'isolated' ? 'its own sandbox' : 'shared pool'}</span>{:else if runtimes.length}<span class="dot err"></span> No runtime takes it{:else}—{/if}</span>
        <button class="btn2" onclick={() => (step = 'placement')}>Change</button>
        <span class="sub">{spec.placement.runtime ? `pinned to ${spec.placement.runtime}` : selText ? `runtimes labelled ${selText}` : 'the default runtime'}</span></li>
      <li><span class="lbl">How it runs</span><span class="val">{howRuns}</span>
        <button class="btn2" onclick={() => (step = 'lifecycle')}>Change</button>
        <span class="sub">restarts {spec.lifecycle.restart.policy}, {spec.lifecycle.restart.backoff} backoff, up to {spec.lifecycle.restart.maxRetries}</span></li>
      <li><span class="lbl">Depends on</span>
        <span class="val">{spec.health.dependsOn.length ? spec.health.dependsOn.map((d) => d.resource).join(', ') : ctx.calls.length ? `what its flow calls: ${ctx.calls.join(', ')}` : 'nothing declared'}</span>
        <button class="btn2" onclick={() => (step = 'deps')}>Change</button>
        <span class="sub">{ctx.triggers.length} trigger{ctx.triggers.length === 1 ? '' : 's'} feed it · if one is down: {spec.health.onDependencyDown}</span></li>
      <li><span class="lbl">Live</span>
        <span class="val"><span class="dot" class:ok={liveState === 'serving'} class:warn={liveState && liveState !== 'serving'}></span> {servingLabel(liveState)}{ctx.live?.deployment?.runtime ? ` on ${ctx.live.deployment.runtime}` : ''}</span>
        <button class="btn2" onclick={() => (step = 'deploy')}>{ctx.live ? 'Manage' : 'Deploy'}</button>
        {#if ctx.live?.deployment?.pauseReason}<span class="sub">“{ctx.live.deployment.pauseReason}”</span>{/if}</li>
    </ul>
  </div>
{/snippet}

{#snippet placement()}
  <div class="card">
    <div class="card-head"><h3>1 · Where it loads</h3><span>a shared runtime pool, or a sandbox of its own</span></div>
    <div class="seg src" role="tablist">
      <button role="tab" class:on={spec.placement.mode === 'shared'} aria-selected={spec.placement.mode === 'shared'} onclick={() => (spec.placement.mode = 'shared')}>Shared pool</button>
      <button role="tab" class:on={spec.placement.mode === 'isolated'} aria-selected={spec.placement.mode === 'isolated'} onclick={() => (spec.placement.mode = 'isolated')}>Isolated sandbox</button>
    </div>
    <p class="dim">{spec.placement.mode === 'isolated' ? 'Its own sandbox (EE: a dedicated pod): nothing else runs beside it. For untrusted code, noisy work, or data that must stay apart.' : 'Loaded into a runtime pool with other flows: cheapest and quickest to start. Right for most flows.'}</p>
    <div class="fields">
      <label class="fld">Runtime labels <small>key=value, comma-separated; empty: the default runtime</small><input class="mono" bind:value={selText} placeholder="class=gpu, tier=prod" /></label>
      <label class="fld">Namespace <small>empty: the runtime's own</small><input bind:value={spec.placement.namespace} placeholder="a3t" /></label>
      <label class="fld full">Pinned runtime <small>one runtime by name; it must still match the labels</small>
        <select value={spec.placement.runtime ?? ''} onchange={(e) => (spec.placement.runtime = e.currentTarget.value || null)}>
          <option value="">— none: pick by labels —</option>
          {#if spec.placement.runtime && !pinKnown}<option value={spec.placement.runtime}>{spec.placement.runtime} (not registered)</option>{/if}
          {#each runtimes as r (r.namespace + '/' + r.name)}<option value={r.name}>{r.name}</option>{/each}
        </select></label>
    </div>
  </div>
  <div class="card">
    <div class="card-head"><h3>2 · Where it lands</h3><span>each registered runtime, and whether it takes this canvas</span></div>
    {#if placed}<p class="ok">It lands on <b>{placed.name}</b>.</p>{:else if runtimes.length}<div class="note bad">No runtime takes it. Change the labels, the namespace or the pin — or register a runtime with these labels in Runtime.</div>{/if}
    <ul class="rows">
      {#each verdictList as v (v.rt.namespace + '/' + v.rt.name)}
        <li><span class="lbl">{v.rt.name}</span>
          <span class="val mono">{selectorText(v.rt.labels) || 'no labels'} · {v.rt.namespace}{v.rt.default ? ' · default' : ''}</span>
          <span class="tag" class:prod={v.ok && placed?.name === v.rt.name}>{v.ok ? (placed?.name === v.rt.name ? 'takes it' : 'would take it') : 'no'}</span>
          {#if !v.ok}<span class="sub">{v.why}</span>{/if}</li>
      {:else}<li><span class="dim">No runtime registered yet.</span><span></span><button class="btn2" onclick={() => navigateTo('runtime', 'Runtime')}>Runtime →</button></li>{/each}
    </ul>
  </div>
{/snippet}

{#snippet lifecycle()}
  <div class="card">
    <div class="card-head"><h3>1 · State and session</h3><span>whether it takes work, and whether it remembers between runs</span></div>
    <div class="row">
      <div class="seg src" role="tablist">
        <button role="tab" class:on={spec.lifecycle.state === 'running'} onclick={() => (spec.lifecycle.state = 'running')}>Running</button>
        <button role="tab" class:on={spec.lifecycle.state === 'paused'} onclick={() => (spec.lifecycle.state = 'paused')}>Paused (hibernate)</button>
      </div>
      <div class="seg src" role="tablist">
        <button role="tab" class:on={spec.lifecycle.session === 'ephemeral'} onclick={() => (spec.lifecycle.session = 'ephemeral')}>Ephemeral</button>
        <button role="tab" class:on={spec.lifecycle.session === 'persistent'} onclick={() => (spec.lifecycle.session = 'persistent')}>Persistent</button>
      </div>
    </div>
    <p class="dim">{spec.lifecycle.state === 'paused' ? 'Paused: deployed but asleep; calls wake it.' : 'Running: it takes work as it comes.'}
      {spec.lifecycle.session === 'persistent' ? 'Persistent: each run checkpoints, and the next one resumes from it — for agents that remember.' : 'Ephemeral: each run starts clean.'}</p>
  </div>
  <div class="card">
    <div class="card-head"><h3>2 · Scaling</h3><span>how many instances, and on what signal</span></div>
    <div class="fields">
      <label class="fld">At least<input type="number" min="0" bind:value={spec.lifecycle.scaling.min} /></label>
      <label class="fld">At most<input type="number" min="0" bind:value={spec.lifecycle.scaling.max} /></label>
      <label class="fld">Scale on <small>a metric, e.g. queue_depth; empty: CPU</small><input class="mono" value={spec.lifecycle.scaling.metric ?? ''} oninput={(e) => (spec.lifecycle.scaling.metric = e.currentTarget.value || null)} placeholder="queue_depth" /></label>
      <label class="fld">Target per instance<input type="number" min="0" value={spec.lifecycle.scaling.target ?? ''} oninput={(e) => (spec.lifecycle.scaling.target = e.currentTarget.value === '' ? null : Number(e.currentTarget.value))} placeholder="20" /></label>
    </div>
    {#if spec.lifecycle.scaling.max < spec.lifecycle.scaling.min}<p class="err">At most must be at least {spec.lifecycle.scaling.min}.</p>{/if}
  </div>
  <div class="card">
    <div class="card-head"><h3>3 · Restarts</h3><span>when an instance stops</span></div>
    <div class="fields three">
      <label class="fld">Restart<select bind:value={spec.lifecycle.restart.policy}><option value="on-failure">on failure</option><option value="always">always</option><option value="never">never</option></select></label>
      <label class="fld">Backoff<select bind:value={spec.lifecycle.restart.backoff}><option value="exponential">exponential</option><option value="linear">linear</option><option value="fixed">fixed</option></select></label>
      <label class="fld">At most<input type="number" min="0" bind:value={spec.lifecycle.restart.maxRetries} /></label>
    </div>
  </div>
{/snippet}

{#snippet deps()}
  <div class="card">
    <div class="card-head"><h3>1 · What feeds it</h3><span>triggers that send to its flow</span></div>
    {#if ctx.triggers.length}
      <ul class="rows">{#each ctx.triggers as t (t.id)}<li><span class="lbl">{t.name}</span><span class="val mono">{triggerLabel(t)}</span><span class="tag" class:prod={t.enabled !== false}>{t.enabled === false ? 'paused' : 'active'}</span></li>{/each}</ul>
    {:else}<p class="dim">No trigger sends to this flow: it runs when it is called.</p>{/if}
    <div class="acts"><button class="link" onclick={() => navigateTo('triggers', 'Triggers')}>Triggers →</button></div>
  </div>
  <div class="card">
    <div class="card-head"><h3>2 · What its flow calls</h3><span>read from the flow's definition</span></div>
    {#if ctx.calls.length}<div class="chips">{#each ctx.calls as c (c)}<span class="tag">{c}</span>{/each}</div>{:else}<p class="dim">{ctx.flowExists === false ? 'Its flow does not exist.' : 'No calls to outside systems.'}</p>{/if}
  </div>
  <div class="card">
    <div class="card-head"><h3>3 · Resources it depends on</h3><span>watched while it runs</span></div>
    {#if spec.health.dependsOn.length}
      <ul class="rows">
        {#each spec.health.dependsOn as d (d.resource)}
          <li><span class="lbl mono">{d.resource}</span>
            <span class="val">{#if resourceKnown(d.resource)}<span class="dot ok"></span> registered · {resources.find((r) => r.ref === d.resource)?.type}{:else}<span class="dot err"></span> not registered{/if}</span>
            <button class="btn2 quiet" onclick={() => removeDep(d.resource)} disabled={readonly}>Remove</button></li>
        {/each}
      </ul>
    {:else}<p class="dim">None declared: FlowR watches the resources its flow calls.</p>{/if}
    {#if !readonly}
      <div class="row">
        <select class="grow pick" bind:value={depPick}>
          <option value="">— add a resource —</option>
          {#each resources.filter((r) => !spec.health.dependsOn.some((d) => d.resource === r.ref)) as r (r.ref)}<option value={r.ref}>{r.ref} · {r.type}</option>{/each}
        </select>
        <button class="btn2" disabled={!depPick} onclick={addDep}>Add</button>
      </div>
    {/if}
  </div>
  <div class="card">
    <div class="card-head"><h3>4 · When one is down</h3><span>what happens to its runs</span></div>
    <div class="seg src" role="tablist">
      {#each ON_DOWN as o (o.v)}<button role="tab" class:on={spec.health.onDependencyDown === o.v} onclick={() => (spec.health.onDependencyDown = o.v)}>{o.v}</button>{/each}
    </div>
    <p class="dim">{ON_DOWN.find((o) => o.v === spec.health.onDependencyDown)?.t}.</p>
  </div>
{/snippet}

{#snippet yaml()}
  <div class="te-yaml">{#key yamlRev}
    <FlowEditorPane part="main" initialYaml={yamlDraft} onchange={readonly ? undefined : applyYaml} readOnly={readonly}>
      {#snippet toolbar()}<span class="grow"></span><span class="ver mono">{namespace}/{name || 'new canvas'}</span>
        {#if cur}<button class="link" onclick={() => openInStudio('canvas', cur.namespace ?? namespace, cur.name)}>Edit in Studio →</button>{/if}{/snippet}
    </FlowEditorPane>
  {/key}</div>
  {#if yamlErr}<div class="note bad">{yamlErr}</div>{/if}
{/snippet}

{#snippet deployStep()}
  <div class="card">
    <div class="card-head"><h3>1 · Deploy</h3><span>{dirty || !cur ? 'it is saved first' : 'what it will do'}</span></div>
    <ul class="rows">
      <li><span class="lbl">Flow</span><span class="val mono">{spec.flowRef || '—'}</span><span></span></li>
      <li><span class="lbl">On</span><span class="val">{placed ? placed.name : 'no runtime takes it'} · {spec.placement.mode === 'isolated' ? 'its own sandbox' : 'shared pool'}</span><span></span></li>
      <li><span class="lbl">As</span><span class="val">{howRuns}</span><span></span></li>
    </ul>
    {#if problems.length}<div class="note bad"><b>Fix first:</b><ul class="notes">{#each problems as p (p)}<li>{p}</li>{/each}</ul></div>{/if}
    <div class="acts">
      <button class="btn2 primary" disabled={readonly || deploying || problems.length > 0} onclick={deploy}>{deploying ? 'Deploying…' : ctx.live ? 'Deploy again' : 'Deploy'}</button>
      {#if deployMsg}<span class="dim">{deployMsg}</span>{/if}
    </div>
  </div>
  <div class="card">
    <div class="card-head"><h3>2 · Live</h3><span>how it is running now</span><span class="grow"></span><button class="btn2 quiet rm" onclick={() => refreshAround()}>Refresh</button></div>
    {#if ctx.live}
      <ul class="rows">
        <li><span class="lbl">State</span><span class="val"><span class="dot" class:ok={liveState === 'serving'} class:warn={liveState !== 'serving'}></span> {servingLabel(liveState)}</span><span></span>
          {#if ctx.live.deployment?.pauseReason}<span class="sub">“{ctx.live.deployment.pauseReason}”</span>{/if}</li>
        <li><span class="lbl">Runtime</span><span class="val mono">{ctx.live.deployment?.runtime ?? '—'}</span><span></span></li>
      </ul>
      {#if !readonly && depId}
        <div class="acts">
          {#if liveState === 'paused'}<button class="btn2" disabled={!!acting} onclick={() => act('resume')}>Resume</button>{:else}<button class="btn2" disabled={!!acting} onclick={() => act('pause')}>Pause</button>{/if}
          <button class="btn2" disabled={!!acting} onclick={() => act('restart')}>Restart</button>
          <button class="btn2 quiet" disabled={!!acting} onclick={() => act('undeploy')}>Undeploy</button>
          <span class="grow"></span><button class="link" onclick={() => navigateTo('runtime', 'Runtime')}>Open in Runtime →</button>
        </div>
      {/if}
    {:else}<p class="empty">Not deployed.</p>{/if}
  </div>
{/snippet}

{#if embedded}
  <!-- The chat pane: every section, one under another; saved through the caller. -->
  <div class="cd-embedded">
    <div class="cd-embar">
      <span class="dim">{readonly ? (readonlyNote || 'Shared preset — view only. Copy it to make your own.') : spec.flowRef ? `Deploys ${spec.flowRef}` : 'Your copy — not bound to a flow yet'}</span>
      {#if !readonly && onsavespec}<button class="btn2 primary" onclick={() => save(true)} disabled={saving}>{saving ? 'Saving…' : 'Save'}</button>{/if}
    </div>
    <div class="cd-embody"><fieldset disabled={readonly}><section class="agt">
      {#if msg}<div class="note bad">{msg}</div>{/if}
      {@render details()}{@render placement()}{@render lifecycle()}{@render deps()}
    </section></fieldset></div>
  </div>
{:else}
<SlidePanel {open} {onclose} onsave={readonly ? undefined : () => save()}
  onopenineditor={cur ? () => openInStudio('canvas', cur.namespace ?? namespace, cur.name) : undefined}>
  <div class="panel-layout">
    <PropertiesTree icon="CV" color="#3a6ea5" name={name || 'New canvas'} meta={`${spec.flowRef || 'no flow'} · ${namespace}`} nodes={[]}
      {steps} wizard={!cur && !readonly} activeNode={step} onselect={(id) => (step = id)} />
    <div class="panel-col">
      <div class="panel-content"><div class="pf">
        <WizardHead step={steps.find((x) => x.id === step)} />
        <fieldset disabled={readonly}><section class="agt">
          {#if msg}<div class="note bad" role="alert">{msg}</div>{/if}
          {#if readonly}<div class="note">{readonlyNote || 'A shared preset: view only. Copy it to make a canvas of your own.'}</div>{/if}
          {#if step === 'details'}{@render details()}{@render status()}
          {:else if step === 'placement'}{@render placement()}
          {:else if step === 'lifecycle'}{@render lifecycle()}
          {:else if step === 'deps'}{@render deps()}
          {:else if step === 'yaml'}{@render yaml()}
          {:else}{@render deployStep()}{/if}
        </section></fieldset>
      </div></div>
      <WizardNav {steps} active={step} wizard={!cur && !readonly} onselect={(id) => (step = id)}
        final={readonly ? undefined : () => save()} finalLabel={cur ? 'Save' : 'Create canvas'} finalBusy={saving} busyLabel={cur ? 'Saving…' : 'Creating…'} />
    </div>
  </div>
</SlidePanel>
{/if}

<style>
  .panel-layout { display: grid; grid-template-columns: 280px 1fr; height: 100%; }
  .panel-col { position: relative; display: flex; flex-direction: column; min-height: 0; min-width: 0; height: 100%; }
  .panel-content { overflow-y: auto; flex: 1; min-height: 0; }
  .pf { padding: 24px 32px; max-width: var(--detail-w); box-sizing: border-box; }
  @container details (max-width: 560px) { .pf { padding: 16px 14px; } }
  fieldset { border: 0; margin: 0; padding: 0; min-width: 0; }
  .te-yaml { height: max(480px, calc(100vh - 300px)); border: 1px solid var(--border); border-radius: 10px; overflow: hidden; display: flex; flex-direction: column; }
  .te-yaml > :global(*) { flex: 1; min-height: 0; }
  .te-yaml .ver { font-size: 12px; color: var(--text-3); min-width: 0; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .te-yaml .link { padding: 0 8px; white-space: nowrap; flex: 0 0 auto; }
  .fields.three { grid-template-columns: 1fr 1fr 1fr; }
  @container details (max-width: 560px) { .fields.three { grid-template-columns: 1fr; } }
  .pick { height: 34px; padding: 0 10px; border: 1px solid var(--border); border-radius: 8px; background: var(--bg-surface); font: 13px var(--font-sans); color: var(--text-1); min-width: 0; }
  .cd-embedded { display: flex; flex-direction: column; height: 100%; min-height: 0; }
  .cd-embar { display: flex; align-items: center; justify-content: space-between; gap: 8px; padding: 8px 12px; border-bottom: 1px solid var(--border); }
  .cd-embody { flex: 1; min-height: 0; overflow-y: auto; padding: 14px; }
</style>
