<script lang="ts">
  import { openPublish } from '$lib/stores/publish.svelte.ts';
  import ExposedSections from '$lib/components/shared/ExposedSections.svelte';
  import { inContext, namespacesFor, isPlatform, itemProject, getProject, canWriteIn } from '$lib/stores/context.svelte.ts';
  import FoldSection from '$lib/components/shared/FoldSection.svelte';
  import ProjectPill from '$lib/components/shared/ProjectPill.svelte';
  import { contextRev_, bumpItems } from '$lib/stores/apps.svelte.ts';
  import { getActivePanel, openInStudio } from '$lib/stores/panels.svelte.ts';
  import SubmenuToggle from '$lib/components/shared/SubmenuToggle.svelte';
  import NewButton from '$lib/components/shared/NewButton.svelte';
  import { API_BASE } from '$lib/config';
  import { mcpCall } from '$lib/services/api';
  import { createTrigger, putTrigger } from '$lib/studio-core/adapters/config';
  import '$lib/components/triggers/triggers.css';
  import TriggerTargets from '$lib/components/triggers/TriggerTargets.svelte';
  import type { MapCtx } from '$lib/components/triggers/TriggerMapping.svelte';
  import { listRecords, type MappingRecord } from '$lib/components/triggers/mapperApi';
  import FlowEditorPane from '$lib/components/editor/FlowEditorPane.svelte';
  import PropertiesTree from '$lib/components/shared/PropertiesTree.svelte';
  import { toYaml, fromYaml, BODY_OF, targetLabel, type StoredTarget, type SourceKind } from '$lib/core/triggers/triggerSpec';
  import { agentTool, getAccessToken } from '$lib/services/api';
  import '$lib/components/agent/agent.css';
  import { copy, endpointUrl } from '$lib/components/agent/agentApi';

  import SlidePanel from '$lib/components/shared/SlidePanel.svelte';
  import PageBanner from '$lib/components/shared/PageBanner.svelte';
  import { bannerFor } from '$lib/shell/appTheme';
  import AppHome from '$lib/shell/AppHome.svelte';
  import DeleteConfirm from '$lib/components/shared/DeleteConfirm.svelte';
  import { onMount, untrack } from 'svelte';
  import type { WizardStep } from '$lib/components/shared/WizardSteps.svelte';
  import WizardNav from '$lib/components/shared/WizardNav.svelte';
  import WizardHead from '$lib/components/shared/WizardHead.svelte';
  import { trackActiveItem } from '$lib/stores/activeItem.svelte.ts';

  type TriggerTarget = StoredTarget;
  interface Trigger {
    id: string; name: string; namespace: string;
    config: { type: string; path?: string; method?: string; expression?: string; source?: string; message_template?: string; message?: string; timezone?: string; filter?: string; [k: string]: any };
    targets: TriggerTarget[]; enabled: boolean; status: string; description?: string;
  }
  interface FlowSummary { name: string; namespace: string; version?: string; type?: string }

  // ── View state ──
  let view = $state<'list' | 'editor'>('list');
  let loading = $state(true);
  let triggers = $state<Trigger[]>([]);
  const myTriggers = $derived(triggers.filter((t: any) => !isPlatform(t.project)));
  const sharedTriggers = $derived(triggers.filter((t: any) => isPlatform(t.project)));
  let availableFlows = $state<FlowSummary[]>([]);
  let editingTrigger = $state<Trigger | null>(null);
  /// Read-only where you can't write in its project: no save, pause, fire or remove.
  const nsOfTrigger = (t: Trigger) => ((t as any).project ?? t.namespace) as string;
  const trgRO = $derived(!!editingTrigger && !canWriteIn(nsOfTrigger(editingTrigger)));
  let deleteTarget = $state<Trigger | null>(null);

  // ── Form state ──
  let formName = $state('');
  let formDesc = $state('');
  let formType = $state<SourceKind>('webhook');
  let formEnabled = $state(true);
  let formNs = $state(getProject() || 'default');
  let formPath = $state('/');
  let formMethod = $state('POST');
  let formMsgTemplate = $state('');
  let formCron = $state('*/5 * * * *');
  let formCronMsg = $state('');
  let formTimezone = $state('');
  let formEventSource = $state('');
  let formEventFilter = $state('');
  let formTopicRes = $state('a3t/adb');
  let formTopic = $state('');
  let formTopicFilter = $state('');
  let formTargets = $state<TriggerTarget[]>([]);
  /// Agents a trigger can send to (id → name), for the target picker.
  let agentOptions = $state<{ value: string; label: string }[]>([]);
  async function loadAgentOptions() {
    const r = await agentTool('agent_a3t_agents', 'tool_a3t_agent_list', { org_id: 'org-default' });
    agentOptions = (Array.isArray(r.data) ? r.data : []).map((a: any) => ({ value: a.id, label: `${a.name} · ${a.namespace}` }));
  }
  const flowOptions = $derived(availableFlows.map((f) => ({ value: `${f.namespace}/${f.name}`, label: `${f.namespace}/${f.name}` })));

  // ── Test state ──
  let testPayload = $state('{"message": "Hello from test!"}');
  let testRunning = $state(false);
  let testError = $state('');

  /// The runs the last firing started, each followed to its end.
  type TestRun = { target: string; instance_id: string; status: string; input?: unknown; output?: unknown; error?: string; duration_ms?: number };
  let testRuns = $state<TestRun[]>([]);
  /// What the platform answered when nothing ran to follow (a URL target, say).
  let testReply = $state<any>(null);

  async function runTest() {
    if (formType !== 'webhook' || !formPath) return;
    testRunning = true; testRuns = []; testReply = null; testError = '';

    // A new trigger is saved first; edits after that save to it.
    if (!editingTrigger) {
      await saveTrigger(true);
      if (saveError) { testError = `Not saved: ${saveError}`; testRunning = false; return; }
      const found = triggers.find(t => t.name === formName && (t.namespace || 'default') === (formNs || 'default'));
      if (found) editingTrigger = found;
    }

    let payload: any;
    try { payload = JSON.parse(testPayload); } catch { testError = 'The body is not JSON.'; testRunning = false; return; }
    try {
      const path = formPath.startsWith('/') ? formPath : '/' + formPath;
      const res = await fetch(`${API_BASE}/webhook${path}`, {
        method: formMethod || 'POST',
        // As you: an agent's webhook takes no call without a token.
        headers: { 'Content-Type': 'application/json', Authorization: `Bearer ${getAccessToken() ?? ''}` },
        body: JSON.stringify(payload),
      });
      const reply = await res.json().catch(() => ({}));
      // A mapping that fails, a target that refuses: say which and why.
      const failed = (reply.failed ?? []).map((f: any) => f.error);
      if (!res.ok || reply.error) {
        testError = failed.length ? `Nothing ran — ${failed.join('; ')}` : (reply.error === 'paused' ? 'It is paused: nothing ran.' : reply.error ?? `HTTP ${res.status}`);
        testRunning = false; return;
      }
      if (failed.length) testError = `Some targets failed — ${failed.join('; ')}`;
      testRuns = (reply.executions ?? []).map((e: any) => ({ target: e.target, instance_id: e.instance_id, status: 'running' }));
      if (!testRuns.length) {
        testReply = reply;
        if (!reply.delivered) testError = 'It fired, but nothing ran: no target took it.';
        testRunning = false; return;
      }
      // Follow each run to its end (up to 60 s).
      for (let i = 0; i < 60 && testRuns.some((r) => r.status === 'running'); i++) {
        await new Promise((r) => setTimeout(r, 1000));
        await Promise.all(testRuns.map(async (r, k) => {
          if (r.status !== 'running') return;
          const st = await mcpCall('get_execution_status', { instance_id: r.instance_id });
          if (!st.ok) return;
          const d: any = typeof st.data === 'string' ? JSON.parse(st.data) : st.data;
          const status = String(d?.status ?? '').toLowerCase();
          // What the target got — the mapping's result — as soon as it is known.
          if (d?.input !== undefined && testRuns[k].input === undefined) testRuns[k] = { ...testRuns[k], input: d.input };
          if (status === 'completed' || status === 'failed') {
            testRuns[k] = { ...testRuns[k], status, output: d.output, error: status === 'failed' ? (d.error ?? 'It failed.') : undefined,
              duration_ms: d.duration_us != null ? Math.round(d.duration_us / 1000) : undefined };
          }
        }));
      }
      testRuns = testRuns.map((r) => (r.status === 'running' ? { ...r, error: 'Still running after 60 s: see Executions.' } : r));
    } catch (e) {
      testError = String(e);
    }
    testRunning = false;
  }

  // ── What the tabs show ──
  const SOURCES: SourceKind[] = ['webhook', 'cron', 'adb_topic', 'event'];
  const SOURCE_NAME: Record<SourceKind, string> = { webhook: 'Webhook', cron: 'Schedule', adb_topic: 'ADB topic', event: 'Platform event' };
  /// The event each source makes — what the mappings read.
  const EVENT_SHAPE: Record<SourceKind, string> = {
    webhook: `{\n  "body":    <the JSON the caller POSTs>,\n  "message": "<the trigger's message, or the body as text>",\n  "source":  "webhook",\n  "trigger": "<this trigger's name>",\n  "time":    "2026-11-13T17:42:00Z"\n}`,
    cron: `{\n  "body":    { "trigger": "cron", "trigger_id": "…", "fired_at": "…", "minute": "2026-11-13T17:42" },\n  "message": "<its message>",\n  "source":  "cron",\n  "trigger": "<this trigger's name>",\n  "time":    "…"\n}`,
    adb_topic: `{\n  "body":    { <the message's fields>, "_topic": "orders", "_offset": 1042 },\n  "message": "<the message as text>",\n  "source":  "adb_topic",\n  "trigger": "<this trigger's name>",\n  "time":    "…"\n}`,
    event: `{\n  "body":    { "specversion": "1.0", "id": "…", "type": "<event type>", "source": "…", "data": { … } },\n  "message": "…",\n  "source":  "event",\n  "trigger": "<this trigger's name>",\n  "time":    "…"\n}`,
  };
  const firesOn = $derived(formType === 'webhook' ? (formPath ? `${formMethod} /webhook${formPath.startsWith('/') ? formPath : '/' + formPath}` : '')
    : formType === 'cron' ? formCron : formType === 'adb_topic' ? (formTopic ? `${formTopicRes || '?'} · ${formTopic}` : '') : formEventSource);
  const mappedCount = $derived(formTargets.filter((t) => t.input_mapping || t.message || t.conversation).length);
  let copiedUrl = $state(false);
  async function cpUrl() { if (await copy(endpointUrl(formPath))) { copiedUrl = true; setTimeout(() => (copiedUrl = false), 1500); } }


  // ── Load ──
  async function loadTriggers() {
    loading = true;
    const res = await mcpCall('list_triggers', {});
    if (res.ok && res.data) {
      let d: any = res.data;
      if (typeof d === 'string') { try { d = JSON.parse(d); } catch { d = []; } }
      // list_triggers returns { count, triggers: [...] }
      const all: Trigger[] = Array.isArray(d) ? d : (d?.triggers ?? d?.rows ?? []);
      // Users' triggers saved in `a3t` before projects (agent endpoints) are `default`'s.
      triggers = all
        .map((t: any) => ({ ...t, project: itemProject('trigger', t.namespace, t) }))
        .filter((t: any) => inContext(t.project));
    } else triggers = [];
    loading = false;
  }

  // A different project (or "show platform") is a different list.
  let loadedRev = contextRev_();
  $effect(() => {
    const rev = contextRev_();
    if (rev === loadedRev) return;
    loadedRev = rev;
    void loadTriggers();
  });


  async function loadFlows() {
    const all: FlowSummary[] = [];
    // Targets may be platform flows (an agent's chat wrapper), so the shared set.
    for (const ns of namespacesFor('picker')) {
      const res = await mcpCall('list_flows', { namespace: ns });
      if (res.ok && res.data) {
        // A project with no flows answers in words ("No workflows found."), not JSON.
        let raw: any = res.data;
        if (typeof raw === 'string') { try { raw = JSON.parse(raw); } catch { raw = []; } }
        const flows = raw.workflows ?? raw.flows ?? (Array.isArray(raw) ? raw : []);
        for (const f of flows) {
          all.push({ name: f.name, namespace: f.namespace ?? ns, version: f.version, type: f.name.includes('agent') ? 'agent' : 'flow' });
        }
      }
    }
    availableFlows = all;
  }


  onMount(() => {
    loadTriggers(); loadFlows(); loadAgentOptions();
    if (getActivePanel()?.props?.create) openCreate();  // the shell's "New trigger"
  });

  // ── Form helpers ──
  function resetForm() {
    formName = ''; formDesc = ''; formType = 'webhook'; formEnabled = true; formNs = getProject() || 'default';
    formPath = '/'; formMethod = 'POST'; formMsgTemplate = '';
    formCron = '*/5 * * * *'; formCronMsg = ''; formTimezone = '';
    formEventSource = ''; formEventFilter = '';
    formTopicRes = 'a3t/adb'; formTopic = ''; formTopicFilter = '';
    formTargets = [];
  }

  function openCreate() { editingTrigger = null; resetForm(); step = 'details'; view = 'editor'; formRev++; }

  /// The editor's steps: what it is, what fires it, where it delivers, a test.
  /// A new trigger goes in order; an existing one opens on any step.
  let step = $state('details');
  let saving = $state(false);
  const trgSteps = $derived.by((): WizardStep[] => [
    { id: 'details', icon: '<rect x="2" y="2" width="12" height="12" rx="2"/><path d="M5 6h6M5 9h4"/>', label: 'Details', hint: 'Name, project, description', ok: !!formName.trim() && !!formNs, need: 'Name the trigger.' },
    { id: 'source', icon: '<path d="M9 1L3 9h5l-1 6 6-8H8z"/>', label: 'On', hint: 'What fires it: a webhook, a schedule, an ADB topic or a platform event', ok: formType === 'webhook' ? !!formPath.trim() : formType === 'cron' ? !!formCron.trim() : formType === 'adb_topic' ? !!formTopic.trim() && !!formTopicRes.trim() : !!formEventSource.trim(),
      need: formType === 'webhook' ? 'Give the webhook a path.' : formType === 'cron' ? 'Give the schedule a cron expression.' : formType === 'adb_topic' ? 'Name the ADB resource and topic.' : 'Name the event source.' },
    { id: 'destinations', icon: '<path d="M2 8h9M8 4l4 4-4 4"/><path d="M14 3v10"/>', label: 'Send', hint: 'The flows and agents each firing goes to, and the input each gets', ok: formTargets.length > 0 && formTargets.every(targetOk), need: 'Add a target, and pick its flow or agent.' },
    { id: 'yaml', icon: '<path d="M5 3L1 8l4 5M11 3l4 5-4 5"/>', label: 'YAML', hint: 'The trigger as code: a graph of what fires it and where it goes, and the YAML the Studio edits' },
    { id: 'test', icon: '<path d="M4 2l10 6-10 6z"/>', label: 'Test', hint: 'Fire it with a sample body and see what each target got', optional: true },
  ]);

  const targetOk = (g: TriggerTarget) => g.type === 'workflow' ? !!(g.namespace && g.name) : g.type === 'a3t_agent' ? !!g.agent_id : g.type === 'webhook' ? !!g.url : true;

  function openEdit(t: Trigger) {
    editingTrigger = t;
    formName = t.name; formDesc = t.description ?? ''; formType = t.config.type as any;
    formEnabled = t.enabled; formNs = t.namespace;
    formPath = t.config.path ?? '/'; formMethod = t.config.method ?? 'POST'; formMsgTemplate = t.config.message_template ?? '';
    formCron = t.config.expression ?? '*/5 * * * *'; formCronMsg = t.config.message ?? ''; formTimezone = t.config.timezone ?? '';
    formEventSource = t.config.source ?? ''; formEventFilter = typeof t.config.filter === 'string' ? t.config.filter : '';
    formTopicRes = t.config.resource ?? 'a3t/adb'; formTopic = t.config.topic ?? '';
    formTopicFilter = t.config.type === 'adb_topic' && t.config.filter ? JSON.stringify(t.config.filter) : '';
    formTargets = t.targets.map((g) => ({ ...g }));
    step = 'details';
    view = 'editor';
    formRev++;   // the form now holds the saved trigger
  }
  let formRev = $state(0);

  /// The trigger the form describes, as the platform stores it.
  function formTrigger() {
    const c: any = { type: formType };
    if (formType === 'webhook') { c.path = formPath; c.method = formMethod; if (formMsgTemplate) c.message_template = formMsgTemplate; }
    else if (formType === 'cron') { c.expression = formCron; c.message = formCronMsg; if (formTimezone) c.timezone = formTimezone; }
    else if (formType === 'event') { c.source = formEventSource; if (formEventFilter) c.filter = formEventFilter; }
    else if (formType === 'adb_topic') {
      c.resource = formTopicRes; c.topic = formTopic;
      if (formTopicFilter.trim()) { try { c.filter = JSON.parse(formTopicFilter); } catch { c.filter = undefined; } }
    }
    // Empty mappings are no mapping: the target gets the default.
    const targets = formTargets.map((g) => Object.fromEntries(Object.entries(g).filter(([, v]) => v !== '' && v != null)) as TriggerTarget);
    return { name: formName, namespace: formNs, config: c, targets, enabled: formEnabled, description: formDesc };
  }

  // ── YAML: the form and the text are one trigger ──
  let yamlIssues = $state<string[]>([]);
  let yamlRev = $state(0);
  /// The text in the designer: the form's trigger when the YAML step opens, then
  /// what is typed (re-made from the form on each keystroke, it would jump).
  let yamlDraft = $state('');
  $effect(() => { if (step === 'yaml') untrack(() => { yamlDraft = toYaml(formTrigger() as any); yamlRev++; }); });
  function applyYaml(text: string) {
    yamlDraft = text;
    const r = fromYaml(text);
    yamlIssues = r.issues;
    if (!r.trigger || r.issues.length) return;
    const t = r.trigger;
    formName = t.name; formNs = t.namespace; formDesc = t.description ?? ''; formEnabled = t.enabled;
    formType = t.config.type;
    const c = t.config;
    if (c.type === 'webhook') { formPath = c.path; formMethod = c.method ?? 'POST'; formMsgTemplate = c.message_template ?? ''; }
    else if (c.type === 'cron') { formCron = c.expression; formCronMsg = c.message ?? ''; formTimezone = c.timezone ?? ''; }
    else if (c.type === 'adb_topic') { formTopicRes = c.resource; formTopic = c.topic; formTopicFilter = c.filter ? JSON.stringify(c.filter) : ''; }
    else { formEventSource = c.source; formEventFilter = typeof c.filter === 'string' ? c.filter : ''; }
    formTargets = t.targets;
  }

  // The chat's context: the trigger in the editor, its step, and the form as edited.
  trackActiveItem(() => view === 'editor' ? {
    kind: 'trigger', namespace: formNs, name: formName || 'new trigger',
    id: editingTrigger?.id ?? '(new)', section: step,
    text: JSON.stringify(formTrigger(), null, 2), rev: formRev,
  } : null);

  function closeEditor() { view = 'list'; saveError = ''; }

  // ── Mappings: proposed by agent_a3t_mapper or hand-written, checked, approved ──
  /// Sample bodies (JSON text) the mappings are checked on, for this editor session.
  let mapSamples = $state<string[]>([]);
  let mapRecords = $state<MappingRecord[]>([]);
  let mapLoadedFor = '';
  async function reloadMappings() {
    const id = editingTrigger?.id;
    if (!id) { mapRecords = []; return; }
    const r = await listRecords(id);
    mapRecords = r.ok ? r.records : [];
    mapLoadedFor = id;
  }
  // The trigger's records, once per trigger opened; samples start from the Test body.
  $effect(() => {
    const id = editingTrigger?.id ?? '';
    if (view !== 'editor') return;
    untrack(() => {
      if (!mapSamples.length) mapSamples = [testPayload];
      if (id !== mapLoadedFor) void reloadMappings();
    });
  });
  $effect(() => { if (view !== 'editor') untrack(() => { mapSamples = []; mapRecords = []; mapLoadedFor = ''; }); });
  const mapCtx = $derived<MapCtx>({
    triggerId: editingTrigger?.id ?? null,
    trigger: formName,
    source: formType,
    samples: mapSamples,
    records: mapRecords,
    reload: reloadMappings,
    ensureSaved: async () => {
      if (editingTrigger) return editingTrigger.id;
      await saveTrigger(true);
      if (saveError) return null;
      const found = triggers.find((t) => t.name === formName && (t.namespace || 'default') === (formNs || 'default'));
      if (found) editingTrigger = found;
      return found?.id ?? null;
    },
    persist: async () => { await saveTrigger(true); return !saveError; },
  });

  // A trigger picked in the app's list (/triggers/<id>) opens in the editor —
  // once the list is in, and again on each pick (a new panel, even the same id).
  let focusedPanel = '';
  $effect(() => {
    const panel = getActivePanel();
    const id = panel?.props?.selectedId as string | undefined;
    if (!panel || !id || loading || panel.id === focusedPanel) return;
    focusedPanel = panel.id;
    const t = triggers.find((x) => x.id === id) ?? triggers.find((x) => x.name === id);
    if (t) untrack(() => openEdit(t));
  });

  let saveError = $state('');
  /// `stay`: keep the editor open (the Test step saves before it fires).
  async function saveTrigger(stay = false) {
    const triggerObj = formTrigger();
    // The trigger's fields go at the top level, as the platform's Trigger reads them
    // (a wrapped { trigger } failed both calls, silently).
    saving = true;
    const r = editingTrigger ? await putTrigger(editingTrigger.id, triggerObj) : await createTrigger(triggerObj);
    saving = false;
    if (!r.ok) { saveError = r.error; return; }
    saveError = '';
    if (!stay) view = 'list';
    await loadTriggers();
    bumpItems();  // the Items list shows it now
  }

  async function deleteTrigger(t: Trigger) { await mcpCall('delete_trigger', { id: t.id }); deleteTarget = null; await loadTriggers(); bumpItems(); }
  async function toggleEnabled(t: Trigger) { await mcpCall(t.enabled ? 'pause_trigger' : 'enable_trigger', { id: t.id }); await loadTriggers(); bumpItems(); }

  function typeIcon(t: string) { return t === 'webhook' ? 'W' : t === 'cron' ? 'C' : t === 'event' ? 'E' : t === 'adb_topic' ? 'T' : '?'; }
  function typeStyle(t: string) {
    return t === 'webhook' ? 'background: var(--runtime-bg, #e6f4ef); color: var(--runtime, #1a7158);'
      : t === 'cron' ? 'background: var(--control-bg, #fdf0e0); color: var(--control, #c97c2e);'
      : 'background: var(--signal-bg, #f0ebfa); color: var(--signal, #7d4bbb);';
  }
</script>

{#if view === 'list'}
<!-- ═══════════ LIST VIEW ═══════════ -->
<!-- Page Header — same shape as Templates / Agents -->
<div class="screen-header">
  <div class="breadcrumb">Triggers <span class="sep">/</span> All</div>
  <div class="screen-title-row">
    <div class="title-block"><SubmenuToggle appType="triggers" /><h1 class="screen-title">Triggers</h1></div>
    <div class="screen-actions"><NewButton label="New trigger" accent="runtime" onclick={openCreate} /></div>
  </div>
</div>
<div class="trg-page">
  <PageBanner {...bannerFor('triggers')} />
  <!-- Pinned and latest, under the banner: the section page leads with what
       it is for, then the items you actually return to. -->
  <AppHome appType="triggers" embedded />


  {#if loading}
    <!-- the header bar shows loading -->
  {:else}
    {#snippet triggerRows(list: Trigger[])}
    <div class="trg-list">
      {#each list as t}
        <div class="trg-card" role="button" tabindex="0" onclick={() => openEdit(t)} onkeydown={(e) => { if (e.key === 'Enter') openEdit(t); }}>
          <span class="trg-icon" style={typeStyle(t.config.type)}>{typeIcon(t.config.type)}</span>
          <div class="trg-body">
            <div class="trg-row1">
              <span class="trg-name">{t.name}</span>
              <ProjectPill project={(t as any).project ?? t.namespace} />
              <span class="trg-badge">{t.config.type}</span>
              {#if t.config.type === 'webhook'}<span class="trg-detail">{t.config.method ?? 'POST'} {t.config.path}</span>{/if}
              {#if t.config.type === 'cron'}<span class="trg-detail">{t.config.expression}</span>{/if}
              {#if t.config.type === 'event'}<span class="trg-detail">{t.config.source}</span>{/if}
              {#if t.config.type === 'adb_topic'}<span class="trg-detail">{t.config.resource} · {t.config.topic}</span>{/if}
            </div>
            {#if t.targets.length > 0}
              <div class="trg-targets">
                {#each t.targets as tgt}
                  <span class="trg-target">{targetLabel(tgt)}{tgt.input_mapping || tgt.message ? ' · mapped' : ''}</span>
                {/each}
              </div>
            {/if}
          </div>
          <span class="trg-status" class:active={t.enabled}>{t.enabled ? 'active' : 'paused'}</span>
          <div class="trg-actions" onclick={(e) => e.stopPropagation()}>
            {#if canWriteIn(nsOfTrigger(t))}
            <button class="trg-act" onclick={() => toggleEnabled(t)} title={t.enabled ? 'Pause' : 'Resume'}>
              {#if t.enabled}<svg width="12" height="12" viewBox="0 0 16 16" fill="currentColor"><rect x="3" y="2" width="3.5" height="12" rx="1"/><rect x="9.5" y="2" width="3.5" height="12" rx="1"/></svg>
              {:else}<svg width="12" height="12" viewBox="0 0 16 16" fill="currentColor"><polygon points="3,2 13,8 3,14"/></svg>{/if}
            </button>
            {#if !isPlatform((t as any).project ?? t.namespace)}
              <button class="trg-act" onclick={() => openPublish({ kind: 'trigger', from: t.namespace, ref: t.id, label: t.name })} title="Publish a release others can use">↗</button>
            {/if}
            <button class="trg-act danger" onclick={() => deleteTarget = t} title="Delete">
              <svg width="12" height="12" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5"><path d="M12 4L4 12M4 4l8 8"/></svg>
            </button>
            {/if}
          </div>
        </div>
      {/each}
    </div>
    {/snippet}

    <FoldSection title="My Triggers" count={myTriggers.length} hint="Start this project's flows">
      {@render triggerRows(myTriggers)}
    </FoldSection>
    {#if myTriggers.length === 0}
      <div class="trg-empty">
        <p>No triggers in this project yet.</p>
        <button class="trg-create-btn" onclick={openCreate}>+ Create your first trigger</button>
      </div>
    {/if}
    <FoldSection title="Platform Triggers" count={sharedTriggers.length} hint="The platform's own (shown with “Show platform”)">
      {@render triggerRows(sharedTriggers)}
    </FoldSection>
    <ExposedSections kind="trigger" noun="Triggers" />
  {/if}
</div>

{/if}

<!-- ═══════════ EDITOR (SlidePanel pop-in) ═══════════ -->
<!-- Open in Studio once it is saved: the Studio opens a trigger by its id. -->
<SlidePanel open={view === 'editor'} onclose={closeEditor} onsave={trgRO ? undefined : () => saveTrigger()}
  onopenineditor={editingTrigger ? () => openInStudio('trigger', editingTrigger!.namespace || 'default', editingTrigger!.name, editingTrigger!.id) : undefined}>
  <!-- The same frame as an agent's or a template's details: header and steps
       (PropertiesTree), then the step's page at the details width. -->
  <div class="panel-layout">
  <PropertiesTree icon={(formName || 'NT').slice(0, 2).toUpperCase()} color="#1a7158"
    name={formName || 'New trigger'} meta={`${formType} · ${formNs || '—'}`} nodes={[]}
    steps={trgSteps} wizard={!editingTrigger} activeNode={step}
    onselect={(id) => (step = id)} />
  <div class="panel-col te trg-ui">
    <!-- As an agent's details: the pane scrolls; the page inside is --detail-w wide,
         each tab a set of numbered cards (agent.css, .agt). -->
    <div class="panel-content">
    <div class="pf wide">
    <WizardHead step={trgSteps.find((x) => x.id === step)} />
    <section class="agt">
    {#if saveError}<div class="note bad" role="alert">Could not save: {saveError}</div>{/if}
    {#if trgRO}<div class="note">You can see this trigger, but not change, pause or fire it: that needs write access to project <b class="mono">{nsOfTrigger(editingTrigger!)}</b>.</div>{/if}
    <fieldset class="ro-fs" disabled={trgRO}>

    {#if step === 'details'}
      <div class="card">
        <div class="card-head"><h3>1 · Trigger</h3><span>its name, the project it belongs to, what it is for</span></div>
        <div class="fields">
          <label class="fld">Name<input type="text" bind:value={formName} placeholder="refunds" /></label>
          <label class="fld">Project
            <select bind:value={formNs}>
              {#each [...new Set([formNs, ...namespacesFor('owned')].filter(Boolean))] as ns (ns)}<option value={ns}>{ns}</option>{/each}
            </select></label>
          <label class="fld full">Description<input type="text" bind:value={formDesc} placeholder="What it is for, e.g. refund requests from the shop" /></label>
        </div>
      </div>
      <div class="card">
        <div class="card-head"><h3>2 · Status</h3><span>whether it fires, on what, and where it sends</span></div>
        <ul class="rows">
          <li><span class="lbl">State</span>
            <span class="val"><span class="dot" class:ok={formEnabled}></span> {formEnabled ? 'Active' : 'Paused'}</span>
            <button class="btn2" onclick={() => (formEnabled = !formEnabled)}>{formEnabled ? 'Pause' : 'Resume'}</button>
            <span class="sub">{formEnabled ? 'It fires on every event below.' : 'Paused: calls are answered "paused" and nothing runs.'}{editingTrigger ? ' Saved with Save.' : ''}</span></li>
          <li><span class="lbl">Fires on</span>
            <span class="val mono">{firesOn || '—'}</span>
            <button class="btn2" onclick={() => (step = 'source')}>Change</button>
            <span class="sub">{SOURCE_NAME[formType]}</span></li>
          <li><span class="lbl">Sends to</span>
            <span class="val">{formTargets.length ? formTargets.map(targetLabel).join(', ') : 'nothing yet'}</span>
            <button class="btn2" onclick={() => (step = 'destinations')}>Change</button>
            <span class="sub">{formTargets.length} target{formTargets.length === 1 ? '' : 's'}{mappedCount ? `, ${mappedCount} with a mapping` : ''}</span></li>
        </ul>
      </div>
    {/if}

    {#if step === 'source'}
      <div class="card">
        <div class="card-head"><h3>1 · What fires it</h3><span>one source per trigger</span></div>
        <div class="seg src" role="tablist">
          {#each SOURCES as s (s)}<button role="tab" class:on={formType === s} aria-selected={formType === s} onclick={() => (formType = s)}>{SOURCE_NAME[s]}</button>{/each}
        </div>
        {#if formType === 'webhook'}
          <div class="fields">
            <label class="fld">Path<input type="text" class="mono" bind:value={formPath} placeholder="/shop/refund" /></label>
            <label class="fld">Method<select bind:value={formMethod}><option>POST</option><option>PUT</option><option>GET</option></select></label>
          </div>
          {#if formPath}
            <div class="ep"><span class="verb">{formMethod}</span><span class="path">{endpointUrl(formPath)}</span>
              <button class="btn2" onclick={() => cpUrl()}>{copiedUrl ? 'Copied' : 'Copy'}</button></div>
            <p class="dim">Callers POST JSON here with a token: <span class="mono">Authorization: Bearer &lt;token&gt;</span>.</p>
          {/if}
        {:else if formType === 'cron'}
          <div class="fields">
            <label class="fld">Schedule <small>cron: minute hour day month weekday</small><input type="text" class="mono" bind:value={formCron} placeholder="*/5 * * * *" /></label>
            <label class="fld">Timezone<input type="text" bind:value={formTimezone} placeholder="UTC" /></label>
            <label class="fld full">Message <small>optional; what .message holds</small><input type="text" bind:value={formCronMsg} placeholder="nightly report" /></label>
          </div>
          <p class="dim">Once per matching minute, however many FlowRs run it.</p>
        {:else if formType === 'adb_topic'}
          <div class="fields">
            <label class="fld">ADB resource<input type="text" class="mono" bind:value={formTopicRes} placeholder="a3t/adb" /></label>
            <label class="fld">Topic<input type="text" class="mono" bind:value={formTopic} placeholder="orders" /></label>
            <label class="fld full">Only messages where <small>optional; a JSON match</small><input type="text" class="mono" bind:value={formTopicFilter} placeholder={'{"kind": "refund"}'} /></label>
          </div>
          <div class="note">Once per message published to the topic, one run per message. <b>Kafka, Postgres and Pulsar</b> will arrive this way: listeners (FlowR plugins) publish their events to an ADB topic, and a trigger like this one starts flows and agents from it. Planned, not built yet.</div>
        {:else}
          <div class="fields">
            <label class="fld full">Event type<input type="text" class="mono" bind:value={formEventSource} placeholder="system.workflow.completed" /></label>
          </div>
          <p class="dim">The platform's own events, e.g. a flow completing.</p>
        {/if}
      </div>
      <div class="card">
        <div class="card-head"><h3>2 · What the event holds</h3><span>what every mapping in Send reads</span></div>
        <pre class="code">{EVENT_SHAPE[formType]}</pre>
      </div>
    {/if}

    {#if step === 'destinations'}
      <TriggerTargets bind:targets={formTargets} bind:samples={mapSamples} ctx={mapCtx} flows={flowOptions} agents={agentOptions} source={formType} trigger={formName} />
    {/if}

    {#if step === 'yaml'}
      <div class="te-yaml">{#key yamlRev}
        <FlowEditorPane part="main" initialYaml={yamlDraft} onchange={applyYaml}>
          {#snippet toolbar()}<span class="grow"></span><span class="ver mono">{formNs || 'default'}/{formName || 'new trigger'}</span>
            {#if editingTrigger}<button class="link" onclick={() => openInStudio('trigger', editingTrigger!.namespace || 'default', editingTrigger!.name, editingTrigger!.id)}>Edit in Studio →</button>{/if}{/snippet}
        </FlowEditorPane>
      {/key}</div>
      {#if yamlIssues.length}<div class="note bad"><b>Not applied yet:</b><ul class="notes">{#each yamlIssues as i (i)}<li>{i}</li>{/each}</ul></div>{/if}
    {/if}

    {#if step === 'test'}
      {#if formType === 'webhook'}
        <div class="card">
          <div class="card-head"><h3>1 · Fire it</h3><span>as you, with this body{editingTrigger ? '' : ' — it is saved first'}</span></div>
          {#if formPath}<div class="ep"><span class="verb">{formMethod}</span><span class="path">{endpointUrl(formPath)}</span></div>{/if}
          <label class="fld">Body <small>JSON; it is <span class="mono">.body</span> in the mappings</small>
            <textarea rows="5" bind:value={testPayload} spellcheck="false"></textarea></label>
          <div class="acts">
            <button class="btn2 primary" onclick={runTest} disabled={testRunning || !formTargets.length || !formPath}>{testRunning ? 'Firing…' : 'Fire it'}</button>
            {#if !formTargets.length}<span class="dim">Add a target in Send first.</span>{/if}
            <button class="btn2 quiet" disabled={!testPayload.trim() || mapSamples.includes(testPayload)} onclick={() => (mapSamples = [...mapSamples.filter((x) => x.trim()), testPayload])}>{mapSamples.includes(testPayload) ? 'Kept as a sample' : 'Keep as a sample'}</button>
          </div>
          {#if testError}<div class="note bad">{testError}</div>{/if}
        </div>
        <div class="card">
          <div class="card-head"><h3>2 · What ran</h3><span>{testRuns.length ? `${testRuns.length} run${testRuns.length === 1 ? '' : 's'} started` : 'each target\'s run, with what it got and gave'}</span></div>
          {#if testRuns.length}
            <ul class="tl">
              {#each testRuns as r (r.instance_id)}
                <li class="ev" class:err={r.status === 'failed'}>
                  <span class="ic"><span class="dot" class:ok={r.status === 'completed'} class:err={r.status === 'failed'} class:warn={r.status === 'running'}></span></span>
                  <div>
                    <div class="t">{r.target.replace(/^workflow:/, 'flow ').replace(/^a3t_agent:/, 'agent ')}</div>
                    <div class="m mono">{r.instance_id}{r.duration_ms != null ? ` · ${r.duration_ms} ms` : ''}</div>
                    {#if r.input !== undefined}<div class="m">It got (the mapped input):</div><pre class="code out">{JSON.stringify(r.input, null, 2)}</pre>{/if}
                    {#if r.error}<div class="m err">{r.error}</div>{/if}
                    {#if r.output !== undefined}<div class="m">It gave:</div><pre class="code out">{JSON.stringify(r.output, null, 2)}</pre>{/if}
                  </div>
                  <span class="tag" class:prod={r.status === 'completed'}>{r.status}</span>
                </li>
              {/each}
            </ul>
          {:else if testReply}
            <pre class="code out">{JSON.stringify(testReply, null, 2)}</pre>
          {:else}
            <p class="empty">Nothing fired yet.</p>
          {/if}
        </div>
      {:else}
        <div class="card">
          <div class="card-head"><h3>Test</h3><span>{SOURCE_NAME[formType]}</span></div>
          <p class="dim">{formType === 'cron' ? 'A schedule fires on its own, each minute it matches, once saved and active.' : formType === 'adb_topic' ? 'It fires on each message published to its topic, once saved and active.' : 'It fires on each matching platform event, once saved and active.'} Its runs show in Executions.</p>
        </div>
      {/if}
    {/if}
    </fieldset>
    </section>
    </div>
    </div>
    <WizardNav steps={trgSteps} active={step} wizard={!editingTrigger} onselect={(id) => (step = id)}
      final={trgRO ? undefined : () => saveTrigger()} finalLabel={editingTrigger ? 'Save' : 'Create trigger'} finalBusy={saving} busyLabel={editingTrigger ? 'Saving…' : 'Creating…'} />
  </div>
  </div>
</SlidePanel>

<!-- Was passed `name`/`type` and no `open`, so the dialog never appeared and a
     trigger could not be deleted from here. -->
<DeleteConfirm open={!!deleteTarget} entityType="Trigger" entityName={deleteTarget?.name ?? ''}
  onconfirm={() => deleteTrigger(deleteTarget!)} oncancel={() => deleteTarget = null} />

<style>
  .grow { flex: 1 1 auto; }
  .mono { font-family: var(--font-mono); font-size: 0.92em; }
  /* As an agent's Build › Code: the designer pane, framed. */
  .te-yaml { height: max(480px, calc(100vh - 300px)); border: 1px solid var(--border); border-radius: 10px; overflow: hidden; display: flex; flex-direction: column; }
  .te-yaml > :global(*) { flex: 1; min-height: 0; }
  .te-yaml .ver { font-size: 12px; color: var(--text-3); min-width: 0; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .te-yaml .link { border: 0; background: none; padding: 0 8px; color: var(--spec); cursor: pointer; font: 500 12.5px var(--font-sans); white-space: nowrap; flex: 0 0 auto; }
  .panel-layout { display: grid; grid-template-columns: 280px 1fr; height: 100%; }
  .panel-col { position: relative; display: flex; flex-direction: column; min-height: 0; min-width: 0; height: 100%; }
  /* ═══ LIST VIEW ═══ */
  .screen-header { padding: 18px 32px 12px; border-bottom: 1px solid var(--border); background: var(--bg-page); }
  .breadcrumb { font-size: 12px; color: var(--text-3); margin-bottom: 8px; font-family: var(--font-mono); }
  .breadcrumb .sep { color: var(--text-5); }
  .screen-title-row { display: flex; align-items: center; gap: 18px; }
  .title-block { flex: 1; display: flex; align-items: center; gap: 10px; }
  .screen-title { font-family: var(--font-serif); font-size: 24px; font-weight: 500; letter-spacing: -0.02em; }
  .screen-actions { display: flex; gap: 8px; }
  .trg-page { flex: 1; min-height: 0; overflow-y: auto; padding: 20px 32px 40px; }
  .trg-create-btn { display: flex; align-items: center; gap: 5px; padding: 6px 14px; border: 1px solid var(--runtime); border-radius: 6px; background: var(--runtime-bg); color: var(--runtime); font-size: 12px; font-weight: 550; cursor: pointer; }
  .trg-create-btn:hover { background: var(--runtime); color: white; }
  .trg-empty { padding: 60px 20px; text-align: center; color: var(--text-4); font-size: 14px; display: flex; flex-direction: column; align-items: center; gap: 12px; }
  .trg-list { display: flex; flex-direction: column; gap: 4px; }
  .trg-card { display: flex; align-items: center; gap: 10px; padding: 10px 14px; border: 1px solid var(--border); border-radius: var(--r-sm); background: var(--bg-surface); cursor: pointer; }
  .trg-card:hover { border-color: var(--spec); }
  .trg-icon { width: 32px; height: 32px; border-radius: 6px; display: grid; place-items: center; font-size: 13px; font-weight: 700; font-family: var(--font-mono); flex-shrink: 0; }
  .trg-body { flex: 1; min-width: 0; }
  .trg-row1 { display: flex; align-items: baseline; gap: 6px; }
  .trg-name { font-weight: 600; font-size: 13px; color: var(--text-1); }
  .trg-badge { font-family: var(--font-mono); font-size: 10px; color: var(--text-3); background: var(--bg-subtle); padding: 1px 5px; border-radius: 3px; }
  .trg-detail { font-family: var(--font-mono); font-size: 11px; color: var(--text-3); }
  .trg-targets { display: flex; gap: 4px; margin-top: 4px; flex-wrap: wrap; }
  .trg-target { font-family: var(--font-mono); font-size: 10px; color: var(--spec-ink); background: var(--spec-bg); padding: 1px 6px; border-radius: 3px; }
  .trg-status { font-family: var(--font-mono); font-size: 10px; text-transform: uppercase; color: var(--text-4); }
  .trg-status.active { color: var(--runtime); }
  .trg-actions { display: flex; gap: 3px; }
  .trg-act { width: 28px; height: 28px; border: 1px solid var(--border); border-radius: 5px; background: var(--bg-surface); color: var(--text-3); cursor: pointer; display: grid; place-items: center; }
  .trg-act:hover { background: var(--bg-subtle); color: var(--text-1); }
  .trg-act.danger:hover { background: var(--err-bg); color: var(--err); border-color: var(--err); }

  /* ═══ EDITOR (inside SlidePanel) ═══ */
  .te { display: flex; flex-direction: column; height: 100%; }

  /* TOP: form — two columns side by side */
  .panel-content { overflow-y: auto; flex: 1; min-height: 0; }
  .pf { padding: 24px 32px; max-width: var(--detail-w); box-sizing: border-box; }
  @container details (max-width: 560px) { .pf { padding: 16px 14px; } }








  /* Connected targets bar */

  /* BOTTOM: canvas + divider + flow list */

  /* Side divider */

  /* Bottom divider */

  /* Canvas */

  @keyframes spin { to { transform: rotate(360deg); } }

  .ro-fs { border: 0; margin: 0; padding: 0; min-width: 0; display: contents; }
</style>
