<script lang="ts">
  import { openPublish } from '$lib/stores/publish.svelte.ts';
  import ExposedSections from '$lib/components/shared/ExposedSections.svelte';
  import { inContext, namespacesFor, isPlatform, itemProject, getProject } from '$lib/stores/context.svelte.ts';
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
  import TriggersGuide from '$lib/components/triggers/TriggersGuide.svelte';
  import TriggerTargets from '$lib/components/triggers/TriggerTargets.svelte';
  import FlowEditorPane from '$lib/components/editor/FlowEditorPane.svelte';
  import PropertiesTree from '$lib/components/shared/PropertiesTree.svelte';
  import { toYaml, fromYaml, BODY_OF, targetLabel, type StoredTarget, type SourceKind } from '$lib/components/triggers/triggerSpec';
  import { agentTool, getAccessToken } from '$lib/services/api';

  import SlidePanel from '$lib/components/shared/SlidePanel.svelte';
  import PageBanner from '$lib/components/shared/PageBanner.svelte';
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
  let testResult = $state<any>(null);
  let testError = $state('');
  let showOutput = $state(false);

  async function runTest() {
    if (formType !== 'webhook' || !formPath) return;
    testRunning = true; testResult = null; testError = ''; showOutput = true;

    // Step 1: save trigger first if it hasn't been saved
    if (!editingTrigger) {
      await saveTrigger(true);
      if (saveError) { testError = `Not saved: ${saveError}`; testRunning = false; return; }
      // The trigger just made: from now on, edits save to it.
      const found = triggers.find(t => t.name === formName && (t.namespace || 'default') === (formNs || 'default'));
      if (found) editingTrigger = found;
    }

    // Step 2: fire the webhook
    try {
      let payload: any;
      try { payload = JSON.parse(testPayload); } catch { testError = 'Invalid JSON payload'; testRunning = false; return; }

      const base = API_BASE;
      const path = formPath.startsWith('/') ? formPath : '/' + formPath;
      const webhookRes = await fetch(`${base}/webhook${path}`, {
        method: 'POST',
        // As you: an agent's webhook takes no call without a token.
        headers: { 'Content-Type': 'application/json', Authorization: `Bearer ${getAccessToken() ?? ''}` },
        body: JSON.stringify(payload),
      });
      const webhookData = await webhookRes.json();

      if (!webhookRes.ok || webhookData.error) {
        // A mapping that fails, a target that refuses: say which and why.
        const why = (webhookData.failed ?? []).map((f: any) => f.error).join('\n');
        testError = why ? `Nothing ran — ${why}` : (webhookData.error ?? `HTTP ${webhookRes.status}`);
        testRunning = false;
        return;
      }

      if (!webhookData.executions?.length) {
        // Nothing to follow: show what the platform answered (a paused target, a URL target).
        testError = webhookData.delivered ? '' : 'It fired, but nothing ran: no target took it.';
        testResult = { trigger: webhookData };
        testRunning = false;
        return;
      }

      // Step 3: poll for execution result
      const instanceId = webhookData.executions[0].instance_id;
      testResult = { phase: 'running', instanceId, trigger: webhookData };

      for (let i = 0; i < 30; i++) {
        await new Promise(r => setTimeout(r, 1000));
        const statusRes = await mcpCall('get_execution_status', { instance_id: instanceId });
        if (!statusRes.ok) continue;
        const d = typeof statusRes.data === 'string' ? JSON.parse(statusRes.data) : statusRes.data;
        if (d.status === 'completed' || d.status === 'Completed') {
          testResult = { phase: 'completed', instanceId, duration: d.duration_us, output: d.output, trigger: webhookData };
          testRunning = false;
          return;
        }
        if (d.status === 'failed' || d.status === 'Failed') {
          testResult = { phase: 'failed', instanceId, trigger: webhookData };
          testError = d.error ?? 'Execution failed';
          testRunning = false;
          return;
        }
        testResult = { phase: 'running', instanceId, poll: i + 1, trigger: webhookData };
      }
      testError = 'Execution timed out (30s)';
      testRunning = false;
    } catch (e) {
      testError = String(e);
      testRunning = false;
    }
  }

  // ── Resize state ──
  let outputHeight = $state(200);
  let draggingBottom = $state(false);
  let editorContainer: HTMLDivElement;

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
    { id: 'yaml', icon: '<path d="M5 3L1 8l4 5M11 3l4 5-4 5"/>', label: 'YAML', hint: 'The whole trigger as YAML — the same text the Studio edits' },
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

  // ── Output panel resize (height) ──
  function onBottomDragStart(e: MouseEvent) {
    draggingBottom = true; e.preventDefault();
    const onMove = (ev: MouseEvent) => {
      if (!editorContainer) return;
      const rect = editorContainer.getBoundingClientRect();
      const fromBottom = rect.bottom - ev.clientY;
      outputHeight = Math.max(80, Math.min(500, fromBottom));
    };
    const onUp = () => { draggingBottom = false; window.removeEventListener('mousemove', onMove); window.removeEventListener('mouseup', onUp); };
    window.addEventListener('mousemove', onMove); window.addEventListener('mouseup', onUp);
  }


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
  <PageBanner headline="Start flows and agents from events." highlightWord="events"
    subtitle="A webhook call, a schedule, a message on an ADB topic or a platform event — each firing goes to the flows and agents you name, with the input you map."
    description="A trigger is YAML: what fires it, and where each firing goes. Write it here or in the Studio; pause or rewire it without changing the flow."
    accent="runtime" />
  <div class="trg-how"><TriggersGuide oncreate={openCreate} /></div>
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
<SlidePanel open={view === 'editor'} onclose={closeEditor} onsave={() => saveTrigger()}
  onopenineditor={editingTrigger ? () => openInStudio('trigger', editingTrigger!.namespace || 'default', editingTrigger!.name, editingTrigger!.id) : undefined}>
  <!-- The same frame as an agent's or a template's details: header and steps
       (PropertiesTree), then the step's page at the details width. -->
  <div class="panel-layout">
  <PropertiesTree icon={(formName || 'NT').slice(0, 2).toUpperCase()} color="#1a7158"
    name={formName || 'New trigger'} meta={`${formType} · ${formNs || '—'}`} nodes={[]}
    steps={trgSteps} wizard={!editingTrigger} activeNode={step}
    onselect={(id) => { step = id; if (id === 'test') showOutput = true; }} />
  <div class="panel-col te trg-ui" bind:this={editorContainer}>
    {#if saveError}<div class="te-error" role="alert">Could not save: {saveError}</div>{/if}
    <!-- Details / Source -->
    {#if step === 'details' || step === 'source'}
    <div class="te-form">
      <WizardHead step={trgSteps.find((x) => x.id === step)} />
      <div class="tf-row-top">
        {#if step === 'details'}
        <div class="tf-col">
          <div class="tf-row2">
            <label><span>Name</span><input type="text" bind:value={formName} placeholder="my-webhook" /></label>
            <label><span>Namespace</span>
              <select bind:value={formNs}>
                {#each [...new Set([formNs, ...namespacesFor('owned')].filter(Boolean))] as ns (ns)}<option value={ns}>{ns}</option>{/each}
              </select>
            </label>
          </div>
          <label class="tf-full"><span>Description</span><input type="text" bind:value={formDesc} placeholder="What this trigger does" /></label>
          <label class="tf-check"><input type="checkbox" bind:checked={formEnabled} /> Enabled</label>
        </div>
        {:else}
        <div class="tf-col">
          <div class="tf-type-tabs">
            <button class="tf-type" class:active={formType === 'webhook'} onclick={() => formType = 'webhook'}>
              <span class="tf-ti" style="background:var(--runtime-bg);color:var(--runtime);">W</span> Webhook
            </button>
            <button class="tf-type" class:active={formType === 'cron'} onclick={() => formType = 'cron'}>
              <span class="tf-ti" style="background:var(--control-bg);color:var(--control);">C</span> Cron
            </button>
            <button class="tf-type" class:active={formType === 'adb_topic'} onclick={() => formType = 'adb_topic'}>
              <span class="tf-ti" style="background:var(--spec-bg);color:var(--spec);">T</span> ADB topic
            </button>
            <button class="tf-type" class:active={formType === 'event'} onclick={() => formType = 'event'}>
              <span class="tf-ti" style="background:var(--signal-bg);color:var(--signal);">E</span> Event
            </button>
          </div>
          {#if formType === 'webhook'}
            <div class="tf-row2">
              <label><span>Path</span><input type="text" bind:value={formPath} placeholder="/agent" /></label>
              <label><span>Method</span><select bind:value={formMethod}><option>POST</option><option>GET</option><option>PUT</option></select></label>
            </div>
            {#if formPath}
              <div class="tf-ep"><span class="tf-ep-l">Endpoint</span><code>{formMethod} /webhook{formPath.startsWith('/') ? formPath : '/' + formPath}</code></div>
            {/if}
          {:else if formType === 'cron'}
            <div class="tf-row2">
              <label><span>Expression</span><input type="text" bind:value={formCron} placeholder="*/5 * * * *" /></label>
              <label><span>Timezone</span><input type="text" bind:value={formTimezone} placeholder="UTC" /></label>
            </div>
          {:else if formType === 'adb_topic'}
            <div class="tf-row2">
              <label><span>ADB resource</span><input type="text" bind:value={formTopicRes} placeholder="a3t/adb" /></label>
              <label><span>Topic</span><input type="text" bind:value={formTopic} placeholder="orders" /></label>
            </div>
            <label class="tf-full"><span>Only messages where (optional)</span><input type="text" class="mono" bind:value={formTopicFilter} placeholder={'{"kind": "refund"}'} /></label>
            <div class="tf-note">Fires once per message published to the topic, on every FlowR, one run per message. <b>Kafka, Postgres and Pulsar</b> come in this way: ADB connects to them and publishes their events to a topic, which a trigger like this one starts flows and agents from. The ADB connectors are requested from ADB (parked).</div>
          {:else if formType === 'event'}
            <label class="tf-full"><span>Source</span><input type="text" bind:value={formEventSource} placeholder="system.workflow.completed" /></label>
          {/if}
          <div class="tf-note">Its targets' expressions read <span class="mono">.body</span>: {BODY_OF[formType]}.</div>
        </div>
        {/if}
      </div>
    </div>
    {/if}

    {#if step === 'destinations'}
      <div class="te-form">
        <WizardHead step={trgSteps.find((x) => x.id === step)} />
        <TriggerTargets bind:targets={formTargets} flows={flowOptions} agents={agentOptions} source={formType} trigger={formName} />
      </div>
    {/if}

    {#if step === 'yaml'}
      <div class="te-form">
        <WizardHead step={trgSteps.find((x) => x.id === step)} />
        <p class="tf-note">The trigger as code, in the flow designer: Graph shows what fires it and where each firing goes; Code is the YAML — change it here or in the form, they are one. <span class="mono">on</span> is what fires it; <span class="mono">send</span> the targets, each with its <span class="mono">input</span> (a flow) or <span class="mono">message</span> and <span class="mono">conversation</span> (an agent), in jq over the event.</p>
        <div class="te-yaml">{#key yamlRev}
          <FlowEditorPane part="main" initialYaml={yamlDraft} onchange={applyYaml}>
            {#snippet toolbar()}<span class="grow"></span><span class="ver mono">{formNs || 'default'}/{formName || 'new trigger'}</span>
              {#if editingTrigger}<button class="link" onclick={() => openInStudio('trigger', editingTrigger!.namespace || 'default', editingTrigger!.name, editingTrigger!.id)}>Edit in Studio →</button>{/if}{/snippet}
          </FlowEditorPane>
        {/key}</div>
        {#if yamlIssues.length}<ul class="te-issues">{#each yamlIssues as i (i)}<li>{i}</li>{/each}</ul>{/if}
      </div>
    {/if}

    {#if step === 'test'}
      <div class="te-form">
        <WizardHead step={trgSteps.find((x) => x.id === step)} />
        {#if formType === 'webhook'}
          <p class="tf-note">Fires <span class="mono">{formMethod} /webhook{formPath.startsWith('/') ? formPath : '/' + formPath}</span> with this body, as you{editingTrigger ? '' : ' (it is saved first)'}; each target runs with the input its mapping makes.</p>
          <div class="row"><button class="btn2 primary" onclick={runTest} disabled={testRunning || !formTargets.length}>{testRunning ? 'Firing…' : 'Fire it'}</button>
            {#if testResult?.trigger?.executions?.length}<span class="dim">{testResult.trigger.executions.length} run{testResult.trigger.executions.length === 1 ? '' : 's'} started</span>{/if}</div>
        {:else}
          <p class="tf-note">{formType === 'cron' ? 'A schedule fires on its own, each minute its expression matches, once saved and enabled.' : formType === 'adb_topic' ? 'It fires on each message published to its topic, once saved and enabled.' : 'It fires on each matching platform event, once saved and enabled.'} Its runs show in Executions.</p>
        {/if}
      </div>
    {/if}

    <!-- Output panel -->
    {#if step === 'test' && formType === 'webhook'}
      <div class="te-bottom-divider" class:dragging-bottom={draggingBottom} onmousedown={onBottomDragStart} role="separator"></div>
      <div class="te-output" style="height: {Math.max(outputHeight, 260)}px;">
        <div class="te-out-header">
          <span class="te-out-title">Test</span>
          {#if testResult?.phase === 'completed'}<span class="te-out-badge ok">completed</span>{/if}
          {#if testResult?.phase === 'failed'}<span class="te-out-badge err">failed</span>{/if}
          {#if testResult?.phase === 'running'}<span class="te-out-badge run">running ({testResult.poll ?? 0}s)</span>{/if}
          {#if testResult?.instanceId}<span class="te-out-id">{testResult.instanceId}</span>{/if}
          <span style="flex:1"></span>

        </div>
        <div class="te-out-split">
          <!-- INPUT -->
          <div class="te-out-pane">
            <div class="te-out-pane-label">BODY IT POSTS</div>
            <textarea class="te-out-editor" bind:value={testPayload} placeholder='&#123;"message": "Hello"&#125;' spellcheck="false"></textarea>
          </div>
          <!-- OUTPUT -->
          <div class="te-out-pane">
            <div class="te-out-pane-label">OUTPUT</div>
            <div class="te-out-pane-body">
              {#if testError}
                <div class="te-out-err">{testError}</div>
              {/if}
              {#if testResult?.output}
                <pre class="te-out-json">{JSON.stringify(testResult.output, null, 2)}</pre>
              {:else if testResult?.trigger}
                <pre class="te-out-json">{JSON.stringify(testResult.trigger, null, 2)}</pre>
              {:else if testRunning}
                <div class="te-out-empty">Waiting for result...</div>
              {:else}
                <div class="te-out-empty">Click Test to fire the trigger</div>
              {/if}
            </div>
          </div>
        </div>
      </div>
    {/if}
    <WizardNav steps={trgSteps} active={step} wizard={!editingTrigger} onselect={(id) => { step = id; if (id === 'test') showOutput = true; }}
      final={() => saveTrigger()} finalLabel={editingTrigger ? 'Save' : 'Create trigger'} finalBusy={saving} busyLabel={editingTrigger ? 'Saving…' : 'Creating…'} />
  </div>
  </div>
</SlidePanel>

<!-- Was passed `name`/`type` and no `open`, so the dialog never appeared and a
     trigger could not be deleted from here. -->
<DeleteConfirm open={!!deleteTarget} entityType="Trigger" entityName={deleteTarget?.name ?? ''}
  onconfirm={() => deleteTrigger(deleteTarget!)} oncancel={() => deleteTarget = null} />

<style>
  .tf-note .link { border: 0; background: none; padding: 0; color: var(--spec); cursor: pointer; font: inherit; }
  .trg-how { margin: 16px 0 20px; }
  .grow { flex: 1 1 auto; }
  .mono { font-family: var(--font-mono); font-size: 0.92em; }
  /* As an agent's Build › Code: the designer pane, framed. */
  .te-yaml { height: max(480px, calc(100vh - 380px)); margin-top: 12px; border: 1px solid var(--border); border-radius: 10px; overflow: hidden; display: flex; flex-direction: column; }
  .te-yaml > :global(*) { flex: 1; min-height: 0; }
  .te-yaml .ver { font-size: 12px; color: var(--text-3); }
  .te-yaml .link { border: 0; background: none; padding: 0 8px; color: var(--spec); cursor: pointer; font: 500 12.5px var(--font-sans); }
  .panel-layout { display: grid; grid-template-columns: 280px 1fr; height: 100%; }
  .panel-col { position: relative; display: flex; flex-direction: column; min-height: 0; min-width: 0; height: 100%; }
  @container details (max-width: 560px) { .te-form { padding: 16px 14px; } }
  .te-issues { margin: 6px 0 0; padding-left: 18px; font-size: 12.5px; color: var(--err); }
  .tf-note { font-size: 12.5px; color: var(--text-3); line-height: 1.55; margin: 6px 0 0; }
  .te-form .row { display: flex; align-items: center; gap: 10px; margin-top: 10px; }
  .te-form .dim { font-size: 12.5px; color: var(--text-3); }
  .te-form .mono, .tf-note .mono { font-family: var(--font-mono); font-size: 0.92em; }
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
  .te-note { padding: 10px 16px; font-size: 12.5px; color: var(--text-3); background: var(--bg-subtle); border-top: 1px solid var(--border); }

  /* TOP: form — two columns side by side */
  .te-form { padding: 24px 32px; flex: 1; min-height: 0; overflow-y: auto; max-width: var(--detail-w); }
  .tf-row-top { display: block; max-width: 640px; }
  .tf-col { display: flex; flex-direction: column; gap: 14px; }

  .tf-section-first { font-size: 9px; font-weight: 700; color: var(--text-3); letter-spacing: 0.06em; margin-bottom: 6px; }

  .te-form label { display: flex; flex-direction: column; gap: 6px; }
  .te-form label span { font-size: 10.5px; font-weight: 600; color: var(--text-3); text-transform: uppercase; letter-spacing: 0.05em; }
  .te-form input, .te-form select { padding: 9px 12px; border: 1px solid var(--border); border-radius: var(--r-sm, 6px); background: var(--bg-surface); color: var(--text-1); font-size: 13.5px; font-family: var(--font-sans); outline: none; }
  .te-form input:focus, .te-form select:focus { border-color: var(--spec); }

  .tf-row2 { display: grid; grid-template-columns: 1fr 1fr; gap: 14px; }
  .tf-full { margin: 0; }

  .tf-type-tabs { display: flex; gap: 8px; }
  .tf-type { display: flex; align-items: center; gap: 8px; height: 36px; padding: 0 14px; border: 1px solid var(--border); border-radius: var(--r-sm, 6px); background: var(--bg-surface); color: var(--text-2); font-size: 13px; font-weight: 500; cursor: pointer; }
  .tf-type.active { border-color: var(--spec); background: var(--spec-bg); color: var(--spec-ink); }
  .tf-ti { width: 16px; height: 16px; border-radius: 3px; display: grid; place-items: center; font-size: 8px; font-weight: 700; font-family: var(--font-mono); }

  .tf-ep { background: var(--bg-subtle); border: 1px solid var(--border); border-radius: var(--r-sm, 6px); padding: 10px 12px; }
  .tf-ep-l { font-size: 10.5px; font-weight: 600; color: var(--text-4); text-transform: uppercase; letter-spacing: 0.05em; display: block; margin-bottom: 4px; }
  .tf-ep code { font-family: var(--font-mono); font-size: 12.5px; color: var(--runtime); }

  .tf-check { flex-direction: row !important; align-items: center; gap: 8px; font-size: 13px; color: var(--text-2); }
  .tf-check input { accent-color: var(--spec); }

  .te-error { margin: 8px 12px 0; padding: 8px 10px; border-radius: var(--r-sm); background: var(--err-bg, #fde8e8); color: var(--err, #b42318); font-size: 12px; }

  /* Connected targets bar */
  .te-targets-bar { display: flex; align-items: center; gap: 6px; padding: 6px 12px; border-bottom: 1px solid var(--border); background: var(--bg-subtle); flex-wrap: wrap; flex-shrink: 0; }
  .te-tb-label { font-size: 10px; font-weight: 600; color: var(--text-3); text-transform: uppercase; letter-spacing: 0.04em; }
  .te-tb-chip { display: inline-flex; align-items: center; gap: 4px; padding: 3px 6px 3px 8px; background: var(--spec-bg); border: 1px solid var(--spec-bg); border-radius: 4px; font-family: var(--font-mono); font-size: 11px; color: var(--spec-ink); font-weight: 550; }
  .te-tb-rm { width: 16px; height: 16px; border: none; border-radius: 3px; background: none; color: var(--text-4); cursor: pointer; display: grid; place-items: center; flex-shrink: 0; }
  .te-tb-rm:hover { color: var(--err); background: var(--err-bg); }

  /* BOTTOM: canvas + divider + flow list */
  .te-bottom { flex: 1; display: grid; overflow: hidden; min-height: 0; }
  .te-bottom.dragging-side { user-select: none; cursor: col-resize; }

  /* Side divider */
  .te-side-divider { width: 5px; cursor: col-resize; background: var(--border); transition: background 0.15s; }
  .te-side-divider:hover, .te-bottom.dragging-side .te-side-divider { background: var(--spec, #5530a8); }

  /* Bottom divider */
  .te-bottom-divider { height: 5px; cursor: row-resize; background: var(--border); transition: background 0.15s; flex-shrink: 0; }
  .te-bottom-divider:hover, .te-bottom-divider.dragging-bottom { background: var(--spec, #5530a8); }

  /* Canvas */
  .te-canvas { position: relative; overflow: hidden; }
  .te-canvas-hint { position: absolute; bottom: 16px; left: 50%; transform: translateX(-50%); font-size: 12px; color: var(--text-4); background: var(--bg-surface); padding: 6px 14px; border-radius: 6px; border: 1px dashed var(--border); pointer-events: none; z-index: 5; }

  /* Flow/Agent list */
  .te-flowlist { display: flex; flex-direction: column; overflow: hidden; background: var(--bg-surface); }
  .te-fl-header { padding: 6px 8px; border-bottom: 1px solid var(--border); }
  .te-fl-tabs { display: flex; gap: 2px; margin-bottom: 4px; }
  .te-fl-tab { padding: 3px 8px; border: none; border-radius: 4px; background: none; color: var(--text-4); font-size: 10px; font-weight: 550; cursor: pointer; }
  .te-fl-tab.active { background: var(--bg-subtle); color: var(--text-1); }
  .te-fl-search { display: flex; align-items: center; gap: 4px; padding: 3px 6px; border: 1px solid var(--border); border-radius: 4px; background: var(--bg-page); }
  .te-fl-search input { border: none; background: none; flex: 1; font-size: 11px; outline: none; color: var(--text-1); padding: 0; }
  .te-fl-list { flex: 1; overflow-y: auto; }
  .te-fl-item { display: flex; align-items: center; gap: 6px; padding: 7px 8px; border-bottom: 1px solid var(--border); cursor: grab; transition: background 0.1s; }
  .te-fl-item:hover { background: var(--bg-subtle); }
  .te-fl-item.connected { background: var(--runtime-bg); cursor: default; }
  .te-fl-item:active:not(.connected) { cursor: grabbing; }
  .te-fl-drag { width: 14px; flex-shrink: 0; color: var(--text-5); display: grid; place-items: center; }
  .te-fl-icon { width: 20px; height: 20px; border-radius: 4px; display: grid; place-items: center; font-size: 9px; font-weight: 700; font-family: var(--font-mono); flex-shrink: 0; background: var(--spec-bg); color: var(--spec-ink); }
  .te-fl-icon.agent { background: var(--signal-bg); color: var(--signal); }
  .te-fl-info { flex: 1; min-width: 0; }
  .te-fl-name { font-family: var(--font-mono); font-size: 11px; font-weight: 550; color: var(--text-1); display: block; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .te-fl-ns { font-size: 9px; color: var(--text-4); }
  .te-fl-linked { font-size: 8px; color: var(--runtime); font-weight: 600; text-transform: uppercase; flex-shrink: 0; }
  .te-fl-empty { padding: 20px; text-align: center; color: var(--text-4); font-size: 11px; }

  /* Test button (floating on canvas) */
  .te-test-btn {
    position: absolute; top: 12px; right: 12px; z-index: 5;
    display: flex; align-items: center; gap: 5px;
    padding: 7px 16px; border: 1px solid var(--runtime); border-radius: 6px;
    background: var(--runtime); color: white; font-size: 12px; font-weight: 600;
    cursor: pointer; box-shadow: 0 2px 8px rgba(26,113,88,0.25); transition: all 0.12s;
  }
  .te-test-btn:hover:not(:disabled) { opacity: 0.9; transform: translateY(-1px); }
  .te-test-btn:disabled { opacity: 0.6; cursor: default; }
  .te-test-btn.running { background: var(--control); border-color: var(--control); }
  .te-test-spin { width: 12px; height: 12px; border: 2px solid rgba(255,255,255,0.3); border-top-color: white; border-radius: 50%; animation: spin 0.8s linear infinite; }
  @keyframes spin { to { transform: rotate(360deg); } }

  /* Output panel (bottom) */
  .te-output { flex-shrink: 0; background: var(--bg-surface); display: flex; flex-direction: column; overflow: hidden; }

  .te-out-header { display: flex; align-items: center; gap: 6px; padding: 6px 12px; border-bottom: 1px solid var(--border); flex-shrink: 0; }
  .te-out-title { font-size: 11px; font-weight: 600; color: var(--text-2); text-transform: uppercase; letter-spacing: 0.04em; }
  .te-out-badge { font-size: 9px; font-family: var(--font-mono); padding: 1px 6px; border-radius: 3px; font-weight: 600; text-transform: uppercase; }
  .te-out-badge.ok { background: var(--runtime-bg); color: var(--runtime); }
  .te-out-badge.err { background: var(--err-bg, #fde8e8); color: var(--err, #dc2626); }
  .te-out-badge.run { background: var(--control-bg); color: var(--control); }
  .te-out-id { font-family: var(--font-mono); font-size: 9px; color: var(--text-4); }
  .te-out-close { width: 22px; height: 22px; border: 1px solid var(--border); border-radius: 4px; background: var(--bg-surface); color: var(--text-4); cursor: pointer; display: grid; place-items: center; flex-shrink: 0; }
  .te-out-close:hover { background: var(--bg-subtle); color: var(--text-1); }

  /* Split: input | output */
  .te-out-split { flex: 1; display: grid; grid-template-columns: 1fr 1fr; overflow: hidden; }
  .te-out-pane { display: flex; flex-direction: column; overflow: hidden; }
  .te-out-pane:first-child { border-right: 1px solid var(--border); }
  .te-out-pane-label { font-size: 9px; font-weight: 700; color: var(--text-4); text-transform: uppercase; letter-spacing: 0.05em; padding: 5px 10px; background: var(--bg-subtle); border-bottom: 1px solid var(--border); flex-shrink: 0; }
  .te-out-editor {
    flex: 1; padding: 8px 10px; border: none; background: var(--bg-page); color: var(--text-1);
    font-family: var(--font-mono); font-size: 11px; resize: none; outline: none;
  }
  .te-out-pane-body { flex: 1; overflow-y: auto; padding: 8px 10px; }
  .te-out-err { color: var(--err); font-size: 11px; font-family: var(--font-mono); padding: 6px 8px; background: var(--err-bg, #fde8e8); border-radius: 4px; margin-bottom: 6px; }
  .te-out-json { font-family: var(--font-mono); font-size: 11px; color: var(--text-1); background: var(--bg-page); border: 1px solid var(--border); border-radius: 4px; padding: 8px; margin: 0; white-space: pre-wrap; word-break: break-word; }
  .te-out-empty { font-size: 12px; color: var(--text-4); text-align: center; padding: 20px; }
</style>
