<script lang="ts">
  import { type WizardStep } from '$lib/components/shared/WizardSteps.svelte';
  import WizardNav from '$lib/components/shared/WizardNav.svelte';
  import WizardHead from '$lib/components/shared/WizardHead.svelte';
  import { openPublish } from '$lib/stores/publish.svelte.ts';
  import ExposedSections from '$lib/components/shared/ExposedSections.svelte';
  import SubmenuToggle from '$lib/components/shared/SubmenuToggle.svelte';
  import NewButton from '$lib/components/shared/NewButton.svelte';
  import { messages } from '$lib/core/status/messages';
  import {
    RESOURCE_CATEGORIES as CATEGORIES, CAT_COLORS, categorize, catMeta,
  } from '$lib/core/resources/categories';
  import { resources as resourcesMcp, flows as flowsMcp, mcpCall, execFlow } from '$lib/services/api';
  import DocsPanel from '$lib/shell/DocsPanel.svelte';
  import SlidePanel from '$lib/components/shared/SlidePanel.svelte';
  import PageBanner from '$lib/components/shared/PageBanner.svelte';
  import { bannerFor } from '$lib/shell/appTheme';
  import AppHome from '$lib/shell/AppHome.svelte';
  import EntityCard from '$lib/components/shared/EntityCard.svelte';
  import PropertiesTree from '$lib/components/shared/PropertiesTree.svelte';
  import DeleteConfirm from '$lib/components/shared/DeleteConfirm.svelte';
  import FlowEditorPane from '$lib/components/editor/FlowEditorPane.svelte';
  import { navigateTo, getActivePanel, openInStudio } from '$lib/stores/panels.svelte.ts';
  import { can, currentUser } from '$lib/stores/auth';
  import { onMount, tick, untrack } from 'svelte';
  import { trackActiveItem } from '$lib/stores/activeItem.svelte.ts';
  import { namespacesFor, inContext, itemProject, getProject, isPlatform } from '$lib/stores/context.svelte.ts';
  import { contextRev_, bumpItems } from '$lib/stores/apps.svelte.ts';
  import * as YAML from 'yaml';
  import '$lib/components/agent/agent.css';
  import '$lib/components/triggers/triggers.css';
  import { usedBy, type Use } from '$lib/components/resources/resourceUse';

  const canCreate = can('create', 'resource');
  const canUpdate = can('update', 'resource');

  let docsOpen = $state(false);
  let loading = $state(true);
  let error = $state('');

  // Raw data
  interface Resource {
    name: string; namespace: string; type: string; category: string;
    /// The project it shows under (itemProject); `namespace` is where it's stored.
    project?: string;
    description: string; created_at: string; created_by: string;
    tags: string[]; healthy: boolean; config?: Record<string, any>;
  }

  let allResources: Resource[] = $state([]);

  // Category mapping






  // Active category filter
  // The shell's context pane lists these fixed categories and navigates to
  // /resources/<category>, so the URL selection seeds the filter. Falls back to
  // 'all' for an unknown key rather than filtering everything away.
  let activeCategory = $state(
    (() => {
      const sel = getActivePanel()?.props?.selectedId;
      return sel && CATEGORIES.some(c => c.key === sel) ? sel : 'all';
    })(),
  );

  // Filtered resources
  let filteredResources = $derived(
    activeCategory === 'all' ? allResources : allResources.filter(r => r.category === activeCategory)
  );

  // Count per category
  let categoryCounts = $derived.by(() => {
    const counts: Record<string, number> = { all: allResources.length };
    for (const r of allResources) {
      counts[r.category] = (counts[r.category] ?? 0) + 1;
    }
    return counts;
  });

  // ── Detail panel ──
  let selected = $state<Resource | null>(null);
  /// Who relies on the open resource (agents, canvases, triggers).
  let uses = $state<Use[] | null>(null);
  async function loadUses() { const r = selected; uses = null; if (!r || r.category === 'flows') return; const u = await usedBy(`${r.namespace}/${r.name}`); if (selected === r) uses = u; }
  /// The last test of the open resource, this session: did it answer.
  let lastTest = $state<{ ok: boolean; msg: string } | null>(null);
  let showJson = $state(false);
  let flowMsg = $state('');
  /// The name typed to confirm a delete, and the delete in progress.
  let delName = $state('');
  let deleting = $state(false);
  async function deleteSelected() {
    if (!selected || delName !== selected.name) return;
    deleting = true;
    deleteTarget = { name: selected.name, namespace: selected.namespace };
    await deleteResource();
    deleting = false;
    bumpItems();
  }
  let panelTab = $state('overview');
  let testResult = $state('');
  let testing = $state(false);

  // ── LLM test state ──
  let llmInput = $state('');
  let llmMessages: { role: string; content: string }[] = $state([]);
  // ── Config (endpoint etc.) editing ──
  let llmCfg = $state<Record<string, any>>({});
  let cfgSaving = $state(false);
  let cfgMsg = $state('');

  // Secret resources selectable as an LLM's api_key_ref (namespace/name).
  let secretRefs = $derived(
    allResources
      .filter((r) => r.category === 'credentials')
      .map((r) => ({ ref: `${r.namespace}/${r.name}`, name: r.name }))
  );

  // Switch the LLM key between a secret reference (recommended) and an inline key.
  function setKeyMode(mode: string) {
    if (mode === 'secret') {
      delete llmCfg.api_key;
      if (llmCfg.api_key_ref == null) llmCfg.api_key_ref = '';
    } else {
      delete llmCfg.api_key_ref;
      if (llmCfg.api_key == null) llmCfg.api_key = '';
    }
    llmCfg = { ...llmCfg };
  }

  async function loadConfig(r: Resource) {
    cfgMsg = '';
    let cfg = r.config ?? {};
    if (!cfg || Object.keys(cfg).length === 0) {
      const full = await resourcesMcp.get(r.name, r.namespace);
      cfg = (full?.data?.config ?? full?.data ?? {}) as Record<string, any>;
    }
    if (selected && selected.name === r.name && selected.namespace === r.namespace) {
      selected.config = cfg;
      llmCfg = { ...cfg };
      cfgDraft = fieldsOf(cfg);
      cfgRev++;   // the saved config, for telling edits apart
    }
  }

  // The chat's context: this resource (or flow), the tab on screen, and its
  // config as edited — the field editor's fields while it is open, otherwise
  // the config form. A secret's value is never sent.
  let cfgRev = $state(0);
  trackActiveItem(() => selected ? {
    kind: selected.category === 'flows' ? 'flow' : 'resource',
    namespace: selected.namespace, name: selected.name, section: panelTab,
    text: selected.category === 'credentials' ? undefined
      : selected.category === 'flows' ? flowYaml
      : JSON.stringify(cfgEditing ? Object.fromEntries(cfgDraft.map((f) => [f.key, f.value])) : llmCfg, null, 2),
    rev: `${cfgRev}:${cfgEditing}`,
    ...(selected.category === 'flows' ? { dirty: flowDirty } : {}),
    // The app's Chat edits it (propose_edit): a flow's YAML, or the config in
    // whichever editor is open. A secret's value is never sent, so never edited.
    applyEdit: selected.category === 'credentials' ? undefined
      : selected.category === 'flows' ? (t: string) => { flowYaml = t; flowDirty = true; }
      : async (t: string) => {
          let o: Record<string, any>;
          try { o = JSON.parse(t); } catch { return; }
          // An LLM edits in its own form, always open.
          if (selected?.category === 'llm') { llmCfg = o; return; }
          // Others edit in the field editor: open it first (its saved fields
          // are what the change is compared with), then put the change in, so
          // it is on screen and Save is there.
          if (!cfgEditing) { startEdit(); await tick(); }
          cfgDraft = fieldsOf(o);
        },
  } : null);

  // ── Generic config editing (every resource without its own form) ──
  type CfgField = { key: string; value: string; kind: 'string' | 'number' | 'boolean' | 'json' };
  let cfgEditing = $state(false);
  let cfgDraft = $state<CfgField[]>([]);

  /// A config as the field editor's rows.
  function fieldsOf(cfg: Record<string, any>): CfgField[] {
    return Object.entries(cfg ?? {}).map(([key, v]) => ({
      key,
      kind: typeof v === 'number' ? 'number' : typeof v === 'boolean' ? 'boolean' : v !== null && typeof v === 'object' ? 'json' : 'string',
      value: v !== null && typeof v === 'object' ? JSON.stringify(v, null, 2) : String(v ?? ''),
    }));
  }

  function startEdit() {
    cfgDraft = fieldsOf(selected?.config ?? {});
    cfgMsg = '';
    cfgEditing = true;
    cfgRev++;
  }

  async function saveEdit() {
    const cfg: Record<string, any> = {};
    for (const f of cfgDraft) {
      const k = f.key.trim();
      if (!k) continue;
      if (f.kind === 'number') {
        const n = Number(f.value);
        if (f.value.trim() === '' || Number.isNaN(n)) { cfgMsg = `${k}: not a number`; return; }
        cfg[k] = n;
      } else if (f.kind === 'boolean') cfg[k] = f.value === 'true';
      else if (f.kind === 'json') {
        try { cfg[k] = JSON.parse(f.value); } catch { cfgMsg = `${k}: invalid JSON`; return; }
      } else cfg[k] = f.value;
    }
    llmCfg = cfg;
    await saveConfig();
    if (cfgMsg === 'Saved') { cfgEditing = false; cfgDraft = fieldsOf(cfg); }
  }
  /// The field form differs from what is saved.
  const cfgDirty = $derived(!!selected && JSON.stringify(cfgDraft) !== JSON.stringify(fieldsOf(selected.config ?? {})));
  const llmDirty = $derived(!!selected && JSON.stringify(llmCfg) !== JSON.stringify(selected.config ?? {}));
  /// Keys whose values are secret: shown as dots, typed as passwords.
  const SECRET_KEY = /^(value|token|password|api_key|secret|client_secret|key)$/i;
  let revealed = $state(false);
  const FIELD_LABEL: Record<string, string> = { url: 'URL', transport: 'Transport', db_path: 'Database path', backend: 'Backend', flight_url: 'Flight URL', value: 'Value', labels: 'Labels', default: 'Default', external_url: 'External URL', tools: 'Tools', auth: 'Auth', oauth2: 'OAuth2', host: 'Host', port: 'Port', database: 'Database', user: 'User', password: 'Password' };
  const fieldLabel = (k: string) => FIELD_LABEL[k] ?? k;

  async function saveConfig() {
    if (!selected) return;
    cfgSaving = true; cfgMsg = '';
    const cfg: Record<string, any> = { ...llmCfg };
    if (cfg.max_tokens != null && cfg.max_tokens !== '') cfg.max_tokens = Number(cfg.max_tokens);
    if (cfg.temperature != null && cfg.temperature !== '') cfg.temperature = Number(cfg.temperature);
    const res = await resourcesMcp.update(selected.namespace, selected.name, cfg);
    cfgSaving = false;
    if (res.ok) {
      cfgMsg = 'Saved';
      selected.config = cfg;
      llmCfg = { ...cfg };
      cfgRev++;
      allResources = allResources.map((r) => (r.namespace === selected!.namespace && r.name === selected!.name ? { ...r, config: cfg } : r));
    } else {
      cfgMsg = res.error || `Save failed (HTTP ${res.status})`;
    }
  }

  // ── DB test state ──
  let dbQuery = $state('');
  // ADB resources (type adb, legacy clawdb) are tested with AQL through the ADB driver.
  const ADB_SAMPLE_QUERY = 'RECALL FROM WORKING LIMIT 5';
  let adbQuery = $state(ADB_SAMPLE_QUERY);
  const isAdb = (r: { type: string } | null) => r?.type === 'adb' || r?.type === 'clawdb';
  let dbOperation = $state('list_tables');

  // ── MCP test state ──
  let mcpTools: { name: string; description: string }[] = $state([]);
  let mcpSelectedTool = $state('');
  let mcpToolArgs = $state('{}');
  let mcpToolResult = $state('');
  /// The server was asked for its tools (so an empty list means it has none).
  let mcpListed = $state(false);

  // ── Flow test state ──
  let flowInput = $state('{}');

  // ── Secret test state ──
  let secretConfig: Record<string, any> | null = $state(null);
  let secretRevealed = $state(false);

  // ── Create form ──
  let showCreate = $state(false);
  let createForCategory = $state('');
  let createTab = $state('details');
  let newResource = $state({ name: '', namespace: getProject() || 'default', type: '', config: '{}' });

  // Starting config for a new resource of a type; replaced when the type changes,
  // until the user edits it.
  const CONFIG_TEMPLATES: Record<string, object> = {
    adb: { url: 'http://adb.a3t.svc.cluster.local:3000' },
    lancedb: { db_path: '/data/a3t/my-db.lance' },
    postgres: { host: 'localhost', port: 5432, database: 'postgres', user: 'postgres', password: '' },
  };
  const configTemplate = (type: string) => JSON.stringify(CONFIG_TEMPLATES[type] ?? {}, null, 2);
  const untouchedConfig = (config: string) =>
    config.trim() === '{}' || Object.keys(CONFIG_TEMPLATES).some(t => config === configTemplate(t));
  let lastTemplateType = '';
  $effect(() => {
    const t = newResource.type;
    if (t !== lastTemplateType) {
      lastTemplateType = t;
      if (untouchedConfig(newResource.config)) newResource.config = configTemplate(t);
    }
  });
  let creating = $state(false);

  /// New resource, in three steps; each one needs what it asks for before Next.
  const configOk = $derived.by(() => { try { JSON.parse(newResource.config || '{}'); return true; } catch { return false; } });
  const createSteps = $derived.by((): WizardStep[] => [
    { id: 'details', label: 'Details', hint: 'Name, project, kind and type',
      ok: !!newResource.name.trim() && !!newResource.namespace && !!createForCategory && !!newResource.type,
      need: !createForCategory ? 'Choose what kind of resource it is.' : !newResource.name.trim() ? 'Name the resource.' : !newResource.namespace ? 'Pick its project.' : 'Pick its type.' },
    { id: 'config', label: 'Configuration', hint: 'How to connect to it, as JSON', ok: configOk, need: 'The configuration is not valid JSON.' },
    { id: 'review', label: 'Review & Create', hint: 'Check, then create' },
  ]);
  /// An existing resource's sections, numbered the same way (any step, any time).
  const detailSteps = $derived(detailNodes.map((n) => ({ id: n.id, label: n.label, icon: n.icon, hint: hintFor(n.id) })));
  /// What each tab is for, for the kind of resource open.
  function hintFor(tab: string): string {
    const c = selected?.category ?? '';
    if (c === 'flows') return ({ overview: 'Its name, project, version and what it does', definition: 'Its steps, as a graph and as YAML', test: 'Run it with an input and see what it gives', logs: 'Its recent runs, newest first' } as Record<string, string>)[tab] ?? '';
    const what = c === 'llm' ? 'model' : c === 'mcp' ? 'tool server' : c === 'database' ? 'database' : c === 'credentials' ? 'secret' : 'resource';
    return ({
      overview: `What this ${what} is, whether it works, and who relies on it`,
      config: c === 'llm' ? 'Where its calls go, which model, and the key it sends' : c === 'credentials' ? 'The value it holds — never shown unless you ask' : c === 'mcp' ? 'Where the server is and how to reach it' : 'How FlowR connects to it',
      test: c === 'llm' ? 'Ask it something, as you' : c === 'mcp' ? 'Its tools, and a call to one' : c === 'database' ? 'Look inside it' : c === 'credentials' ? 'Check it resolves' : 'Read it back',
      used: 'The agents, canvases and triggers that rely on it',
    } as Record<string, string>)[tab] ?? '';
  }

  const createTreeNodes = [
    { id: 'details', label: 'Details', icon: '<rect x="2" y="2" width="12" height="12" rx="2"/><path d="M5 6h6M5 9h4"/>' },
    { id: 'config', label: 'Configuration', icon: '<path d="M8 1v4M8 11v4M1 8h4M11 8h4"/><circle cx="8" cy="8" r="3"/>' },
    { id: 'review', label: 'Review & Create', icon: '<path d="M4 8l3 3 5-6"/><circle cx="8" cy="8" r="7" fill="none"/>' },
  ];

  // ── Delete ──
  let deleteTarget = $state<{ name: string; namespace: string } | null>(null);

  const detailTreeNodes = [
    { id: 'overview', label: 'Overview', icon: '<rect x="2" y="2" width="12" height="12" rx="2"/><path d="M5 6h6M5 9h4"/>' },
    { id: 'config', label: 'Configuration', icon: '<path d="M8 1v4M8 11v4M1 8h4M11 8h4"/><circle cx="8" cy="8" r="3"/>' },
    { id: 'test', label: 'Test', icon: '<polygon points="4,2 13,8 4,14"/>' },
    { id: 'used', label: 'Used by', icon: '<circle cx="4" cy="4" r="2"/><circle cx="12" cy="4" r="2"/><circle cx="8" cy="12" r="2"/><path d="M5.5 5.5L7 10.5M10.5 5.5L9 10.5"/>' },
  ];

  // Flows: editable Overview form first, then Definition (graph + YAML)
  const flowDetailTreeNodes = [
    { id: 'overview', label: 'Overview', icon: '<rect x="2" y="2" width="12" height="12" rx="2"/><path d="M5 6h6M5 9h4"/>' },
    { id: 'definition', label: 'Definition', icon: '<path d="M5 3L1 8l4 5M11 3l4 5-4 5"/>' },
    { id: 'test', label: 'Run', icon: '<polygon points="4,2 13,8 4,14"/>' },
    { id: 'logs', label: 'Runs', icon: '<rect x="2" y="1" width="12" height="14" rx="1.5"/><path d="M5 5h6M5 8h6M5 11h3"/>' },
  ];

  let detailNodes = $derived(selected?.category === 'flows' ? flowDetailTreeNodes : detailTreeNodes);

  // ── Flow definition (Overview form + graph/YAML) ──
  const DEFAULT_FLOW_YAML = `document:
  dsl: 1.0.3
  namespace: a3t
  name: new-flow
  version: 1.0.0
  metadata:
    description: ""
do:
  - step-1:
      set:
        hello: world
`;

  let flowYaml = $state('');
  let flowYamlLoading = $state(false);
  let flowDirty = $state(false);
  let flowDeploying = $state(false);
  let isNewFlow = $state(false);
  let flowMeta = $state({ name: '', namespace: getProject() || 'default', version: '1.0.0', description: '' });
  let flowMetaDirty = $state(false);

  // ── Flow execution history (Logs tab) ──
  let flowExecutions = $state<any[]>([]);
  let execLoading = $state(false);

  async function loadExecutions(r: Resource) {
    execLoading = true;
    flowExecutions = [];
    try {
      const res = await mcpCall('list_executions', { namespace: r.namespace, name: r.name, limit: 25 });
      const rows = Array.isArray(res?.data) ? res.data : (res?.data?.rows ?? res?.data?.output ?? []);
      flowExecutions = Array.isArray(rows) ? rows : [];
    } catch (e) {
      flowExecutions = [];
    } finally {
      execLoading = false;
    }
  }

  function execDuration(e: any): string {
    if (e.duration_us != null) {
      const ms = e.duration_us / 1000;
      return ms < 1000 ? `${ms.toFixed(0)}ms` : `${(ms / 1000).toFixed(2)}s`;
    }
    return '—';
  }

  async function loadFlowDefinition(r: Resource) {
    flowYaml = '';
    flowDirty = false;
    flowYamlLoading = true;
    try {
      const res = await flowsMcp.get(r.name, r.namespace);
      const text = typeof res?.data === 'string' ? res.data : (res?.data?.text ?? '');
      const match = text.match(/```yaml\n([\s\S]*?)```/);
      flowYaml = match ? match[1].trim() : text;
    } catch (e) {
      flowYaml = `# Failed to load flow: ${e}`;
    } finally {
      flowYamlLoading = false;
      syncYamlToMeta();
    }
  }

  // Parse the header fields out of the YAML into the Overview form
  function syncYamlToMeta() {
    try {
      const doc = YAML.parse(flowYaml) ?? {};
      const d = doc.document ?? doc;
      flowMeta = {
        name: d.name ?? '',
        namespace: d.namespace ?? 'a3t',
        version: (d.version ?? '1.0.0') + '',
        description: (d.metadata?.description ?? doc.description ?? '') + '',
      };
      flowMetaDirty = false;
    } catch { /* leave form as-is on parse error */ }
  }

  // Write the Overview form fields back into the YAML header (preserves do/input body)
  function syncMetaToYaml() {
    let doc: any;
    try { doc = YAML.parse(flowYaml) ?? {}; } catch { doc = {}; }
    if (!doc.document) doc.document = {};
    const d = doc.document;
    d.dsl = d.dsl ?? '1.0.3';
    d.name = flowMeta.name;
    d.namespace = flowMeta.namespace;
    d.version = flowMeta.version;
    if (!d.metadata) d.metadata = {};
    d.metadata.description = flowMeta.description;
    flowYaml = YAML.stringify(doc);
    flowMetaDirty = false;
  }

  function newFlow() {
    isNewFlow = true;
    flowMeta = { name: '', namespace: getProject() || 'default', version: '1.0.0', description: '' };
    flowMetaDirty = false;
    flowYaml = DEFAULT_FLOW_YAML;
    flowYamlLoading = false;
    flowDirty = false;
    panelTab = 'overview';
    resetTestState();
    selected = {
      name: 'new-flow', namespace: getProject() || 'default', type: 'workflow', category: 'flows',
      description: '', created_at: '', created_by: '', tags: [], healthy: true,
    };
  }

  // Switching panel tabs — keep Overview form and YAML in sync as you navigate
  function selectPanelTab(id: string) {
    if (selected?.category === 'flows') {
      if (panelTab === 'overview' && id === 'definition' && flowMetaDirty) syncMetaToYaml();
      else if (panelTab === 'definition' && id === 'overview') syncYamlToMeta();
      if (id === 'logs' && !isNewFlow) loadExecutions(selected);
    }
    panelTab = id;
  }

  async function saveFlow() {
    if (selected?.category !== 'flows' || !flowMeta.name) return;
    if (flowMetaDirty) syncMetaToYaml();
    flowDeploying = true; flowMsg = '';
    try {
      // Saved as its version; then made the active one. A version already saved
      // is refused: raise the version to save a change (no silent no-op).
      const c = await mcpCall('create_flow', { yaml: flowYaml });
      if (!c.ok) {
        const t = String(c.error ?? c.data ?? '');
        flowMsg = /exist/i.test(t) ? `Version ${flowMeta.version} is already saved: raise the version to save this change.` : `Not saved: ${t.slice(0, 200)}`;
        return;
      }
      const u = await mcpCall('update_flow', { namespace: flowMeta.namespace, name: flowMeta.name, version: flowMeta.version, enabled: true });
      flowMsg = u.ok ? `Saved v${flowMeta.version}, now the active version.` : `Saved v${flowMeta.version}, but not made active: ${String(u.error ?? '').slice(0, 160)}`;
      flowDirty = false;
      flowMetaDirty = false;
      if (isNewFlow) {
        isNewFlow = false;
        allResources = await fetchAllResources();
        const created = allResources.find(x => x.category === 'flows' && x.name === flowMeta.name && x.namespace === flowMeta.namespace);
        if (created) selected = created;
      }
    } catch (e) { error = String(e); }
    finally { flowDeploying = false; }
  }

  // ── Fetch ──
  async function fetchAllResources() {
    // Platform namespaces too: users' resources and flows saved there before
    // projects are `default`'s (itemProject); only the selected project's are kept.
    const namespaces = namespacesFor('picker');
    const results: Resource[] = [];

    // Fetch resources
    for (const ns of namespaces) {
      const res = await resourcesMcp.list(ns);
      const data = res?.data;
      if (data?.resources) {
        for (const r of data.resources) {
          // A project's own record (Projects app) is not a resource to manage here.
          if (r.type === 'project') continue;
          const project = itemProject('resource', r.namespace ?? ns, r);
          if (!inContext(project, 'shared')) continue;
          results.push({
            name: r.name, namespace: r.namespace ?? ns, project,
            type: r.type ?? 'unknown', category: categorize(r.type ?? ''),
            description: r.description ?? '', created_at: r.created_at ?? '',
            created_by: r.created_by ?? '', tags: r.tags ?? [], healthy: true,
            config: r.config ?? {},
          });
        }
      }
    }

    // Fetch workflows and show them under "flows" category
    for (const ns of namespacesFor('picker')) {
      const res = await flowsMcp.list(ns);
      const data = res?.data;
      if (data?.workflows) {
        for (const wf of data.workflows) {
          const project = itemProject('flow', wf.namespace ?? ns, wf);
          if (!inContext(project, 'owned')) continue;
          results.push({
            name: wf.name, namespace: wf.namespace ?? ns, project,
            type: 'workflow', category: 'flows',
            description: `v${wf.version} · ${wf.step_count} steps`,
            created_at: '', created_by: '',
            tags: [], healthy: wf.enabled !== false,
          });
        }
      }
    }

    return results;
  }

  onMount(async () => {
    try {
      allResources = await fetchAllResources();
    } catch (e) { error = String(e); }
    finally { loading = false; }
    // The shell's "New resource": the form asks for the kind first.
    if (getActivePanel()?.props?.create) openCreate('');
    // A resource picked in the shell's Items: `<category>:<namespace>/<name>`.
    const sel = getActivePanel()?.props?.selectedId as string | undefined;
    if (sel) openById(sel);
  });

  /// A resource picked in the shell's Items: `<category>:<namespace>/<name>`.
  function openById(sel: string) {
    const m = sel.match(/^([^:]+):([^/]+)\/(.+)$/);
    if (!m) return;
    const r = allResources.find((x) => x.category === m[1] && x.namespace === m[2] && x.name === m[3]);
    if (r) selectResource(r);
  }

  // Another item picked in Items: the page stays mounted and opens it in place,
  // from the list it already has — no reload, no flash of this page.
  let shownSel: string | undefined = untrack(() => getActivePanel()?.props?.selectedId);
  $effect(() => {
    const id = getActivePanel()?.props?.selectedId as string | undefined;
    untrack(() => {
      if (id === shownSel) return;
      shownSel = id;
      if (id) openById(id);
    });
  });

  // A different project (or "show platform") is a different list.
  let loadedRev = contextRev_();
  $effect(() => {
    const rev = contextRev_();
    if (rev === loadedRev) return;
    loadedRev = rev;
    fetchAllResources().then((r) => { allResources = r; }).catch((e) => { error = String(e); });
  });

  // ── Actions ──
  function selectResource(r: Resource) {
    selected = r;
    isNewFlow = false;
    panelTab = 'overview';
    resetTestState();
    cfgEditing = false;
    // A test still running on the last one is not this one's.
    lastTest = null; showJson = false; flowMsg = ''; testing = false; mcpListed = false; delName = '';
    if (r.category === 'flows') loadFlowDefinition(r);
    else { loadConfig(r); void loadUses(); }
  }

  function pickKind(category: string) {
    createForCategory = category;
    newResource.type = TYPES_BY_CATEGORY[category]?.[0]?.value ?? '';
    newResource.config = '{}';
  }

  function closePanel() {
    selected = null;
    isNewFlow = false;
  }

  /// Open the create form for a kind of resource; with none (the "All" view)
  /// the form asks for the kind first.
  function openCreate(category: string) {
    if (category === 'flows') { newFlow(); return; }
    // One panel at a time: a resource open in Details gives way to the new one.
    selected = null;
    createForCategory = category;
    newResource = { name: '', namespace: getProject() || '', type: TYPES_BY_CATEGORY[category]?.[0]?.value ?? '', config: '{}' };
    createTab = 'details';
    showCreate = true;
  }

  async function createResource() {
    if (!newResource.name || !newResource.type) return;
    creating = true;
    try {
      let config: any;
      try { config = JSON.parse(newResource.config); } catch { config = {}; }
      await resourcesMcp.create(newResource.name, newResource.type, config, newResource.namespace);
      showCreate = false;
      allResources = await fetchAllResources();
    } catch (e) { error = String(e); }
    finally { creating = false; }
  }

  function resetTestState() {
    testResult = '';
    llmInput = '';
    llmMessages = [];
    dbQuery = '';
    dbOperation = 'list_tables';
    adbQuery = ADB_SAMPLE_QUERY;
    mcpTools = [];
    mcpSelectedTool = '';
    mcpToolArgs = '{}';
    mcpToolResult = '';
    flowInput = '{}';
    secretConfig = null;
    secretRevealed = false;
  }

  // ── LLM: chat-style test ──
  async function testLlm() {
    if (!selected || !llmInput.trim()) return;
    const msg = llmInput.trim();
    llmMessages = [...llmMessages, { role: 'user', content: msg }];
    llmInput = '';
    testing = true;
    try {
      const res = await execFlow('flow_a3t_resource_test', 'a3t', {
        input: { test_type: 'llm', resource: `${selected.namespace}/${selected.name}`, message: msg }
      });
      const content = res.data?.content ?? res.data?.text ?? (typeof res.data === 'string' ? res.data : JSON.stringify(res.data));
      llmMessages = [...llmMessages, { role: 'assistant', content }];
      lastTest = res.ok && !/^error/i.test(String(content)) ? { ok: true, msg: 'It answered.' } : { ok: false, msg: String(res.error ?? content).slice(0, 200) };
    } catch (e: any) {
      llmMessages = [...llmMessages, { role: 'assistant', content: `Error: ${e.message ?? e}` }];
      lastTest = { ok: false, msg: String(e.message ?? e) };
    } finally {
      testing = false;
    }
  }

  // ── ADB: one AQL statement via the flow_a3t_resource_test flow's call:adb branch ──
  async function testAdb() {
    if (!selected || !adbQuery.trim()) return;
    testing = true;
    testResult = '';
    try {
      const input = { test_type: 'adb', resource: `${selected.namespace}/${selected.name}`, query: adbQuery.trim() };
      const res = await execFlow('flow_a3t_resource_test', 'a3t', { input });
      testResult = !res.ok ? `Error: ${res.error ?? 'failed'}` : typeof res.data === 'string' ? res.data : JSON.stringify(res.data, null, 2);
      lastTest = res.ok ? { ok: true, msg: 'It answered AQL.' } : { ok: false, msg: String(res.error ?? 'failed') };
    } catch (e: any) {
      testResult = `Error: ${e.message ?? e}`;
    } finally {
      testing = false;
    }
  }

  // ── DB: connection + query test ──
  async function testDb() {
    if (!selected) return;
    testing = true;
    testResult = '';
    try {
      const input: any = { test_type: 'db', resource: `${selected.namespace}/${selected.name}`, operation: dbOperation };
      if (dbOperation === 'query' && dbQuery) input.filter = dbQuery;
      if (dbOperation === 'query' && dbQuery) input.table = dbQuery.split(' ')[0];
      const res = await execFlow('flow_a3t_resource_test', 'a3t', { input });
      testResult = !res.ok ? `Error: ${res.error ?? 'failed'}` : typeof res.data === 'string' ? res.data : JSON.stringify(res.data, null, 2);
      lastTest = res.ok ? { ok: true, msg: 'It opened.' } : { ok: false, msg: String(res.error ?? 'failed') };
    } catch (e: any) {
      testResult = `Error: ${e.message ?? e}`;
    } finally {
      testing = false;
    }
  }

  // ── MCP: list tools ──
  async function testMcpListTools() {
    if (!selected) return;
    testing = true;
    mcpTools = [];
    testResult = '';
    try {
      const res = await execFlow('flow_a3t_resource_test', 'a3t', {
        input: { test_type: 'mcp', resource: `${selected.namespace}/${selected.name}` }
      });
      const tools = res.data?.tools ?? res.data;
      if (res.ok && Array.isArray(tools)) {
        mcpTools = tools.map((t: any) => ({ name: t.name ?? '', description: t.description ?? '' }));
        mcpListed = true;
        lastTest = { ok: true, msg: `It lists ${mcpTools.length} tool${mcpTools.length === 1 ? '' : 's'}.` };
      } else {
        testResult = !res.ok ? `Error: ${res.error ?? 'failed'}` : typeof tools === 'string' ? tools : JSON.stringify(tools, null, 2);
        lastTest = { ok: false, msg: String(res.error ?? testResult).slice(0, 200) };
      }
    } catch (e: any) {
      testResult = `Error: ${e.message ?? e}`;
    } finally {
      testing = false;
    }
  }

  // ── MCP: call selected tool ──
  async function testMcpCallTool() {
    if (!selected || !mcpSelectedTool) return;
    testing = true;
    mcpToolResult = '';
    try {
      let args: any = {};
      try { args = JSON.parse(mcpToolArgs); } catch {}
      const res = await execFlow('flow_a3t_resource_test', 'a3t', {
        input: { test_type: 'mcp_call', resource: `${selected.namespace}/${selected.name}`, tool: mcpSelectedTool, arguments: args }
      });
      mcpToolResult = typeof res.data === 'string' ? res.data : JSON.stringify(res.data, null, 2);
    } catch (e: any) {
      mcpToolResult = `Error: ${e.message ?? e}`;
    } finally {
      testing = false;
    }
  }

  // ── Flow: execute with input ──
  async function testFlow() {
    if (!selected) return;
    testing = true;
    testResult = '';
    try {
      let input: any = {};
      try { input = JSON.parse(flowInput); } catch {}
      const res = await execFlow(selected.name, selected.namespace, input);
      testResult = typeof res.data === 'string' ? res.data : JSON.stringify(res.data, null, 2);
    } catch (e: any) {
      testResult = `Error: ${e.message ?? e}`;
    } finally {
      testing = false;
    }
  }

  // ── Secret: verify and show masked ──
  async function testSecret() {
    if (!selected) return;
    testing = true;
    testResult = '';
    secretConfig = null;
    try {
      const res = await mcpCall('get_resource', { name: selected.name, namespace: selected.namespace });
      const config = res?.data?.config ?? {};
      secretConfig = config;
      testResult = '';
      const v = config.value ?? config.token ?? config.api_key ?? config.password;
      lastTest = v ? { ok: true, msg: 'It holds a value.' } : { ok: false, msg: 'It is empty.' };
    } catch (e: any) {
      testResult = `Error: ${e.message ?? e}`;
    } finally {
      testing = false;
    }
  }

  // ── Generic fallback test ──
  async function testGeneric() {
    if (!selected) return;
    testing = true;
    testResult = '';
    try {
      const res = await mcpCall('get_resource', { name: selected.name, namespace: selected.namespace });
      testResult = typeof res === 'string' ? res : JSON.stringify(res?.data ?? res, null, 2);
    } catch (e: any) {
      testResult = `Error: ${e.message ?? e}`;
    } finally {
      testing = false;
    }
  }

  function maskValue(val: string): string {
    if (!val || val.length < 8) return '••••••••';
    return val.slice(0, 6) + '••••••••' + val.slice(-4);
  }

  async function deleteResource() {
    if (!deleteTarget) return;
    try {
      let res = await resourcesMcp.remove(deleteTarget.namespace, deleteTarget.name);
      // A runtime is not refused: deleting it terminates it and reschedules what
      // ran there. The server answers 409 with that plan first — show it.
      if (res.status === 409 && res.detail?.impact) {
        const im = res.detail.impact;
        const lines = (im.plan ?? []).map((p: any) =>
          `• ${p.target} (${p.mode}) → ${p.action === 'move' ? `moves to ${p.to}` : `fresh ${p.to} recreated`}${p.warning ? '  ⚠ ' + p.warning : ''}`);
        const pinned = (im.pinnedCanvases ?? []).length ? `\nCanvases pinned to it: ${im.pinnedCanvases.join(', ')}` : '';
        const ok = confirm(`Delete runtime ${deleteTarget.name}?\n\nRuns on it are cancelled and what is deployed there is rescheduled:\n${lines.join('\n')}${pinned}`);
        if (!ok) { deleteTarget = null; return; }
        res = await resourcesMcp.remove(deleteTarget.namespace, deleteTarget.name, deleteTarget.name);
      }
      if (!res.ok) { messages.error(res.error ?? 'Delete failed'); deleteTarget = null; return; }
      if (selected?.name === deleteTarget.name) selected = null;
      deleteTarget = null;
      allResources = await fetchAllResources();
    } catch (e) { error = String(e); deleteTarget = null; }
  }

  // Types available for create form dropdown
  const TYPES_BY_CATEGORY: Record<string, { value: string; label: string }[]> = {
    database:    [{ value: 'postgres', label: 'PostgreSQL' }, { value: 'mysql', label: 'MySQL' }, { value: 'lancedb', label: 'LanceDB' }, { value: 'redis', label: 'Redis / Dragonfly' }, { value: 'qdrant', label: 'Qdrant' }, { value: 'adb', label: 'ADB (Agent Memory)' }],
    llm:         [{ value: 'openai', label: 'OpenAI' }, { value: 'anthropic', label: 'Anthropic' }, { value: 'groq', label: 'Groq' }, { value: 'ollama', label: 'Ollama' }],
    mcp:         [{ value: 'mcp', label: 'MCP Server' }],
    functions:   [{ value: 'function', label: 'Function' }],
    scripts:     [{ value: 'script', label: 'Script' }],
    variables:   [{ value: 'variable', label: 'Variable' }],
    credentials: [{ value: 'credential', label: 'Credential' }, { value: 'secret', label: 'Secret' }],
  };
</script>

<!-- Detail: the agents' and triggers' frame — header and tabs (PropertiesTree),
     each tab numbered cards (.agt) at the details width. -->
<SlidePanel
  open={!!selected}
  onclose={closePanel}
  onsave={selected?.category === 'flows' ? saveFlow : undefined}
  onopenineditor={selected && !isNewFlow ? () => openInStudio(selected!.category === 'flows' ? 'flow' : selected!.type === 'canvas' ? 'canvas' : 'resource', selected!.namespace, selected!.name) : undefined}
>
  {#if selected}
    {@const what = selected.category === 'flows' ? 'Flow' : selected.category === 'llm' ? 'Model' : selected.category === 'mcp' ? 'Tool server' : selected.category === 'database' ? (isAdb(selected) ? 'ADB' : 'Database') : selected.category === 'credentials' ? 'Secret' : catMeta(selected.category).label.replace(/s$/, '')}
    <div class="panel-layout">
      <PropertiesTree
        icon={catMeta(selected.category).icon}
        color={CAT_COLORS[selected.category] ?? '#999'}
        name={selected.category === 'flows' && isNewFlow ? (flowMeta.name || 'New flow') : selected.name}
        meta="{selected.type} · {selected.namespace}"
        nodes={detailNodes}
        steps={detailSteps}
        activeNode={panelTab}
        onselect={selectPanelTab}
      />
      <div class="panel-col">
      <div class="panel-content"><div class="pf">
        <WizardHead step={detailSteps.find((x) => x.id === panelTab)} />
        <fieldset class="rs-fs" disabled={!$canUpdate && selected.category !== 'flows'}><section class="agt">

        {#if panelTab === 'overview' && selected.category === 'flows'}
          <div class="card">
            <div class="card-head"><h3>1 · {isNewFlow ? 'New flow' : 'Flow'}</h3><span>its name, project, version and what it does</span></div>
            <div class="fields">
              <label class="fld">Name<input type="text" bind:value={flowMeta.name} oninput={() => flowMetaDirty = true} placeholder="flow_default_refund" /></label>
              <label class="fld">Project<input type="text" bind:value={flowMeta.namespace} oninput={() => flowMetaDirty = true} /></label>
              <label class="fld">Version <small>a change is saved as a new version</small><input type="text" bind:value={flowMeta.version} oninput={() => flowMetaDirty = true} /></label>
              <label class="fld full">Description<textarea rows="3" bind:value={flowMeta.description} oninput={() => flowMetaDirty = true} placeholder="What it does"></textarea></label>
            </div>
            <div class="acts">
              <button class="btn2 primary" disabled={flowDeploying || !flowMeta.name} onclick={saveFlow}>{flowDeploying ? (isNewFlow ? 'Creating…' : 'Saving…') : isNewFlow ? 'Create flow' : 'Save'}</button>
              {#if isNewFlow}<button class="btn2 quiet" onclick={closePanel}>Cancel</button>{/if}
              {#if flowMsg}<span class={/^Saved/.test(flowMsg) ? 'ok' : 'err'}>{flowMsg}</span>{/if}
            </div>
          </div>
          {#if !isNewFlow}
            <div class="card">
              <div class="card-head"><h3>2 · Status</h3><span>its steps, and how to change and run it</span></div>
              <ul class="rows">
                <li><span class="lbl">Version</span><span class="val">{selected.description || `v${flowMeta.version}`}</span><button class="btn2" onclick={() => selectPanelTab('definition')}>Steps</button></li>
                <li><span class="lbl">State</span><span class="val"><span class="dot" class:ok={selected.healthy}></span> {selected.healthy ? 'enabled' : 'disabled'}</span><span></span></li>
                <li><span class="lbl">Runs</span><span class="val">run it with an input, or see its recent runs</span><button class="btn2" onclick={() => selectPanelTab('test')}>Run</button></li>
              </ul>
            </div>
          {/if}

        {:else if panelTab === 'definition'}
          {#if flowYamlLoading}<p class="dim">Reading the flow…</p>
          {:else if flowYaml}
            <div class="def-pane"><FlowEditorPane part="main" initialYaml={flowYaml} onchange={(y) => { flowYaml = y; flowDirty = true; }} /></div>
            <div class="acts"><button class="btn2 primary" disabled={flowDeploying || !flowDirty} onclick={saveFlow}>{flowDeploying ? 'Saving…' : 'Save'}</button>
              <span class="dim">{flowDirty ? 'Changed — saved as v' + flowMeta.version + '.' : 'No changes.'}</span>
              {#if flowMsg}<span class={/^Saved/.test(flowMsg) ? 'ok' : 'err'}>{flowMsg}</span>{/if}</div>
          {:else}<div class="card"><p class="empty">No definition for this flow.</p></div>{/if}

        {:else if panelTab === 'test' && selected.category === 'flows'}
          <div class="card">
            <div class="card-head"><h3>1 · Run it</h3><span>as you, with this input</span></div>
            <label class="fld">Input <small>JSON</small><textarea rows="6" bind:value={flowInput} spellcheck="false"></textarea></label>
            <div class="acts"><button class="btn2 primary" disabled={testing} onclick={testFlow}>{testing ? 'Running…' : 'Run'}</button>
              <button class="link" onclick={() => selectPanelTab('logs')}>Its runs →</button></div>
            {#if testResult}<pre class="code out">{testResult}</pre>{/if}
          </div>

        {:else if panelTab === 'logs' && selected.category === 'flows'}
          <div class="card">
            <div class="card-head"><h3>1 · Recent runs</h3><span>up to 25</span><span class="grow"></span><button class="btn2 quiet rm" disabled={execLoading} onclick={() => loadExecutions(selected!)}>Refresh</button></div>
            {#if execLoading}<p class="dim">Reading its runs…</p>
            {:else if flowExecutions.length === 0}<p class="empty">No runs recorded. Run it from Run.</p>
            {:else}
              <ul class="tl">
                {#each flowExecutions as e (e.instance_id)}
                  <li class="ev" class:err={/fail/i.test(String(e.status))}>
                    <span class="ic"><span class="dot" class:ok={/complet/i.test(String(e.status))} class:err={/fail/i.test(String(e.status))} class:warn={/run/i.test(String(e.status))}></span></span>
                    <details>
                      <summary class="t">{e.status} · {execDuration(e)} <span class="m mono">{String(e.instance_id ?? '').slice(0, 8)}</span></summary>
                      {#if e.error}<div class="m err">{e.error}</div>{/if}
                      <div class="m">Input</div><pre class="code out">{JSON.stringify(e.input ?? {}, null, 2)}</pre>
                      {#if e.output != null}<div class="m">Output</div><pre class="code out">{JSON.stringify(e.output, null, 2)}</pre>{/if}
                    </details>
                    <time>{e.started_at ? new Date(e.started_at).toLocaleString() : '—'}</time>
                  </li>
                {/each}
              </ul>
            {/if}
          </div>

        {:else if panelTab === 'overview'}
          {@const cfg = selected.config ?? {}}
          <div class="card">
            <div class="card-head"><h3>1 · {what}</h3><span>{catMeta(selected.category).label}</span></div>
            <ul class="rows">
              <li><span class="lbl">Name</span><span class="val">{selected.name}</span><span></span></li>
              <li><span class="lbl">Project</span><span class="val">{selected.namespace}</span><span></span></li>
              <li><span class="lbl">Type</span><span class="val mono">{selected.type}</span><span></span></li>
              {#if selected.created_at}<li><span class="lbl">Created</span><span class="val">{new Date(selected.created_at).toLocaleString()}{selected.created_by ? ` · ${selected.created_by}` : ''}</span><span></span></li>{/if}
              {#if selected.description}<li><span class="lbl">About</span><span class="val">{selected.description}</span><span></span></li>{/if}
              {#if selected.tags.length}<li><span class="lbl">Tags</span><span class="val">{selected.tags.join(', ')}</span><span></span></li>{/if}
            </ul>
            {#if selected.type === 'canvas' || selected.type === 'runtime'}
              <div class="note">A {selected.type} is managed in its own app.
                <button class="link" onclick={() => navigateTo(selected!.type === 'canvas' ? 'canvas' : 'runtime', selected!.type === 'canvas' ? 'Canvas' : 'Runtime', selected!.type === 'canvas' ? { selectedId: selected!.name } : {})}>Open in {selected.type === 'canvas' ? 'Canvas' : 'Runtime'} →</button></div>
            {/if}
          </div>
          <div class="card">
            <div class="card-head"><h3>2 · Status</h3><span>whether it works, and who relies on it</span></div>
            <ul class="rows">
              {#if selected.category === 'llm'}
                <li><span class="lbl">Model</span><span class="val mono">{cfg.model ?? '—'}</span><button class="btn2" onclick={() => selectPanelTab('config')}>Change</button><span class="sub">{cfg.api_url ?? cfg.base_url ?? 'no endpoint set'}</span></li>
                <li><span class="lbl">Key</span>
                  <span class="val">{#if cfg.api_key_ref}<span class="dot" class:ok={secretRefs.some((s) => s.ref === cfg.api_key_ref)} class:err={!secretRefs.some((s) => s.ref === cfg.api_key_ref)}></span> secret <span class="mono">{cfg.api_key_ref}</span>{secretRefs.some((s) => s.ref === cfg.api_key_ref) ? '' : ' (missing)'}{:else if cfg.api_key}<span class="dot warn"></span> inline, in its configuration{:else}<span class="dot err"></span> none{/if}</span>
                  <button class="btn2" onclick={() => selectPanelTab('config')}>Change</button>
                  {#if cfg.api_key && !cfg.api_key_ref}<span class="sub">An inline key is stored with the resource: keep it in a Secret instead.</span>{/if}</li>
              {:else if selected.category === 'mcp'}
                <li><span class="lbl">Server</span><span class="val mono">{cfg.url ?? cfg.external_url ?? '—'}</span><button class="btn2" onclick={() => selectPanelTab('config')}>Change</button><span class="sub">{cfg.transport ?? 'http'}{cfg.auth || cfg.oauth2 ? ' · authenticated' : ''}</span></li>
              {:else if selected.category === 'database'}
                <li><span class="lbl">Where</span><span class="val mono">{cfg.url ?? cfg.db_path ?? cfg.host ?? '—'}</span><button class="btn2" onclick={() => selectPanelTab('config')}>Change</button>{#if cfg.flight_url || cfg.backend}<span class="sub">{[cfg.backend, cfg.flight_url].filter(Boolean).join(' · ')}</span>{/if}</li>
              {:else if selected.category === 'credentials'}
                <li><span class="lbl">Value</span><span class="val">{cfg.value || cfg.token || cfg.api_key ? 'set (hidden)' : 'empty'}</span><button class="btn2" onclick={() => selectPanelTab('config')}>Change</button></li>
              {/if}
              <li><span class="lbl">Works</span>
                <span class="val">{#if lastTest}<span class="dot" class:ok={lastTest.ok} class:err={!lastTest.ok}></span> {lastTest.msg}{:else}<span class="dot"></span> not checked yet{/if}</span>
                <button class="btn2" onclick={() => selectPanelTab('test')}>Test</button></li>
              <li><span class="lbl">Used by</span>
                <span class="val">{#if uses === null}…{:else if uses.length}{[['agent', 'agent', 'agents'], ['flow', 'flow', 'flows'], ['resource', 'resource', 'resources'], ['canvas', 'canvas', 'canvases'], ['trigger', 'trigger', 'triggers']].map(([k, one, many]) => { const n = uses!.filter((u) => u.kind === k).length; return n ? `${n} ${n === 1 ? one : many}` : ''; }).filter(Boolean).join(', ')}{:else}nothing{/if}</span>
                <button class="btn2" onclick={() => selectPanelTab('used')}>See</button></li>
            </ul>
          </div>
          {#if $canUpdate}
            <div class="card">
              <div class="card-head"><h3>3 · Delete</h3><span>remove it from the registry</span></div>
              {#if uses?.length}<div class="note warn">{uses.length} thing{uses.length === 1 ? '' : 's'} rely on it: {uses.map((u) => u.name).join(', ')}. They stop working when it is gone.</div>{/if}
              <!-- Confirmed here, in the card: a dialog would open behind the Details pane. -->
              <div class="row"><input class="grow ask" type="text" bind:value={delName} placeholder="Type {selected.name} to delete it" autocomplete="off" />
                <button class="btn2 danger" disabled={delName !== selected.name || deleting} onclick={deleteSelected}>{deleting ? 'Deleting…' : `Delete ${selected.name}`}</button></div>
            </div>
          {/if}

        {:else if panelTab === 'config' && selected.category === 'llm'}
          <div class="card">
            <div class="card-head"><h3>1 · Endpoint</h3><span>where its calls go, and which model</span></div>
            <div class="fields">
              <label class="fld full">API URL <small>the base URL requests are sent to</small><input type="text" class="mono" bind:value={llmCfg.api_url} placeholder="https://api.provider.com/v1" /></label>
              <label class="fld">Model<input type="text" class="mono" bind:value={llmCfg.model} placeholder="model-id" /></label>
              <label class="fld">Provider <small>its API style</small><input type="text" bind:value={llmCfg.provider} placeholder="openai" /></label>
            </div>
          </div>
          <div class="card">
            <div class="card-head"><h3>2 · Key</h3><span>what it sends to the provider</span></div>
            <div class="seg src" role="tablist">
              <button role="tab" class:on={llmCfg.api_key_ref != null} onclick={() => setKeyMode('secret')}>From a secret</button>
              <button role="tab" class:on={llmCfg.api_key_ref == null} onclick={() => setKeyMode('inline')}>Inline</button>
            </div>
            {#if llmCfg.api_key_ref != null}
              <label class="fld">Secret <small>a Secret resource whose value is the key</small>
                <select bind:value={llmCfg.api_key_ref}>
                  <option value="">— pick a secret —</option>
                  {#each secretRefs as s (s.ref)}<option value={s.ref}>{s.ref}</option>{/each}
                  {#if llmCfg.api_key_ref && !secretRefs.some((s) => s.ref === llmCfg.api_key_ref)}<option value={llmCfg.api_key_ref}>{llmCfg.api_key_ref} (missing)</option>{/if}
                </select></label>
            {:else}
              <label class="fld">Key<input type="password" bind:value={llmCfg.api_key} placeholder="sk-…" autocomplete="off" /></label>
              <div class="note warn">An inline key is stored with this resource and shown to anyone who can read it. Keep it in a Secret and pick that instead.</div>
            {/if}
          </div>
          <div class="card">
            <div class="card-head"><h3>3 · Defaults</h3><span>used when a call does not say</span></div>
            <div class="fields">
              <label class="fld">Max tokens<input type="number" bind:value={llmCfg.max_tokens} /></label>
              <label class="fld">Temperature<input type="number" step="0.1" bind:value={llmCfg.temperature} /></label>
            </div>
          </div>
          {#if $canUpdate}
            <div class="acts"><button class="btn2 primary" disabled={cfgSaving || !llmDirty} onclick={saveConfig}>{cfgSaving ? 'Saving…' : 'Save'}</button>
              <button class="btn2 quiet" disabled={cfgSaving || !llmDirty} onclick={() => (llmCfg = { ...(selected?.config ?? {}) })}>Discard</button>
              {#if cfgMsg}<span class={cfgMsg === 'Saved' ? 'ok' : 'err'}>{cfgMsg}</span>{:else}<span class="dim">{llmDirty ? 'Not saved yet.' : ''}</span>{/if}
              <span class="grow"></span><button class="link" onclick={() => (showJson = !showJson)}>{showJson ? 'Hide' : 'Show'} as JSON</button></div>
            {#if showJson}<pre class="code">{JSON.stringify({ ...llmCfg, ...(llmCfg.api_key ? { api_key: '••••••' } : {}) }, null, 2)}</pre>{/if}
          {/if}

        {:else if panelTab === 'config'}
          <div class="card">
            <div class="card-head"><h3>1 · {selected.category === 'credentials' ? 'Value' : 'Settings'}</h3><span>{selected.category === 'credentials' ? 'hidden unless you show it' : `its ${selected.type} configuration`}</span>
              {#if cfgDraft.some((f) => SECRET_KEY.test(f.key))}<span class="grow"></span><button class="btn2 quiet rm" onclick={() => (revealed = !revealed)}>{revealed ? 'Hide' : 'Show'} values</button>{/if}</div>
            {#if selected.type === 'adb'}<p class="dim"><span class="mono">url</span> is its HTTP address (AQL, MCP at <span class="mono">/mcp/sse</span>); <span class="mono">call:adb</span> uses Arrow Flight on the same host, port 3001 — set <span class="mono">flight_url</span> only if it is elsewhere.</p>{/if}
            {#if cfgDraft.length}
              <div class="kvf">
                {#each cfgDraft as f, i (i)}
                  <label class="fld">{f.key ? fieldLabel(f.key) : 'New setting'}{#if f.key && fieldLabel(f.key).toLowerCase() !== f.key.toLowerCase()} <small class="mono">{f.key}</small>{/if}{#if f.kind === 'json'} <small>JSON</small>{/if}
                    {#if !f.key}<input class="mono" type="text" bind:value={f.key} placeholder="key" />{/if}
                    {#if f.kind === 'boolean'}<select bind:value={f.value}><option value="true">true</option><option value="false">false</option></select>
                    {:else if f.kind === 'json'}<textarea rows="4" bind:value={f.value} spellcheck="false"></textarea>
                    {:else}<input class="mono" type={SECRET_KEY.test(f.key) && !revealed ? 'password' : 'text'} bind:value={f.value} autocomplete="off" />{/if}
                  </label>
                  <button class="btn2 quiet" title="Remove {f.key}" onclick={() => (cfgDraft = cfgDraft.filter((_, j) => j !== i))}>Remove</button>
                {/each}
              </div>
            {:else}<p class="empty">No settings.</p>{/if}
            {#if $canUpdate}<div class="acts"><button class="btn2" onclick={() => (cfgDraft = [...cfgDraft, { key: '', value: '', kind: 'string' }])}>+ Add a setting</button></div>{/if}
          </div>
          {#if $canUpdate}
            <div class="acts"><button class="btn2 primary" disabled={cfgSaving || !cfgDirty} onclick={saveEdit}>{cfgSaving ? 'Saving…' : 'Save'}</button>
              <button class="btn2 quiet" disabled={cfgSaving || !cfgDirty} onclick={() => (cfgDraft = fieldsOf(selected?.config ?? {}))}>Discard</button>
              {#if cfgMsg}<span class={cfgMsg === 'Saved' ? 'ok' : 'err'}>{cfgMsg}</span>{:else}<span class="dim">{cfgDirty ? 'Not saved yet.' : ''}</span>{/if}</div>
          {/if}

        {:else if panelTab === 'test'}
          {#if selected.category === 'llm'}
            <div class="card">
              <div class="card-head"><h3>1 · Ask it</h3><span>as you, through flow_a3t_resource_test</span></div>
              {#if llmMessages.length}
                <ol class="msgs">{#each llmMessages as m, i (i)}<li class={m.role}><b>{m.role === 'user' ? 'You' : 'It'}</b><span>{m.content}</span></li>{/each}
                  {#if testing}<li class="assistant"><b>It</b><span class="dim">…</span></li>{/if}</ol>
              {/if}
              <form class="row" onsubmit={(e) => { e.preventDefault(); if (!testing) testLlm(); }}>
                <input class="grow ask" type="text" bind:value={llmInput} placeholder="Say something to it" disabled={testing} />
                <button class="btn2 primary" type="submit" disabled={testing || !llmInput.trim()}>{testing ? 'Answering…' : 'Send'}</button>
              </form>
            </div>
          {:else if isAdb(selected)}
            <div class="card">
              <div class="card-head"><h3>1 · Run AQL</h3><span>one statement against this ADB</span></div>
              <label class="fld">AQL <small>Ctrl+Enter runs it</small><textarea rows="3" bind:value={adbQuery} spellcheck="false"
                onkeydown={(e) => { if ((e.metaKey || e.ctrlKey) && e.key === 'Enter' && !testing) testAdb(); }}></textarea></label>
              <div class="acts"><button class="btn2 primary" disabled={testing || !adbQuery.trim()} onclick={testAdb}>{testing ? 'Running…' : 'Run'}</button></div>
              {#if testResult}<pre class="code out">{testResult}</pre>{/if}
            </div>
          {:else if selected.category === 'database'}
            <div class="card">
              <div class="card-head"><h3>1 · Look inside</h3><span>its tables, or one table's rows</span></div>
              <div class="seg src" role="tablist">
                <button role="tab" class:on={dbOperation === 'list_tables'} onclick={() => (dbOperation = 'list_tables')}>Tables</button>
                <button role="tab" class:on={dbOperation === 'query'} onclick={() => (dbOperation = 'query')}>Rows of a table</button>
              </div>
              {#if dbOperation === 'query'}<label class="fld">Table<input type="text" class="mono" bind:value={dbQuery} placeholder="table_name" /></label>{/if}
              <div class="acts"><button class="btn2 primary" disabled={testing} onclick={testDb}>{testing ? 'Running…' : 'Run'}</button></div>
              {#if testResult}<pre class="code out">{testResult}</pre>{/if}
            </div>
          {:else if selected.category === 'mcp'}
            <div class="card">
              <div class="card-head"><h3>1 · Its tools</h3><span>{mcpListed ? `${mcpTools.length} listed` : 'ask the server what it offers'}</span></div>
              <div class="acts"><button class="btn2 primary" disabled={testing} onclick={testMcpListTools}>{testing && !mcpTools.length ? 'Asking…' : mcpTools.length ? 'Ask again' : 'List its tools'}</button></div>
              {#if mcpListed && !mcpTools.length}<div class="note warn">It answered, but lists no tools. If it should have some, check its URL in Configuration{selected.config?.url ? ` (now ${selected.config.url})` : ''}.</div>{/if}
              {#if mcpTools.length}
                <ul class="rows wide">{#each mcpTools as t (t.name)}<li><span class="lbl mono">{t.name}</span><span class="val">{t.description}</span>
                  <button class="btn2" class:primary={mcpSelectedTool === t.name} onclick={() => { mcpSelectedTool = t.name; mcpToolArgs = '{}'; mcpToolResult = ''; }}>{mcpSelectedTool === t.name ? 'Picked' : 'Try'}</button></li>{/each}</ul>
              {/if}
              {#if testResult}<pre class="code out">{testResult}</pre>{/if}
            </div>
            {#if mcpSelectedTool}
              <div class="card">
                <div class="card-head"><h3>2 · Call {mcpSelectedTool}</h3><span>as you</span></div>
                <label class="fld">Arguments <small>JSON</small><textarea rows="4" bind:value={mcpToolArgs} spellcheck="false"></textarea></label>
                <div class="acts"><button class="btn2 primary" disabled={testing} onclick={testMcpCallTool}>{testing ? 'Calling…' : 'Call'}</button></div>
                {#if mcpToolResult}<pre class="code out">{mcpToolResult}</pre>{/if}
              </div>
            {/if}
          {:else if selected.category === 'credentials'}
            <div class="card">
              <div class="card-head"><h3>1 · Check it</h3><span>that it holds a value, masked</span></div>
              <div class="acts"><button class="btn2 primary" disabled={testing} onclick={testSecret}>{testing ? 'Checking…' : 'Check'}</button>
                {#if secretConfig}<button class="btn2 quiet" onclick={() => (secretRevealed = !secretRevealed)}>{secretRevealed ? 'Hide' : 'Show'} values</button>{/if}</div>
              {#if secretConfig}
                <ul class="rows">{#each Object.entries(secretConfig) as [k, v] (k)}<li><span class="lbl">{k}</span><span class="val mono">{SECRET_KEY.test(k) && !secretRevealed ? maskValue(String(v)) : String(v)}</span><span></span></li>{/each}</ul>
              {/if}
              {#if testResult}<pre class="code out">{testResult}</pre>{/if}
            </div>
          {:else}
            <div class="card">
              <div class="card-head"><h3>1 · Read it back</h3><span>what the registry holds</span></div>
              <div class="acts"><button class="btn2 primary" disabled={testing} onclick={testGeneric}>{testing ? 'Reading…' : 'Read'}</button></div>
              {#if testResult}<pre class="code out">{testResult}</pre>{/if}
            </div>
          {/if}

        {:else if panelTab === 'used'}
          {#each [['agent', 'Agents', 'their model, tool servers or memory'], ['flow', 'Flows', 'whose steps name it'], ['resource', 'Resources', 'whose settings name it (a secret, a model)'], ['canvas', 'Canvases', 'the deployments that depend on it'], ['trigger', 'Triggers', 'that read an ADB topic through it']] as [k, title, sub], i (k)}
            <div class="card">
              <div class="card-head"><h3>{i + 1} · {title}</h3><span>{sub}</span></div>
              {#if uses === null}<p class="dim">Looking…</p>
              {:else if uses.some((u) => u.kind === k)}
                <ul class="rows wide">{#each uses.filter((u) => u.kind === k) as u (u.id)}<li><span class="lbl">{u.name}</span><span class="val">{u.how} <span class="dim">· {u.project}</span></span>
                  <button class="btn2" onclick={() => k === 'flow' ? openInStudio('flow', u.project, u.name) : navigateTo(k === 'agent' ? 'agents' : k === 'canvas' ? 'canvas' : k === 'resource' ? 'resources' : 'triggers', title, { selectedId: k === 'canvas' ? u.name : u.id })}>Open</button></li>{/each}</ul>
              {:else}<p class="empty">None.</p>{/if}
            </div>
          {/each}
        {/if}

        </section></fieldset>
      </div></div>
      </div>
    </div>
  {/if}
</SlidePanel>

<!-- Delete Confirm -->
<DeleteConfirm
  open={!!deleteTarget}
  entityType="Resource"
  entityName={deleteTarget?.name ?? ''}
  onconfirm={deleteResource}
  oncancel={() => deleteTarget = null}
/>

<!-- Create Slide Panel -->
<SlidePanel open={showCreate} onclose={() => showCreate = false}>
  <div class="panel-layout">
    <PropertiesTree
      icon="+"
      color={CAT_COLORS[createForCategory] ?? '#999'}
      name={createForCategory ? `New ${catMeta(createForCategory).label.replace(/s$/, '')}` : 'New resource'}
      meta={createForCategory}
      nodes={createTreeNodes}
      steps={createSteps}
      wizard
      activeNode={createTab}
      onselect={(id) => { createTab = id; }}
    />
    <div class="panel-col">
    <div class="panel-content">
      <div class="pf">
        <WizardHead step={createSteps.find((x) => x.id === createTab)} />
        <section class="agt">
        {#if createTab === 'details'}
          <div class="card">
            <div class="card-head"><h3>1 · What it is</h3><span>its kind and type</span></div>
            <div class="seg src kinds" role="tablist">
              {#each Object.keys(TYPES_BY_CATEGORY) as c (c)}<button role="tab" class:on={createForCategory === c} onclick={() => pickKind(c)}>{catMeta(c).label}</button>{/each}
            </div>
            {#if createForCategory}
              <div class="fields"><label class="fld">Type
                <select bind:value={newResource.type}>{#each (TYPES_BY_CATEGORY[createForCategory] ?? []) as t (t.value)}<option value={t.value}>{t.label}</option>{/each}</select></label></div>
            {/if}
          </div>
          <div class="card">
            <div class="card-head"><h3>2 · Name</h3><span>how flows and agents refer to it: project/name</span></div>
            <div class="fields">
              <label class="fld">Name<input type="text" class="mono" bind:value={newResource.name} placeholder="my-resource" /></label>
              <label class="fld">Project
                <select bind:value={newResource.namespace}>
                  {#if !newResource.namespace}<option value="" disabled>Choose a project…</option>{/if}
                  {#each [...new Set([newResource.namespace, ...namespacesFor('owned')].filter(Boolean))] as ns (ns)}<option value={ns}>{ns}</option>{/each}
                </select></label>
            </div>
          </div>
        {:else if createTab === 'config'}
          <div class="card">
            <div class="card-head"><h3>1 · Configuration</h3><span>how FlowR connects to it, as JSON — change it any time after</span></div>
            {#if newResource.type === 'adb'}<p class="dim"><span class="mono">url</span> is its HTTP address (AQL, MCP at <span class="mono">/mcp/sse</span>); <span class="mono">call:adb</span> uses Arrow Flight on the same host, port 3001 — set <span class="mono">flight_url</span> only if it is elsewhere.</p>{/if}
            {#if createForCategory === 'credentials'}<div class="note">Put the secret in <span class="mono">value</span>. It is shown masked from then on.</div>{/if}
            <label class="fld">JSON<textarea rows="12" spellcheck="false" bind:value={newResource.config}></textarea></label>
            {#if !configOk}<p class="err">Not valid JSON yet.</p>{/if}
          </div>
        {:else if createTab === 'review'}
          <div class="card">
            <div class="card-head"><h3>1 · Review</h3><span>what will be created</span></div>
            <ul class="rows">
              <li><span class="lbl">Name</span><span class="val mono">{newResource.namespace}/{newResource.name || '—'}</span><span></span></li>
              <li><span class="lbl">Kind</span><span class="val">{createForCategory ? catMeta(createForCategory).label : '—'} · <span class="mono">{newResource.type || '—'}</span></span><span></span></li>
            </ul>
            <pre class="code">{createForCategory === 'credentials' ? newResource.config.replace(/("(?:value|token|password|api_key)"\s*:\s*")[^"]*"/g, '$1••••••"') : newResource.config}</pre>
          </div>
        {/if}
        </section>
      </div>
    </div>
    <WizardNav steps={createSteps} active={createTab} wizard onselect={(id) => { createTab = id; }}
      final={createResource} finalLabel="Create resource" finalBusy={creating} busyLabel="Creating…" />
    </div>
  </div>
</SlidePanel>

<!-- Page Header -->
<div class="screen-header">
  <div class="breadcrumb">Resources <span class="sep">/</span> Registry</div>
  <div class="screen-title-row">
    <div class="title-block">
      <SubmenuToggle appType="resources" />
      <h1 class="screen-title">Resources</h1>
      <button class="help-btn" onclick={() => docsOpen = true} title="Learn more">
        <svg width="14" height="14" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5"><circle cx="8" cy="8" r="7"/><path d="M6 6a2 2 0 114 0c0 1-1.5 1.5-2 2M8 11.5v.5"/></svg>
      </button>
    </div>
    <div class="screen-actions">
      {#if $canCreate}<NewButton label="New resource" accent="ephemeral" onclick={() => openCreate(activeCategory === 'all' ? '' : activeCategory)} />{/if}
    </div>
  </div>
</div>

<!-- Body only: the category menu is the shell's context pane now, so keeping one
     here put two identical column of categories side by side. -->
<div class="resources-layout">
  <div class="resources-body">
    {#if !loading}
      <PageBanner {...bannerFor('resources')} />
      <!-- Pinned and latest, under the banner: the section page leads with what
           it is for, then the items you actually return to. -->
      <AppHome appType="resources" embedded />

      {#if activeCategory === 'all'}
        <!-- Show all categories as sections -->
        {#each CATEGORIES.filter(c => c.key !== 'all') as cat}
          {@const items = allResources.filter(r => r.category === cat.key)}
          <div class="section-group">
            <div class="section-header">
              <span class="cat-icon-sm" style={cat.iconStyle}>{cat.icon}</span>
              <h3>{cat.label}</h3>
              <span class="section-count">{items.length}</span>
              <span style="flex: 1;"></span>
              <button class="section-add" onclick={() => openCreate(cat.key)}>
                <svg width="12" height="12" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round"><path d="M8 3v10M3 8h10"/></svg>
                + New
              </button>
            </div>
            {#if items.length > 0}
              <div class="card-grid">
                {#each items as r}
                  <EntityCard
                    icon={catMeta(r.category).icon}
                    color={CAT_COLORS[r.category] ?? '#999'}
                    name={r.name}
                    project={r.project ?? r.namespace}
                    description={r.description || r.type}
                    tags={[r.type, r.namespace]}
                    meta={r.created_at ? new Date(r.created_at).toLocaleDateString() : ''}
                    actionLabel="Test Connection"
                    onclick={() => selectResource(r)}
                    onaction={() => { selectResource(r); panelTab = 'test'; }}
                    ondelete={() => deleteTarget = { name: r.name, namespace: r.namespace }}
                    onpublish={isPlatform(r.project ?? r.namespace) ? undefined : () => openPublish({ kind: r.category === 'flows' ? 'flow' : 'resource', from: r.namespace, ref: r.name, label: r.name })}
                  />
                {/each}
              </div>
            {:else}
              <div class="empty-section-inline">No {cat.label.toLowerCase()} yet</div>
            {/if}
          </div>
        {/each}

        <ExposedSections kind="resource" noun="Resources" />
        <ExposedSections kind="flow" noun="Flows" />
      {:else}
        <!-- Single category view -->
        {@const items = filteredResources}
        <div class="section-group">
          <div class="section-header">
            <h3>{catMeta(activeCategory).label}</h3>
            <span class="section-count">{items.length}</span>
            <span style="flex: 1;"></span>
            <button class="section-add" onclick={() => openCreate(activeCategory)}>
              <svg width="12" height="12" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round"><path d="M8 3v10M3 8h10"/></svg>
              + New
            </button>
          </div>
          {#if items.length > 0}
            <div class="card-grid">
              {#each items as r}
                <EntityCard
                  icon={catMeta(r.category).icon}
                  color={CAT_COLORS[r.category] ?? '#999'}
                  name={r.name}
                    project={r.project ?? r.namespace}
                  description={r.description || r.type}
                  tags={[r.type, r.namespace]}
                  meta={r.created_at ? new Date(r.created_at).toLocaleDateString() : ''}
                  actionLabel="Test Connection"
                  onclick={() => selectResource(r)}
                  onaction={() => { selectResource(r); panelTab = 'test'; }}
                  ondelete={() => deleteTarget = { name: r.name, namespace: r.namespace }}
                    onpublish={isPlatform(r.project ?? r.namespace) ? undefined : () => openPublish({ kind: r.category === 'flows' ? 'flow' : 'resource', from: r.namespace, ref: r.name, label: r.name })}
                />
              {/each}
            </div>
          {:else}
            <div class="empty-section">
              <p>No {catMeta(activeCategory).label.toLowerCase()} registered yet.</p>
              {#if $canCreate}
                <button class="btn spec sm" onclick={() => openCreate(activeCategory)}>+ Add {catMeta(activeCategory).label.replace(/s$/, '')}</button>
              {/if}
            </div>
          {/if}
        </div>
      {/if}
    {:else}
      <!-- the header bar shows loading -->
    {/if}
  </div>
</div>

<DocsPanel open={docsOpen} pageKey="resources" onClose={() => docsOpen = false} />

<style>
  /* Header */
  .screen-header { padding: 18px 32px 12px; border-bottom: 1px solid var(--border); background: var(--bg-page); flex-shrink: 0; }
  .breadcrumb { font-size: 12px; color: var(--text-3); margin-bottom: 8px; font-family: var(--font-mono); }
  .breadcrumb .sep { color: var(--text-5); }
  .screen-title-row { display: flex; align-items: center; gap: 18px; }
  .title-block { flex: 1; display: flex; align-items: center; gap: 10px; }
  .screen-title { font-family: var(--font-serif); font-size: 24px; font-weight: 500; letter-spacing: -0.02em; }
  .help-btn { width: 24px; height: 24px; border: 1px solid var(--border); border-radius: 50%; background: var(--bg-surface); color: var(--text-4); display: grid; place-items: center; cursor: pointer; }
  .help-btn:hover { background: var(--spec-bg); border-color: var(--spec); color: var(--spec); }
  .screen-actions { display: flex; gap: 8px; }

  /* Layout: sidebar + body */
  .resources-layout { flex: 1; display: flex; overflow: hidden; }

  /* Category menu */

  /* Body */
  .resources-body { flex: 1; overflow-y: auto; padding: 20px 32px 40px; }

  /* Section groups */
  .section-group { margin-bottom: 28px; }
  .section-header {
    display: flex; align-items: center; gap: 10px;
    padding-bottom: 10px; margin-bottom: 12px; border-bottom: 1px solid var(--divider);
  }
  .section-header h3 { font-family: var(--font-serif); font-size: 16px; font-weight: 500; color: var(--text-1); }
  .section-count { font-family: var(--font-mono); font-size: 11px; color: var(--text-4); background: var(--bg-subtle); padding: 1px 6px; border-radius: 3px; }
  .cat-icon-sm {
    width: 20px; height: 20px; border-radius: 4px;
    display: inline-grid; place-items: center;
    font-size: 9px; font-weight: 700; font-family: var(--font-mono); flex-shrink: 0;
  }
  .section-add {
    display: flex; align-items: center; gap: 4px;
    padding: 4px 10px; border: 1px dashed var(--border); border-radius: var(--r-sm);
    background: none; color: var(--text-3); font-size: 11px;
    cursor: pointer; transition: all 0.12s;
  }
  .section-add:hover { background: var(--bg-subtle); color: var(--text-1); border-color: var(--border-strong); }

  /* Card grid */
  .card-grid { display: grid; grid-template-columns: repeat(auto-fill, minmax(280px, 1fr)); gap: 12px; }

  /* Empty categories */

  .empty-section {
    padding: 40px 20px; text-align: center;
    background: var(--bg-surface); border: 1px solid var(--border); border-radius: var(--r);
  }
  .empty-section p { color: var(--text-4); font-size: 13px; margin-bottom: 12px; }
  .empty-section-inline {
    padding: 14px 16px; font-size: 12px; color: var(--text-4);
    background: var(--bg-surface); border: 1px solid var(--border); border-radius: var(--r);
  }

  /* Create form fields */
  .create-actions { display: flex; gap: 8px; justify-content: flex-end; margin-top: 20px; padding-top: 16px; border-top: 1px solid var(--divider); }
  .pf-footer { align-items: center; margin: 0 24px 24px; }
  .pf-footer .pf-hint { margin: 0 auto 0 0; }

  /* Panel layout (matches AgentsList) */
  .panel-layout { display: grid; grid-template-columns: 280px 1fr; height: 100%; }
  .panel-content { overflow-y: auto; }
  .mono-area { font-family: var(--font-mono); font-size: 12.5px; line-height: 1.55; }
  .panel-col { display: flex; flex-direction: column; min-height: 0; height: 100%; }
  .panel-col > .panel-content { flex: 1; min-height: 0; }
  .panel-content-flush { overflow: hidden; }
  .flow-def-pane { height: 100%; overflow: hidden; }

  /* Execution logs */
  .logs-head { display: flex; align-items: center; justify-content: space-between; gap: 12px; }
  .logs-head h3 { margin-bottom: 0; }
  .exec-list { display: flex; flex-direction: column; gap: 6px; margin-top: 8px; }
  .exec-row { border: 1px solid var(--border); border-radius: var(--r-sm); background: var(--bg-surface); overflow: hidden; }
  .exec-summary { display: flex; align-items: center; gap: 12px; padding: 9px 12px; cursor: pointer; font-size: 12px; list-style: none; }
  .exec-summary::-webkit-details-marker { display: none; }
  .exec-status { font-size: 10px; font-weight: 600; padding: 2px 8px; border-radius: 3px; text-transform: capitalize; background: var(--bg-subtle); color: var(--text-3); flex-shrink: 0; }
  .exec-status.completed { background: var(--ok-bg, #e3f4ef); color: var(--ok, #23987f); }
  .exec-status.failed { background: var(--err-bg, #fde8e8); color: var(--err, #d44); }
  .exec-status.running { background: var(--spec-bg); color: var(--spec-ink); }
  .exec-time { color: var(--text-2); flex: 1; }
  .exec-dur { color: var(--text-4); font-family: var(--font-mono); font-size: 11px; }
  .exec-id { color: var(--text-4); font-size: 11px; }
  .exec-detail { padding: 10px 12px; border-top: 1px solid var(--divider); display: flex; flex-direction: column; gap: 8px; }
  .exec-err { font-size: 12px; color: var(--err, #d44); font-family: var(--font-mono); background: var(--err-bg, #fde8e8); padding: 8px 10px; border-radius: var(--r-sm); }
  .exec-kv { display: flex; flex-direction: column; gap: 3px; }
  .exec-kv > span { font-size: 10px; font-weight: 600; text-transform: uppercase; letter-spacing: 0.04em; color: var(--text-4); }
  .pf { padding: 24px 32px; max-width: var(--detail-w); box-sizing: border-box; }
  @container details (max-width: 560px) { .pf { padding: 16px 14px; } }
  .rs-fs { border: 0; margin: 0; padding: 0; min-width: 0; }
  .def-pane { height: max(520px, calc(100vh - 340px)); border: 1px solid var(--border); border-radius: 10px; overflow: hidden; display: flex; flex-direction: column; }
  .def-pane > :global(*) { flex: 1; min-height: 0; }
  /* Settings: each a labelled field with its Remove beside it. */
  .kvf { display: grid; grid-template-columns: 1fr auto; gap: 10px 10px; align-items: end; }
  /* Ask a model: the conversation, then the box. */
  .msgs { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 8px; }
  .msgs li { display: flex; flex-direction: column; gap: 2px; padding: 8px 12px; border-radius: 8px; background: var(--bg-subtle); font-size: 13px; color: var(--text-1); white-space: pre-wrap; }
  .msgs li.user { background: var(--spec-bg); }
  .msgs b { font-size: 11.5px; color: var(--text-3); font-weight: 600; }
  .ask { height: 34px; padding: 0 12px; border: 1px solid var(--border); border-radius: 8px; background: var(--bg-surface); font: 13px var(--font-sans); color: var(--text-1); min-width: 0; }
  .seg.kinds { flex-wrap: wrap; }
  .pf h3 { font-family: var(--font-serif); font-size: 18px; font-weight: 500; margin-bottom: 16px; }
  .pf-kv { display: flex; flex-direction: column; }
  .pf-row { display: flex; justify-content: space-between; align-items: center; padding: 9px 0; border-bottom: 1px solid var(--divider); font-size: 13px; gap: 12px; }
  .pf-row:last-child { border-bottom: 0; }
  .pf-k { color: var(--text-3); flex-shrink: 0; }
  .pf-v { color: var(--text-1); text-align: right; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .pf-hint { font-size: 12px; color: var(--text-4); margin-bottom: 12px; }
  .pf-code { font-family: var(--font-mono); font-size: 12px; line-height: 1.6; padding: 12px 16px; margin: 0; background: var(--bg-subtle); border: 1px solid var(--border); border-radius: var(--r-sm); white-space: pre; overflow: auto; max-height: 300px; }
  /* LLM/resource config fields */
  .cf-grid2 { display: grid; grid-template-columns: 1fr 1fr; gap: 12px; }
  .cfg-actions { display: flex; align-items: center; gap: 10px; margin-top: 8px; }
  .cfg-msg { font-size: 12px; color: var(--text-4); }
  .cfg-msg.ok { color: var(--ok, #23987f); }
  .cfg-edit { display: flex; flex-direction: column; gap: 8px; margin-bottom: 4px; }
  .cfg-edit-row { display: grid; grid-template-columns: minmax(0, 1fr) minmax(0, 2fr) auto; gap: 8px; align-items: start; }
  .cfg-edit-row input, .cfg-edit-row select, .cfg-edit-row textarea { width: 100%; font-size: 12px; }
  .cfg-add { align-self: flex-start; }
  .cfg-raw { margin-top: 16px; }
  .cfg-raw summary { font-size: 12px; color: var(--text-4); cursor: pointer; }
  .cfg-raw pre { margin-top: 8px; }
  .pf-empty-state { padding: 20px; text-align: center; color: var(--text-4); font-size: 13px; background: var(--bg-subtle); border-radius: var(--r-sm); }

  .loading-msg { padding: 40px; text-align: center; color: var(--text-4); font-size: 14px; }

  /* ── LLM Chat test ── */
  .test-chat { display: flex; flex-direction: column; gap: 12px; }
  .chat-messages {
    max-height: 320px; overflow-y: auto;
    border: 1px solid var(--border); border-radius: var(--r-sm);
    background: var(--bg-subtle); padding: 12px;
    display: flex; flex-direction: column; gap: 8px;
  }
  .chat-empty { color: var(--text-4); font-size: 12px; text-align: center; padding: 24px; }
  .chat-msg { display: flex; flex-direction: column; gap: 2px; }
  .chat-role { font-size: 10px; font-family: var(--font-mono); color: var(--text-4); text-transform: uppercase; letter-spacing: 0.05em; }
  .chat-content {
    font-size: 13px; line-height: 1.5; color: var(--text-1);
    padding: 8px 12px; border-radius: var(--r-sm);
    white-space: pre-wrap; word-break: break-word;
  }
  .chat-user .chat-content { background: var(--spec-bg); }
  .chat-assistant .chat-content { background: var(--bg-surface); border: 1px solid var(--border); }
  .chat-typing { color: var(--text-3); font-style: italic; }
  .chat-input-row { display: flex; gap: 8px; }
  .chat-input {
    flex: 1; padding: 8px 12px; border: 1px solid var(--border); border-radius: var(--r-sm);
    background: var(--bg-surface); color: var(--text-1); font-size: 13px;
    font-family: var(--font-sans);
  }
  .chat-input:focus { outline: none; border-color: var(--spec); }

  /* ── DB test ── */
  .test-db { display: flex; flex-direction: column; gap: 12px; }

  /* ── MCP tools ── */
  .mcp-tools {
    border: 1px solid var(--border); border-radius: var(--r-sm);
    background: var(--bg-surface); margin-bottom: 16px; max-height: 240px; overflow-y: auto;
  }
  .mcp-tools-header {
    padding: 8px 12px; font-size: 11px; color: var(--text-3);
    border-bottom: 1px solid var(--divider); font-family: var(--font-mono);
  }
  .mcp-tool-item {
    display: flex; flex-direction: column; gap: 2px; width: 100%;
    padding: 8px 12px; border: none; border-bottom: 1px solid var(--divider);
    background: none; text-align: left; cursor: pointer; transition: background 0.1s;
  }
  .mcp-tool-item:last-child { border-bottom: none; }
  .mcp-tool-item:hover { background: var(--bg-subtle); }
  .mcp-tool-item.active { background: var(--spec-bg); }
  .mcp-tool-name { font-size: 12px; font-family: var(--font-mono); color: var(--text-1); font-weight: 500; }
  .mcp-tool-desc { font-size: 11px; color: var(--text-3); }
  .mcp-call-section { margin-top: 4px; padding-top: 12px; border-top: 1px solid var(--divider); }
  .mcp-call-header { font-size: 12px; font-family: var(--font-mono); color: var(--spec); margin-bottom: 8px; }

  /* ── Secret display ── */
  .secret-display {
    border: 1px solid var(--border); border-radius: var(--r-sm);
    background: var(--bg-surface); padding: 12px 16px;
  }
</style>
