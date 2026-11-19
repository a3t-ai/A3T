<script lang="ts">
  /// Executions — every run in the run store. Items lists them (newest first, in
  /// the project picked in the top bar); the one picked opens here in the agents'
  /// frame: Overview (status, why it failed, what ran, run it again), Steps,
  /// Timeline, Journal, Resume points, Input & output. A failed run's Overview
  /// carries the A3T error log's classification of it (the authoring tier —
  /// operator, admin, SuperAdmin; the error log flow refuses anyone else); this replaced the Errors app.
  import { untrack } from 'svelte';
  import SlidePanel from '$lib/components/shared/SlidePanel.svelte';
  import PropertiesTree from '$lib/components/shared/PropertiesTree.svelte';
  import WizardHead from '$lib/components/shared/WizardHead.svelte';
  import RunView from '$lib/components/run/RunView.svelte';
  import '$lib/components/agent/agent.css';
  import '$lib/components/triggers/triggers.css';
  import { execFlow } from '$lib/services/api';
  import { hasRole } from '$lib/stores/auth';
  import { trackActiveItem } from '$lib/stores/activeItem.svelte.ts';
  import { bumpItems } from '$lib/stores/apps.svelte.ts';
  import { getActivePanel, navigateTo, navigateToDetail, openInStudio } from '$lib/stores/panels.svelte.ts';

  type View = 'overview' | 'steps' | 'timeline' | 'journal' | 'checkpoints' | 'io';
  let id = $state('');
  let tab = $state<View>('overview');
  let run = $state<any>(null);
  let failure = $state<any>(null);
  let shown = '';
  $effect(() => {
    const sel = (getActivePanel()?.props?.selectedId as string | undefined) ?? '';
    untrack(() => {
      if (sel && sel !== shown) { shown = sel; id = sel; tab = 'overview'; run = null; failure = null; }
      else if (!sel) { shown = ''; id = ''; }
    });
  });
  const close = () => navigateTo('executions', 'Executions');
  const openRun = (rid: string) => navigateToDetail('executions', 'Executions', rid);

  /// The error log's row for a failed run, read once per run.
  let classified = '';
  function onrun(r: any) {
    run = r;
    if (r.phase === 'faulted' && classified !== r.run_id && hasRole('operator')) {
      classified = r.run_id;
      void execFlow('flow_a3t_error_log', 'a3t', { action: 'list', run_id: r.run_id, limit: 1 }, { silent: true }).then((res) => {
        if (res.ok && res.data?.ok !== false && id === r.run_id) failure = res.data?.errors?.[0] ?? null;
      });
    }
  }

  // The chat's context: the run on screen (read-only).
  trackActiveItem(() => run ? { kind: 'run', namespace: run.namespace || undefined, name: `${run.flow || 'run'} (${run.run_id})`, id: run.run_id } : null);

  const PHASE: Record<string, string> = { completed: 'completed', faulted: 'failed', cancelled: 'cancelled', running: 'running', pending: 'queued', waiting: 'waiting', suspended: 'paused' };
  const I = {
    overview: '<rect x="2" y="2" width="12" height="12" rx="2"/><path d="M5 6h6M5 9h4"/>',
    steps: '<path d="M3 4h10M3 8h10M3 12h6"/><circle cx="13" cy="12" r="1.2"/>',
    timeline: '<path d="M2 4h6M5 8h8M3 12h5"/>',
    journal: '<rect x="2" y="1" width="12" height="14" rx="1.5"/><path d="M5 5h6M5 8h6M5 11h3"/>',
    checkpoints: '<path d="M4 14V2M4 3h8l-2 3 2 3H4"/>',
    io: '<path d="M2 5h8M7 2l3 3-3 3M14 11H6M9 8l-3 3 3 3"/>',
  };
  const steps = $derived([
    { id: 'overview', label: 'Overview', icon: I.overview, hint: 'Its status, why it failed, what ran — and run it again' },
    { id: 'steps', label: 'Steps', icon: I.steps, hint: 'Each step it ran: its input, output and the context after it' },
    { id: 'timeline', label: 'Timeline', icon: I.timeline, hint: 'Where its time went: waiting for a worker, running, paused, each step' },
    { id: 'journal', label: 'Journal', icon: I.journal, hint: 'Every record the engine wrote for it, in order' },
    { id: 'checkpoints', label: 'Resume points', icon: I.checkpoints, hint: 'Each completed step — re-run from one in debug' },
    { id: 'io', label: 'Input & output', icon: I.io, hint: 'What it was started with, and what it returned' },
  ]);
  const meta = $derived(run ? `run · ${PHASE[run.phase] ?? run.phase} · ${run.namespace}` : 'run');
</script>

<SlidePanel open={!!id} onclose={close}>
  {#if id}
    <div class="panel-layout">
      <PropertiesTree icon={run?.phase === 'faulted' ? '✕' : run?.phase === 'completed' ? '✓' : '▶'}
        color={run?.phase === 'faulted' ? '#b42318' : run?.phase === 'completed' ? '#1a7158' : '#55514a'}
        name={run?.flow ?? 'Run'} {meta} nodes={[]} {steps} activeNode={tab} onselect={(v) => (tab = v as View)} />
      <div class="panel-col"><div class="panel-content"><div class="pf">
        <WizardHead step={steps.find((s) => s.id === tab)} />
        <section class="agt">
          {#key id}
            <RunView runId={id} view={tab} {failure} {onrun} onview={(v) => (tab = v as View)}
              onopen={run?.flow ? () => openInStudio('flow', run.namespace || 'a3t', run.flow) : undefined}
              onparent={openRun} onrerun={(rid) => { bumpItems(); openRun(rid); }} />
          {/key}
        </section>
      </div></div></div>
    </div>
  {/if}
</SlidePanel>

<style>
  .panel-layout { display: grid; grid-template-columns: 280px 1fr; height: 100%; }
  .panel-col { display: flex; flex-direction: column; min-height: 0; min-width: 0; height: 100%; }
  .panel-content { overflow-y: auto; flex: 1; min-height: 0; }
  .pf { padding: 24px 32px; max-width: var(--detail-w); box-sizing: border-box; }
  @container details (max-width: 560px) { .pf { padding: 16px 14px; } }
</style>
