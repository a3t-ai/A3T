<script lang="ts">
  import { type WizardStep } from '$lib/components/shared/WizardSteps.svelte';
  import WizardNav from '$lib/components/shared/WizardNav.svelte';
  import WizardHead from '$lib/components/shared/WizardHead.svelte';
  import AgentOverview from '$lib/components/agent/AgentOverview.svelte';
  import AgentAccess from '$lib/components/agent/AgentAccess.svelte';
  import AgentExposure from '$lib/components/agent/AgentExposure.svelte';
  import AgentControls from '$lib/components/agent/AgentControls.svelte';
  import AgentConfig from '$lib/components/agent/AgentConfig.svelte';
  import AgentReleases from '$lib/components/agent/AgentReleases.svelte';
  import AgentAudit from '$lib/components/agent/AgentAudit.svelte';
  import AgentSettings from '$lib/components/agent/AgentSettings.svelte';
  import AgentHooks from '$lib/components/agent/AgentHooks.svelte';
  import AgentLineage from '$lib/components/agent/AgentLineage.svelte';
  import AgentSharing from '$lib/components/agent/AgentSharing.svelte';
  import { control } from '$lib/components/agent/controlPlane';
  import { liveFor } from '$lib/components/agent/agentLive.svelte';
  import '$lib/components/agent/agent.css';
  import AgentMakeFrom from '$lib/components/agent/AgentMakeFrom.svelte';
  import { createAgent as createAgentRec, updateAgent } from '$lib/studio-core/adapters/agents';
  import ExposedSections from '$lib/components/shared/ExposedSections.svelte';
  import { namespacesFor } from '$lib/stores/context.svelte.ts';
  import SubmenuToggle from '$lib/components/shared/SubmenuToggle.svelte';
  import NewButton from '$lib/components/shared/NewButton.svelte';
  import { execFlow, mcpCall, canvasApi, agentTool } from '$lib/services/api';
  import { serverRef } from '$lib/services/mcpTools';
  import { readTemplate } from '$lib/components/templates/templateSpec';
  import { navigateTo, navigateToDetail, getActivePanel, openInStudio } from '$lib/stores/panels.svelte.ts';
  import { can } from '$lib/stores/auth';
  import { onMount, untrack } from 'svelte';
  import * as YAML from 'yaml';
  import SlidePanel from '$lib/components/shared/SlidePanel.svelte';
  import PageBanner from '$lib/components/shared/PageBanner.svelte';
  import AppHome from '$lib/shell/AppHome.svelte';
  import EntityCard from '$lib/components/shared/EntityCard.svelte';
  import FoldSection from '$lib/components/shared/FoldSection.svelte';
  import { currentUser } from '$lib/stores/auth';
  import { getProject, getProjects, inContext, projectOf, EXPOSED_ROLES } from '$lib/stores/context.svelte.ts';
  import PropertiesTree from '$lib/components/shared/PropertiesTree.svelte';
  import DeleteConfirm from '$lib/components/shared/DeleteConfirm.svelte';
  import ResourceSelect from '$lib/components/editor/ResourceSelect.svelte';
  import { trackActiveItem } from '$lib/stores/activeItem.svelte.ts';

  const canCreate = can('create', 'agent');
  const canEditAgent = can('update', 'agent');

  let loading = $state(true);
  let error = $state('');
  let selected = $state('');
  let propNode = $state('overview');

  let allAgents: any[] = $state([]);
  /// In view for the project; split by who created them.
  const visibleAgents = $derived(allAgents.filter((a) => inContext(a.project, 'shared')));
  const myAgents = $derived(visibleAgents.filter((a) => a.created_by === $currentUser?.id));
  const sharedAgents = $derived(visibleAgents.filter((a) => a.created_by !== $currentUser?.id));
  /// New agents go into the selected project (`default` under "All projects").
  const newNamespace = () => getProject() || namespaceChoices()[0] || 'default';
  /// Projects a new agent can go in: yours (not ones only exposed to you), the
  /// selected one first; the current value stays listed even if it is none of them.
  function namespaceChoices(current?: string): string[] {
    const mine = getProjects().filter((p) => !p.platform && !(EXPOSED_ROLES as readonly string[]).includes(p.info?.role ?? '')).map((p) => p.name);
    const sel = getProject();
    const list = [...(sel ? [sel] : []), ...mine.filter((n) => n !== sel)];
    return current && !list.includes(current) ? [current, ...list] : list;
  }
  /// One entry per template release (name + version). A release passed on from
  /// system to a3t is listed twice by the registry; keep the copy nearest you.
  const templateChoices = $derived.by(() => {
    const nsOf = (y: string) => (y ?? '').match(/^\s+namespace:\s*['"]?([\w.-]+)/m)?.[1] ?? '';
    const best = new Map<string, { id: string; name: string; version: string; ns: string }>();
    for (const t of templates) {
      const k = `${t.name}@${t.version}`;
      const it = { id: t.id, name: t.name, version: t.version, ns: nsOf(t.spec_yaml) };
      const cur = best.get(k);
      if (!cur || (cur.ns.startsWith('system') && !it.ns.startsWith('system'))) best.set(k, it);
    }
    // The one already chosen (a deep link) stays selectable.
    const chosen = templates.find((t) => t.id === newAgent.template_id);
    if (chosen && ![...best.values()].some((b) => b.id === chosen.id)) best.set(`${chosen.id}`, { id: chosen.id, name: chosen.name, version: chosen.version, ns: nsOf(chosen.spec_yaml) });
    return [...best.values()].sort((a, b) => a.name.localeCompare(b.name) || b.version.localeCompare(a.version));
  });
  let templates: { id: string; name: string; version: string; spec_yaml: string }[] = $state([]);

  // Default target flow (path) for an agent, derived from its template's spec document.
  // Agents must target a CHAT WRAPPER that assembles the `config` object (system_prompt,
  // llm_resource, mcp_servers) from the flat webhook body. The raw system flows
  // (system/agent-engine, system/agent-template) expect a full `config` and would receive
  // null → "invalid agent config: invalid type: null, expected struct AgentSettings".
  function deriveTarget(specYaml: string): string {
    // An agent ALWAYS runs through a chat WRAPPER flow (a registered workflow that
    // assembles the config + reasoning loop). A template is NOT a workflow, so the
    // target must never be the template's own name (→ "Workflow not found").
    try {
      const doc = YAML.parse(specYaml) ?? {};
      const d = doc.document ?? doc;
      const name = d?.name;
      void name;
    } catch {}
    // Every agent runs through the A3T agent template (a3t/tmpl_a3t_agent), which
    // reads the agent's record and runs the core agent with its values.
    return 'a3t/tmpl_a3t_agent';
  }

  function onTemplateChange() {
    const t = templates.find(t => t.id === newAgent.template_id);
    if (t) {
      newAgent.template_version = t.version;
      newAgent.target = deriveTarget(t.spec_yaml);
      const cfg = templateConfig(t.spec_yaml);
      if (cfg.llm_resource) newAgent.llm = String(cfg.llm_resource).includes('/') ? cfg.llm_resource : `a3t/${cfg.llm_resource}`;
      if (Array.isArray(cfg.mcp_servers)) newAgent.mcp_servers = cfg.mcp_servers;
      // Inherit the template's capability ceiling as the new agent's default.
      newAgent.enabled_tools = Array.isArray(cfg.enabled_tools) ? [...cfg.enabled_tools] : null;
    }
  }

  // When true, the slide-in panel is creating a not-yet-persisted agent. The
  // same panel serves both editing and creation; `agent` reflects the draft
  // (from newAgent) in create mode so the tree header updates as you type.
  let creatingNew = $state(false);
  let agent = $derived(
    creatingNew
      ? { ...newAgent, id: '(new)', isNew: true, status: 'draft', created_by: '' }
      // The shell's Items list addresses an agent by name; links from elsewhere
      // use its id. Accept either.
      : (selected ? (allAgents.find(a => a.id === selected) ?? allAgents.find(a => a.name === selected) ?? null) : null)
  );

  // Delete state
  let deleteTarget = $state<{ id: string; name: string } | null>(null);

  // Create form — all references are selections (no loose strings)
  let newAgent = $state({
    name: '', namespace: newNamespace(), template_id: '', template_version: '1.0.0',
    mode: 'session', target: '', ep_path: '',
    llm: 'a3t/claude-cli-max', mcp_servers: [] as string[], enabled_tools: null as string[] | null,
  });
  let creating = $state(false);
  const newTplFacts = $derived(readTemplate(templates.find((t) => t.id === newAgent.template_id)?.spec_yaml));

  function newAgentPanel() {
    selected = '';
    newAgent = {
      name: '', namespace: newNamespace(), template_id: '', template_version: '1.0.0',
      mode: 'session', target: '', ep_path: '', llm: 'a3t/claude-cli-max', mcp_servers: [], enabled_tools: null,
    };
    creatingNew = true;
    propNode = 'identity';
    makeKind = null;
  }

  // Tree nodes
  /// A new agent is made in four steps; an existing one has a tab per question.
  const agentSteps = $derived.by((): WizardStep[] => creatingNew ? [
    { id: 'identity', label: 'Template & name', hint: 'The template it is built from, its name, and the project it lives in', ok: !!newAgent.name.trim() && !!newAgent.namespace && !!newAgent.template_id,
      need: !newAgent.template_id ? 'Pick a template.' : !newAgent.name.trim() ? 'Name the agent.' : 'Pick a project.' },
    { id: 'behaviour', label: 'Model & tools', hint: 'The model it thinks with, the tools it may call, and whether it remembers' },
    { id: 'serving', label: "How it's started", hint: 'Its endpoint and the flow that serves it — the template\'s choice is filled in' },
    { id: 'review', label: 'Review', hint: 'Check it, then create it. It is not deployed until you deploy it.' },
  ] : treeNodes.map((n) => ({ id: n.id, label: n.label, icon: n.icon, hint: AGENT_HINTS[n.id] })));
  const AGENT_HINTS: Record<string, string> = {
    overview: 'What it is, how to call it, what needs attention',
    hooks: 'Every phase of a turn, and the flows that run there: core, template, this agent',
    lineage: 'Where it came from, what came from it, and what flows back',
    sharing: 'Who can see it, and what each namespace may do with it',
    access: 'Everything it can reach: identity, model, tools, data, memory',
    exposure: 'How it exposes itself (REST, MCP, A2A) and who has called it',
    controls: 'Stop it, and what governs it: hooks, policies, approvals',
    audit: 'What it did and what changed',
    releases: 'Where it runs and which version; branches and forks',
    settings: 'Name, environment, owner, where it came from, delete',
  };

  /// An existing agent's tabs. The console's readers are mostly security and
  /// governance teams (developers build in VS Code with Claude, over MCP), so
  /// the tabs follow a review: what it is, what it can reach, who can reach it,
  /// how to stop it, what it did, where it runs.
  const treeNodes = [
    { id: 'overview', label: 'Overview', icon: '<rect x="2" y="2" width="12" height="12" rx="2"/><path d="M5 6h6M5 9h4"/>' },
    { id: 'hooks', label: 'Hooks', icon: '<path d="M5 2v6a3 3 0 006 0V5"/><path d="M9 7l2-2 2 2"/><circle cx="5" cy="13" r="1.6"/>' },
    { id: 'lineage', label: 'Lineage', icon: '<circle cx="8" cy="3" r="1.6"/><circle cx="4" cy="13" r="1.6"/><circle cx="12" cy="13" r="1.6"/><path d="M8 4.6V8M8 8l-4 3.4M8 8l4 3.4"/>' },
    { id: 'sharing', label: 'Sharing', icon: '<circle cx="12" cy="4" r="2"/><circle cx="4" cy="8" r="2"/><circle cx="12" cy="12" r="2"/><path d="M5.8 7l4.4-2M5.8 9l4.4 2"/>' },
    { id: 'access', label: 'Access', icon: '<circle cx="5.5" cy="8" r="3"/><path d="M8.5 8H15M12.5 8v3M14.5 8v2"/>' },
    { id: 'exposure', label: 'Exposure', icon: '<circle cx="8" cy="8" r="6.5"/><path d="M1.5 8h13M8 1.5c2 2 2 11 0 13M8 1.5c-2 2-2 11 0 13"/>' },
    { id: 'controls', label: 'Controls', icon: '<path d="M8 1L2 4v4c0 4 3 6.5 6 7.5 3-1 6-3.5 6-7.5V4z"/>' },
    { id: 'audit', label: 'Audit', icon: '<path d="M1 8h3l2-5 3 10 2-5h4"/>' },
    { id: 'releases', label: 'Releases', icon: '<circle cx="4" cy="3" r="1.6"/><circle cx="4" cy="13" r="1.6"/><circle cx="12" cy="5" r="1.6"/><path d="M4 4.6v6.8M12 6.6c0 3-3 3.4-8 4.8"/>' },
    { id: 'settings', label: 'Settings', icon: '<circle cx="8" cy="5" r="3"/><path d="M2 14a6 6 0 0112 0"/>' },
  ];
  /// Tab names other pages (and earlier versions) link with.
  const TAB_ALIAS: Record<string, string> = {
    detail: 'overview', api: 'exposure', config: 'access', configure: 'access', policy: 'controls', canvas: 'releases', deploy: 'releases',
    branches: 'releases', health: 'audit', activity: 'audit', observe: 'audit', memory: 'audit', ownership: 'settings',
  };
  let activityStart = $state<'memory' | undefined>(undefined);

  async function loadAgents() {
    loading = true; error = '';
    const res = await execFlow('flow_a3t_agents-list', 'a3t', { org_id: 'org-default' });
    if (res.ok) {
      allAgents = (Array.isArray(res.data) ? res.data : (res.data?.rows ?? res.data?.output ?? [])).map((r: any) => ({
        id: r.id, name: r.name, namespace: r.namespace ?? 'a3t',
        // The project it shows under; `namespace` stays what is stored.
        project: projectOf(r.namespace ?? 'a3t', r.owner ?? r.created_by),
        mode: r.mode ?? 'session', status: r.status ?? 'draft',
        template_id: r.template_id, template_version: r.template_version ?? '1.0.0',
        variables: r.variables, resource_bindings: r.resource_bindings,
        description: (parseJson(r.variables ?? '') || {}).description ?? '',
        created_at: r.created_at, created_by: r.owner ?? r.created_by ?? '',
      }));
    } else { error = res.error ?? 'Failed to load agents'; }
    loading = false;
  }

  async function loadTemplates() {
    const res = await execFlow('flow_a3t_templates-list', 'a3t', { org_id: 'org-default' });
    if (res.ok) {
      templates = (Array.isArray(res.data) ? res.data : (res.data?.rows ?? res.data?.output ?? [])).map((r: any) => ({ id: r.id, name: r.name, version: r.version ?? '1.0.0', spec_yaml: r.spec_yaml ?? '' }));
    }
  }

  // ── Registry: everything selectable, no loose strings ──
  const LLM_TYPES = ['openai', 'anthropic', 'groq', 'ollama'];
  let llmResources = $state<{ value: string; label: string }[]>([]);
  let mcpResources = $state<{ value: string; label: string }[]>([]);
  let flowOptions = $state<{ value: string; label: string }[]>([]);
  let triggerOptions = $state<{ value: string; label: string }[]>([]);

  async function loadRegistry() {
    const llm: any[] = [], mcp: any[] = [], fl: any[] = [];
    for (const ns of namespacesFor('picker')) {
      const r = await mcpCall('list_resources', { namespace: ns });
      for (const res of (r.data?.resources ?? [])) {
        const ref = `${res.namespace ?? ns}/${res.name}`;
        if (LLM_TYPES.includes(res.type)) llm.push({ value: `${res.namespace ?? ns}/${res.name}`, label: `${res.name} (${res.type})` });
        if (res.type === 'mcp') mcp.push({ value: ref, label: res.name });
      }
      const f = await mcpCall('list_flows', { namespace: ns });
      for (const w of (f.data?.workflows ?? [])) fl.push({ value: `${w.namespace ?? ns}/${w.name}`, label: `${w.namespace ?? ns}/${w.name}` });
    }
    // One entry per reference: projects can list the same shared resource.
    const once = (xs: { value: string; label: string }[]) => [...new Map(xs.map((x) => [x.value, x])).values()];
    llmResources = once(llm); mcpResources = once(mcp); flowOptions = once(fl);
    await loadTriggerOptions();
  }

  async function loadTriggerOptions() {
    const t = await mcpCall('list_triggers', {});
    let d: any = t.data; if (typeof d === 'string') { try { d = JSON.parse(d); } catch { d = []; } }
    const list = Array.isArray(d) ? d : (d?.triggers ?? []);
    triggerOptions = list
      .filter((x: any) => x?.config?.path)
      .map((x: any) => ({ value: x.config.path, label: `${x.config.path} → ${x.targets?.[0]?.name ?? '?'}` }));
  }

  function selectAgent(id: string) {
    selected = id;
    propNode = 'overview';
    makeKind = null;
    navigateToDetail('agents', 'Agents', id);
  }

  function closePanel() {
    selected = '';
    creatingNew = false;
    navigateTo('agents', 'Agents');
  }

  async function createAgent() {
    if (!newAgent.name || !newAgent.template_id) return;
    creating = true;
    const id = `agt-${crypto.randomUUID().slice(0, 8)}`;
    const target = newAgent.target || 'a3t/tmpl_a3t_agent';
    // Endpoint: selected existing trigger path, else auto /agent/{id}/chat (created below).
    const usingExisting = !!newAgent.ep_path && triggerOptions.some(t => t.value === newAgent.ep_path);
    const epPath = newAgent.ep_path || `/agent/${id}/chat`;
    const vars = { target, ep_path: epPath, mcp_servers: newAgent.mcp_servers, enabled_tools: newAgent.enabled_tools ?? null };
    // Made by the control plane: agent_a3t_agents's tool_a3t_agent_create records the
    // agent, where it came from (its template) and returns its endpoints.
    const res = await agentTool('agent_a3t_agents', 'tool_a3t_agent_create', {
      org_id: 'org-default', user_id: $currentUser?.id ?? '',
      id, name: newAgent.name, project: newAgent.namespace,
      from: { kind: 'template', id: newAgent.template_id, version: newAgent.template_version },
      mode: 'template', agent_mode: newAgent.mode,
      values: { ...vars, llm_resource: newAgent.llm },
      resource_bindings: { llm: newAgent.llm },
    });
    if (res.ok) {
      // Deploy the agent's endpoint (webhook trigger → target flow) unless reusing an existing trigger.
      if (!usingExisting) {
        const [tns, tname] = target.includes('/') ? [target.split('/')[0], target.split('/').slice(1).join('/')] : ['a3t', target];
        await mcpCall('create_trigger', {
          name: `agent-${id}-chat`, namespace: newAgent.namespace,
          config: { type: 'webhook', path: epPath },
          targets: [{ type: 'workflow', namespace: tns, name: tname }],
          enabled: true,
        });
        await loadTriggerOptions();
      }
      creatingNew = false;
      newAgent = { name: '', namespace: newNamespace(), template_id: '', template_version: '1.0.0', mode: 'session', target: '', ep_path: '', llm: 'a3t/claude-cli-max', mcp_servers: [], enabled_tools: null };
      await loadAgents();
      selectAgent(id);
    } else { error = res.error ?? 'Create failed'; }
    creating = false;
  }

  /// Delete an agent, its endpoint and its own canvas. An error, or null.
  async function removeAgent(a: any): Promise<string | null> {
    const res = await execFlow('flow_a3t_agents-delete', 'a3t', { id: a.id, org_id: 'org-default' });
    if (!res.ok) return res.error ?? 'It was not deleted.';
    // Its lineage and shares go with it.
    await agentTool('agent_a3t_agents', 'tool_a3t_agent_forget', { org_id: 'org-default', id: a.id });
    const vars = parseJson(a.variables ?? '') || {};
    const ep = vars.ep_path || `/agent/${a.id}/chat`;
    // Its endpoint: the trigger made for it (by name), or the one on its path.
    const tr = await mcpCall('list_triggers', {});
    let d: any = tr.data; if (typeof d === 'string') { try { d = JSON.parse(d); } catch { d = []; } }
    const list: any[] = Array.isArray(d) ? d : (d?.triggers ?? []);
    const trig = list.find((t) => t?.name === `agent-${a.id}-chat`) ?? list.find((t) => t?.config?.path === ep && t?.name?.startsWith('agent-'));
    const left: string[] = [];
    if (trig?.id) { const r = await mcpCall('delete_trigger', { id: trig.id }); if (!r.ok) left.push(`its endpoint ${ep}`); }
    // Its own canvas (made for it, bound by agentRef) — never a shared profile.
    const own = canvases.find((c) => c.config?.agentRef === `a3t/${a.id}`);
    if (own) { const r = await canvasApi.remove(own.namespace ?? 'a3t', own.name); if (!r.ok) left.push(`its canvas ${own.name}`); }
    await Promise.all([loadAgents(), loadTriggerOptions(), loadCanvases()]);
    return left.length ? `Deleted, but ${left.join(' and ')} could not be removed.` : null;
  }

  async function deleteAgent() {
    if (!deleteTarget) return;
    const a = allAgents.find((x) => x.id === deleteTarget!.id);
    const err = a ? await removeAgent(a) : 'It is no longer listed.';
    deleteTarget = null;
    if (err) error = err; else if (selected === a?.id) selected = '';
  }

  /// Settings → Delete: on success the Details pane closes.
  async function deleteCurrent(): Promise<string | null> {
    if (!agent || creatingNew) return null;
    const err = await removeAgent(agent);
    if (err && !err.startsWith('Deleted')) return err;
    selected = '';
    navigateTo('agents', 'Agents');
    if (err) error = err;
    return null;
  }

  // ── Clone / fork: the two ways an agent is made from another ──
  let makeKind = $state<'clone' | 'fork' | null>(null);
  let branchForm = $state(false);
  function lifecycle(kind: 'clone' | 'fork' | 'branch') {
    if (kind === 'branch') { branchForm = true; propNode = 'releases'; return; }
    makeKind = kind;
  }
  async function makeFrom(name: string, project: string): Promise<string | null> {
    if (!agent || !makeKind) return null;
    if (allAgents.some((a) => a.name === name && a.namespace === project)) return `There is already an agent called ${name} in ${project}.`;
    const id = `agt-${crypto.randomUUID().slice(0, 8)}`;
    const vars = parseJson(agent.variables ?? '') || {};
    // Its own endpoint and no deployment; where it came from.
    delete vars.canvasRef; delete vars.clonedFrom; delete vars.forkedFrom;
    vars.ep_path = `/agent/${id}/chat`;
    vars[makeKind === 'fork' ? 'forkedFrom' : 'clonedFrom'] = agent.id;
    vars.fromName = agent.name;
    const r = await createAgentRec({
      id, name, namespace: project, template_id: agent.template_id, template_version: agent.template_version,
      mode: agent.mode, resource_bindings: typeof agent.resource_bindings === 'string' ? agent.resource_bindings : JSON.stringify(agent.resource_bindings ?? {}),
      variables: JSON.stringify(vars),
    }, vars.target || 'a3t/tmpl_a3t_agent', vars.ep_path);
    if (!r.ok && !String(r.error ?? '').startsWith('Agent created')) return r.error ?? 'It was not made.';
    // Where it came from, for Lineage (the control plane's agent_relations).
    await control.relate(id, { kind: 'agent', id: agent.id, version: agent.template_version }, makeKind);
    makeKind = null;
    await Promise.all([loadAgents(), loadTriggerOptions()]);
    selectAgent(id);
    if (!r.ok) error = r.error ?? '';
    return null;
  }

  function discardBindings() { saveBindMsg = ''; initBindings(); }
  /// Its template's code, in the Studio.
  function openCode() {
    const t = agentTemplate;
    if (!t) return;
    openInStudio('template', readTemplate(t.spec_yaml).namespace ?? 'a3t', t.name, t.id);
  }

  /// Controls → revoke every tool (keeping what it had), or give it back.
  async function revokeTools(revoke: boolean): Promise<string | null> {
    if (!agent) return null;
    const vars = parseJson(agent.variables ?? '') || {};
    if (revoke) {
      vars.toolsBeforeRevoke = { enabled_tools: vars.enabled_tools ?? null };
      vars.enabled_tools = [];
    } else {
      vars.enabled_tools = vars.toolsBeforeRevoke?.enabled_tools ?? null;
      delete vars.toolsBeforeRevoke;
    }
    const r = await updateAgent(agent, { variables: JSON.stringify(vars) });
    if (!r.ok) return r.error ?? 'Not changed';
    await loadAgents();
    return null;
  }
  /// Its model's label, "<name> (<provider>)", for what leaves the platform.
  const llmLabel = $derived(llmResources.find((o) => o.value === bindings.llm)?.label);

  /// Settings → move to a newer release of its template.
  async function upgradeTemplate(templateId: string, version: string): Promise<string | null> {
    if (!agent) return null;
    const r = await updateAgent(agent, { template_id: templateId, template_version: version });
    if (!r.ok) return r.error ?? 'It was not moved.';
    await loadAgents();
    return null;
  }

  function parseJson(s: string): any {
    try { return JSON.parse(s); } catch { return s; }
  }

  // ── Agent bindings (the "allowed to change" overrides) + read-only designer ──
  let agentTemplate = $derived(agent ? templates.find(t => t.id === agent.template_id) : null);

  // Pull the agent config block out of a template's spec (do[].*.with.config)
  function templateConfig(specYaml: string): any {
    try {
      const doc = YAML.parse(specYaml) ?? {};
      for (const step of (doc.do ?? [])) {
        const k = Object.keys(step)[0];
        const cfg = step?.[k]?.with?.config;
        if (cfg) return cfg;
      }
    } catch {}
    return {};
  }

  /// What Configure and Settings edit: its model, instructions, tools, memory,
  /// name and description (and, kept as they are, the settings the Studio owns).
  let bindings = $state<any>({ name: '', description: '', environment: '', mode: 'session', llm: '', target: '', ep_path: '', mcp_servers: [], system_prompt: '', max_iterations: 10, temperature: 0.7 });
  /// The saved bindings, to tell edits apart.
  let savedBindings = $state('');
  const bindDirty = $derived(!!savedBindings && JSON.stringify($state.snapshot(bindings)) !== savedBindings);
  let savingBind = $state(false);
  let saveBindMsg = $state('');
  const saveErr = $derived(saveBindMsg && saveBindMsg !== 'Saved' ? saveBindMsg : '');
  let _lastBindId = '';

  function initBindings() {
    if (!agent) return;
    const tpl = agentTemplate?.spec_yaml ? templateConfig(agentTemplate.spec_yaml) : {};
    const rb = parseJson(agent.resource_bindings) || {};
    const vars = parseJson(agent.variables) || {};
    let llm = rb.llm ?? tpl.llm_resource ?? '';
    if (llm && !String(llm).includes('/')) llm = `a3t/${llm}`;
    const mcpRaw = vars.mcp_servers ?? tpl.mcp_servers ?? [];
    const mcp = (Array.isArray(mcpRaw) ? mcpRaw : String(mcpRaw).split(',').map(s => s.trim()).filter(Boolean))
      .map((s: string) => serverRef(s, agent?.namespace || 'a3t'));   // older agents saved bare names
    bindings = {
      name: agent.name ?? '',
      description: vars.description ?? '',
      environment: vars.environment ?? '',
      mode: agent.mode ?? 'session',
      llm,
      system_prompt: vars.system_prompt ?? String(tpl.system_prompt ?? '').trim(),
      max_iterations: vars.max_iterations ?? tpl.max_iterations ?? 10,
      temperature: vars.temperature ?? tpl.temperature ?? 0.7,
      mcp_servers: mcp,
      target: vars.target ?? '',
      ep_path: vars.ep_path ?? '',
      // Capability ceiling: an allow-list of tool names this agent may invoke.
      // null = unrestricted (every tool its MCP servers expose); a list, even an
      // empty one, = only those tools — the runtime denies the rest.
      enabled_tools: Array.isArray(vars.enabled_tools) ? [...vars.enabled_tools] : null,
    };
    savedBindings = JSON.stringify(bindings);
    bindRev++;   // these are the saved bindings, for telling edits apart
  }
  let bindRev = $state(0);

  // The chat's context: this agent, the tab on screen, and its bindings as edited
  // (a new agent: what is filled in so far).
  trackActiveItem(() => agent ? {
    kind: 'agent', namespace: agent.namespace, name: agent.name || 'new agent',
    id: creatingNew ? '(new)' : agent.id, section: propNode,
    text: JSON.stringify(creatingNew ? newAgent : bindings, null, 2),
    rev: creatingNew ? 'new' : bindRev,
    // The app's Chat edits what is shown (propose_edit): the bindings, or the
    // new agent's fields.
    applyEdit: (t: string) => {
      let o: any;
      try { o = JSON.parse(t); } catch { return; }
      if (!o || typeof o !== 'object') return;
      if (creatingNew) newAgent = { ...newAgent, ...o };
      else bindings = { ...bindings, ...o };
    },
  } : null);

  // Re-init the form once per agent selection (after templates have loaded)
  $effect(() => {
    // Re-init the form when the selected agent changes, once its config actually
    // arrives (agents load async — the panel may open pre-selected before then),
    // and again once templates load. Tracking `agent` (not just `selected`) is
    // what makes it re-run after loadAgents() populates the config.
    const a = agent;
    // Key on the ACTUAL config content (not just its presence), so a re-init
    // fires whenever the binding data lands or changes — even if the async list
    // arrives after the panel is already open. The old `!!present` key could
    // latch empty if the string was present-but-updated. (Deliberately NOT keyed
    // on propNode, so tabbing between nodes never clobbers unsaved edits.)
    const key = a
      ? `${a.id}:${a.resource_bindings ?? ''}:${a.variables ?? ''}:${templates.length > 0}`
      : '';
    if (key && key !== _lastBindId) {
      _lastBindId = key;
      initBindings();
    }
  });

  async function saveBindings() {
    if (!agent) return;
    savingBind = true; saveBindMsg = '';
    const rb = parseJson(agent.resource_bindings) || {};
    rb.llm = bindings.llm;
    const vars = parseJson(agent.variables) || {};
    vars.system_prompt = bindings.system_prompt;
    vars.max_iterations = Number(bindings.max_iterations);
    vars.temperature = Number(bindings.temperature);
    vars.mcp_servers = bindings.mcp_servers;
    vars.target = bindings.target;
    vars.ep_path = bindings.ep_path;
    vars.enabled_tools = bindings.enabled_tools ?? null;
    const description = String(bindings.description ?? '').trim();
    if (description) vars.description = description; else delete vars.description;
    if (bindings.environment) vars.environment = bindings.environment; else delete vars.environment;
    const name = String(bindings.name ?? '').trim() || agent.name;
    const res = await execFlow('flow_a3t_agents-update', 'a3t', {
      id: agent.id, org_id: 'org-default', user_id: 'usr-superadmin', timestamp: new Date().toISOString(),
      data: {
        name, template_id: agent.template_id, template_version: agent.template_version,
        mode: bindings.mode || agent.mode, namespace: agent.namespace, status: agent.status,
        resource_bindings: JSON.stringify(rb), variables: JSON.stringify(vars),
      },
    });
    savingBind = false;
    if (res.ok) { saveBindMsg = 'Saved'; await loadAgents(); setTimeout(() => saveBindMsg = '', 2000); }
    else { saveBindMsg = res.error ?? 'Save failed'; }
  }

  // ── Agent → Canvas (deploy): pick a profile preset, clone it bound to this
  //    agent (two-way refs), review/edit in the designer, deploy. ──────────────
  let canvases = $state<any[]>([]);
  let canvasBusy = $state('');
  let canvasMsg = $state('');
  let pickedPreset = $state('');

  let presets = $derived(canvases.filter((c) => !(c.config?.flowRef)));
  let boundCanvas = $derived.by(() => {
    if (!agent) return null;
    const ref = parseJson(agent.variables)?.canvasRef;
    const byRef = ref ? canvases.find((c) => `a3t/${c.name}` === ref || c.name === String(ref).split('/').pop()) : null;
    return byRef ?? canvases.find((c) => c.config?.agentRef === `a3t/${agent.id}`) ?? null;
  });

  async function loadCanvases() {
    const res = await canvasApi.list();
    canvases = (Array.isArray(res.data) ? res.data : (res.data?.resources ?? [])).map((r: any) => ({ name: r.name, namespace: r.namespace, config: r.config ?? {} }));
  }

  function agentTargetFlow(): string {
    return bindings.target || parseJson(agent?.variables)?.target || 'a3t/tmpl_a3t_agent';
  }

  function openAgentCanvas(id: string) {
    selectAgent(id);
    propNode = 'deploy';
    pickedPreset = ''; canvasMsg = '';
    loadCanvases();
  }

  // Edit = clone the chosen preset into a canvas bound to this agent (agentRef),
  // and write canvasRef back on the agent (two-way). Stays in the Canvas tab.
  async function cloneCanvasForAgent(presetName: string) {
    if (!agent) return;
    const preset = canvases.find((c) => c.name === presetName);
    if (!preset) return;
    canvasBusy = 'cloning'; canvasMsg = '';
    const cname = `${agent.name}-canvas`.toLowerCase().replace(/[^a-z0-9-]/g, '-');
    const cfg = {
      ...structuredClone(preset.config),
      flowRef: agentTargetFlow(),
      agentRef: `a3t/${agent.id}`,
      description: `${agent.name} — from ${preset.name.replace('preset-', '')}`,
    };
    const exists = canvases.some((c) => c.name === cname);
    const res = exists ? await canvasApi.update('a3t', cname, cfg) : await canvasApi.create('a3t', cname, cfg);
    if (!res.ok) { canvasBusy = ''; canvasMsg = res.error || 'clone failed'; return; }
    await setAgentCanvasRef(`a3t/${cname}`);
    await loadCanvases();
    canvasBusy = '';
  }

  async function setAgentCanvasRef(ref: string) {
    if (!agent) return;
    const rb = parseJson(agent.resource_bindings) || {};
    const vars = parseJson(agent.variables) || {};
    if (ref) vars.canvasRef = ref; else delete vars.canvasRef;
    await execFlow('flow_a3t_agents-update', 'a3t', {
      id: agent.id, org_id: 'org-default', user_id: 'usr-superadmin', timestamp: new Date().toISOString(),
      data: {
        name: agent.name, template_id: agent.template_id, template_version: agent.template_version,
        mode: agent.mode, namespace: agent.namespace, status: agent.status,
        resource_bindings: JSON.stringify(rb), variables: JSON.stringify(vars),
      },
    });
    await loadAgents();
  }

  /// Deploy in one click: with its own canvas, made from the chosen profile the
  /// first time. Placement and scaling are refined in the Canvas designer.
  let deployRev = $state(0);
  /// What every tab reads about it, loaded once per agent (and after a deploy change).
  const live = $derived(agent && !creatingNew ? liveFor(agent, templates, deployRev) : null);
  async function deployAgent() {
    canvasMsg = '';
    if (!boundCanvas) {
      if (!pickedPreset) { canvasMsg = 'Choose how it should run.'; return; }
      await cloneCanvasForAgent(pickedPreset);
      if (!boundCanvas) { canvasMsg ||= 'Its canvas was not made.'; return; }
    }
    canvasBusy = 'deploying';
    const res = await canvasApi.deployResource('a3t', boundCanvas.name);
    canvasBusy = '';
    if (res.ok && res.data?.deployed) deployRev++;
    else canvasMsg = res.data?.error || res.error || 'It was not deployed.';
  }

  onMount(async () => {
    loadAgents();
    loadRegistry();
    loadCanvases();
    await loadTemplates();
    const panel = getActivePanel();
    if (panel?.props?.createFromTemplate) {
      // Deep-linked from a Template's "+ Create Agent": open the create pop-in
      // pre-selected to that template (one template → many agents).
      newAgentPanel();
      newAgent.template_id = panel.props.createFromTemplate;
      onTemplateChange();
    } else if (panel?.props?.create) {
      // The shell's "New agent" in Items.
      newAgentPanel();
    } else if (panel?.props?.selectedId) {
      selected = panel.props.selectedId;
      // "Open its memory", "Review its controls"… land on that tab.
      const tab = TAB_ALIAS[panel.props.tab] ?? panel.props.tab;
      if (tab && treeNodes.some((n) => n.id === tab)) propNode = tab;
      activityStart = panel.props.tab === 'memory' ? 'memory' : undefined;
    }
  });

  // Another item picked in Items: the page stays mounted and opens it in place,
  // from the list it already has — no reload, no flash of this page.
  let shownSel: string | undefined = untrack(() => getActivePanel()?.props?.selectedId);
  $effect(() => {
    const id = getActivePanel()?.props?.selectedId as string | undefined;
    untrack(() => {
      if (id === shownSel) return;
      shownSel = id;
      if (id) { selected = id; makeKind = null; branchForm = false; }
    });
  });
</script>

<!-- Slide Panel -->
<SlidePanel open={!!agent} onclose={closePanel} onsave={creatingNew ? createAgent : undefined} onopenineditor={() => agent && !creatingNew ? openInStudio('agent', agent.namespace ?? 'a3t', agent.name ?? agent.id, agent.id) : navigateTo('flow-editor', 'Studio')}>
  {#if agent}
    <div class="panel-layout">
      <PropertiesTree
        icon={agent.name.slice(0, 2).toUpperCase() || 'NEW'}
        color="#2980b9"
        name={agent.name || 'New Agent'}
        meta={creatingNew ? 'new agent' : `${agent.created_by && agent.created_by === $currentUser?.id ? 'yours' : agent.created_by ? `owner ${agent.created_by}` : 'unowned'} · ${agent.status}`}
        nodes={treeNodes}
        steps={agentSteps}
        wizard={creatingNew}
        activeNode={propNode}
        onselect={(id) => propNode = id}
      />
      <div class="panel-col">
      <div class="panel-content">
        {#if agent.isNew}
          <div class="pf">
            <WizardHead step={agentSteps.find((x) => x.id === propNode)} />
            <div class="pf-form">
              {#if propNode === 'identity'}
              <label class="cf-field"><span>Template</span>
                <select bind:value={newAgent.template_id} onchange={onTemplateChange}>
                  <option value="">— choose a template —</option>
                  {#each templateChoices as t (t.id)}<option value={t.id}>{t.name} v{t.version}{t.ns ? ` · ${t.ns}` : ''}</option>{/each}
                </select>
              </label>
              {#if newTplFacts.summary || newTplFacts.description}<p class="cf-about">{newTplFacts.summary || newTplFacts.description}</p>{/if}
              <label class="cf-field"><span>Name</span><input type="text" bind:value={newAgent.name} placeholder="my-agent" /></label>
              <label class="cf-field"><span>Project</span>
                <select bind:value={newAgent.namespace}>
                  {#each namespaceChoices(newAgent.namespace) as ns (ns)}<option value={ns}>{ns}</option>{/each}
                </select>
              </label>
              <p class="pf-hint">To start from an agent you already have, open it and clone or fork it.</p>
              {:else if propNode === 'behaviour'}
              <label class="cf-field"><span>Model</span>
                <ResourceSelect value={newAgent.llm} options={llmResources} placeholder="— choose a model —" onchange={(v) => newAgent.llm = v} />
              </label>
              <!-- Checkboxes, not a multi-select: a grey highlight read as "listed", not "selected". -->
              <div class="cf-field full"><span>Tools it may call</span>
                <div class="cf-checks">
                  {#each mcpResources as m}
                    <label class="cf-check"><input type="checkbox" value={m.value} bind:group={newAgent.mcp_servers} /> {m.label}</label>
                  {:else}
                    <span class="cf-none">No tool servers in this project yet</span>
                  {/each}
                </div>
              </div>
              <label class="cf-check cf-mem"><input type="checkbox" checked={newAgent.mode === 'persistent'} onchange={(e) => (newAgent.mode = (e.currentTarget as HTMLInputElement).checked ? 'persistent' : 'session')} />
                <span><b>Remembers across conversations</b><small>Off: it starts fresh each conversation.</small></span></label>
              {:else if propNode === 'serving'}
              <label class="cf-field"><span>Endpoint</span>
                <ResourceSelect value={newAgent.ep_path} options={triggerOptions} placeholder="A new one, made for it" allowNew={false} onchange={(v) => newAgent.ep_path = v} />
              </label>
              <p class="pf-hint">Callers send messages here. Leave it as is and it gets its own: <span class="mono">POST /webhook/agent/&lt;its id&gt;/chat</span>.</p>
              <label class="cf-field"><span>Served by</span>
                <ResourceSelect value={newAgent.target} options={flowOptions} placeholder="— choose a flow —" onchange={(v) => newAgent.target = v} />
              </label>
              <p class="pf-hint">The flow that runs each conversation — filled in from the template. Change it only if you know why.</p>
              {:else}
              <dl class="cf-review">
                <dt>Template</dt><dd>{templateChoices.find((t) => t.id === newAgent.template_id)?.name ?? '—'} v{newAgent.template_version}</dd>
                <dt>Name</dt><dd class="mono">{newAgent.name || '—'}</dd>
                <dt>Project</dt><dd class="mono">{newAgent.namespace}</dd>
                <dt>Model</dt><dd class="mono">{newAgent.llm || '—'}</dd>
                <dt>Tools</dt><dd>{newAgent.mcp_servers.length ? newAgent.mcp_servers.map((m) => m.split('/').pop()).join(', ') : 'none'}</dd>
                <dt>Memory</dt><dd>{newAgent.mode === 'persistent' ? 'remembers across conversations' : 'starts fresh each conversation'}</dd>
                <dt>Endpoint</dt><dd class="mono">{newAgent.ep_path || 'a new one, made for it'}</dd>
              </dl>
              <p class="pf-hint">It is created as a draft: it answers on its endpoint through the shared runtime. Deploy it from its Deploy tab.</p>
              {/if}
            </div>
          </div>
        {:else}
          {@const studio = () => openInStudio('agent', agent.namespace ?? 'a3t', agent.name ?? agent.id, agent.id)}
          <div class="pf wide">
            <WizardHead step={agentSteps.find((x) => x.id === propNode)} />
            {#if propNode === 'overview'}
              <AgentOverview {agent} {templates} {allAgents} live={live!} me={$currentUser?.id ?? ''} {llmLabel}
                ongo={(t) => (propNode = t)} onstudio={studio} oncode={openCode} onlifecycle={lifecycle} />
            {:else if propNode === 'hooks'}
              <AgentHooks {agent} {templates} />
            {:else if propNode === 'lineage'}
              <AgentLineage {agent} {allAgents} onopen={selectAgent} onmake={lifecycle} />
            {:else if propNode === 'sharing'}
              <AgentSharing {agent} canEdit={$canEditAgent} />
            {:else if propNode === 'access'}
              <AgentAccess {agent} {templates} live={live!} {bindings} {llmLabel} canEdit={$canEditAgent} ongo={(t) => (propNode = t)}>
                {#snippet editor()}
                  <AgentConfig {agent} {templates} bind:bindings dirty={bindDirty} saving={savingBind} saveError={saveErr}
                    onsave={saveBindings} ondiscard={discardBindings} oncode={openCode} {llmResources} {mcpResources} {flowOptions} />
                {/snippet}
              </AgentAccess>
            {:else if propNode === 'exposure'}
              <AgentExposure {agent} {templates} live={live!} ongo={(t) => (propNode = t)} />
            {:else if propNode === 'controls'}
              <AgentControls {agent} {templates} live={live!} canAdmin={$canEditAgent} onrevoke={revokeTools} onchanged={() => deployRev++} />
            {:else if propNode === 'releases'}
              <AgentReleases {agent} {templates} {allAgents} live={live!} me={$currentUser?.email ?? $currentUser?.id ?? ''} bind:branchForm
                onstudio={studio} onopen={selectAgent} onchanged={() => deployRev++}>
                {#snippet notDeployed()}
                  {#if boundCanvas}
                    <div class="dep-row">
                      <button class="btn2 primary" disabled={!!canvasBusy} onclick={deployAgent}>{canvasBusy === 'deploying' ? 'Deploying…' : 'Deploy'}</button>
                      <span class="dim">with <span class="mono">{boundCanvas.name}</span></span>
                      <button class="link" onclick={() => navigateTo('canvas', 'Canvas', { editCanvas: boundCanvas.name })}>Change how it runs →</button>
                    </div>
                  {:else if presets.length}
                    <div class="dep-row">
                      <select class="dep-pick" bind:value={pickedPreset} aria-label="How it should run">
                        <option value="">— how should it run? —</option>
                        {#each presets as pr (pr.name)}<option value={pr.name}>{pr.name.replace('preset-', '')}{pr.config.description ? ` — ${pr.config.description}` : ''}</option>{/each}
                      </select>
                      <button class="btn2 primary" disabled={!pickedPreset || !!canvasBusy} onclick={deployAgent}>{canvasBusy === 'cloning' ? 'Preparing…' : canvasBusy === 'deploying' ? 'Deploying…' : 'Deploy'}</button>
                    </div>
                  {:else}
                    <p class="dim">No deployment profiles yet — an admin adds them on the Canvas page.</p>
                  {/if}
                  {#if canvasMsg}<p class="err">{canvasMsg}</p>{/if}
                {/snippet}
              </AgentReleases>
            {:else if propNode === 'audit'}
              <AgentAudit {agent} {templates} live={live!} start={activityStart} onstudio={studio} ongo={(t) => (propNode = t)} />
            {:else if propNode === 'settings'}
              <AgentSettings {agent} {templates} {allAgents} live={live!} bind:bindings dirty={bindDirty} saving={savingBind} saveError={saveErr}
                onsave={saveBindings} ondiscard={discardBindings} ongo={(t) => (propNode = t)} onopen={selectAgent}
                onupgrade={upgradeTemplate} ondelete={deleteCurrent} />
            {/if}
          </div>
        {/if}
      </div>
      {#if makeKind && !creatingNew}
        <AgentMakeFrom kind={makeKind} name={agent.name} projects={namespaceChoices(agent.namespace)} current={agent.namespace}
          onmake={makeFrom} oncancel={() => (makeKind = null)} />
      {/if}
      <WizardNav steps={agentSteps} active={propNode} wizard={creatingNew} onselect={(id) => (propNode = id)}
        final={creatingNew ? createAgent : undefined}
        finalLabel={creatingNew ? 'Create agent' : 'Save'} finalBusy={creatingNew ? creating : savingBind}
        busyLabel={creatingNew ? 'Creating…' : 'Saving…'} />
      </div>
    </div>
  {/if}
</SlidePanel>

<!-- Delete Confirmation -->
<DeleteConfirm
  open={!!deleteTarget}
  entityType="Agent"
  entityName={deleteTarget?.name ?? ''}
  onconfirm={deleteAgent}
  oncancel={() => deleteTarget = null}
/>

<!-- Page Header -->
<div class="screen-header">
  <div class="breadcrumb">Agents <span class="sep">/</span> All</div>
  <div class="screen-title-row">
    <div class="title-block">
      <SubmenuToggle appType="agents" />
      <h1 class="screen-title">Agents</h1>
    </div>
    <div class="screen-actions">
      {#if $canCreate}<NewButton label="New agent" accent="runtime" onclick={newAgentPanel} />{/if}
    </div>
  </div>
</div>

<!-- Page Body -->
<div class="screen-body">
  <PageBanner
    headline="Configure an agent."
    highlightWord="agent"
    subtitle="Bind a template to resources. Set mode, memory, and tools."
    description="An agent is a template with frozen inputs — which LLM to use, which memory store, which MCP servers. Configure once, deploy to any Canvas."
    accent="runtime"
    docsPath="/agents"
  />
  <!-- Pinned and latest, under the banner: the section page leads with what
       it is for, then the items you actually return to. -->
  <AppHome appType="agents" embedded />


  <!-- New agents now open in the edit slide-in panel (see newAgentPanel). -->

  {#snippet agentCards(list: any[])}
    <div class="card-grid">
      {#each list as a (a.id)}
        <EntityCard
          icon={a.name.slice(0, 2).toUpperCase()}
          color="#1a7158"
          name={a.name}
          description="from {a.template_id ?? 'no template'}"
          tags={[a.mode, a.status]}
          meta="v{a.template_version}"
          project={a.project}
          actionLabel="Deploy to Canvas"
          onclick={() => selectAgent(a.id)}
          onaction={() => openAgentCanvas(a.id)}
          ondelete={() => deleteTarget = { id: a.id, name: a.name }}
        />
      {/each}
    </div>
  {/snippet}

  <FoldSection title="My Agents" count={myAgents.length} hint="Created by you">
    {@render agentCards(myAgents)}
  </FoldSection>
  <FoldSection title="Team Agents" count={sharedAgents.length} hint="Created by others, in projects you can see">
    {@render agentCards(sharedAgents)}
  </FoldSection>
  <ExposedSections kind="agent" noun="Agents" />
</div>

<style>
  .screen-header { padding: 18px 32px 12px; border-bottom: 1px solid var(--border); background: var(--bg-page); }
  .breadcrumb { font-size: 12px; color: var(--text-3); margin-bottom: 8px; font-family: var(--font-mono); }
  .breadcrumb .sep { color: var(--text-5); }
  .screen-title-row { display: flex; align-items: center; gap: 18px; }
  .title-block { flex: 1; display: flex; align-items: center; gap: 10px; }
  .screen-title { font-family: var(--font-serif); font-size: 24px; font-weight: 500; letter-spacing: -0.02em; }
  .screen-actions { display: flex; gap: 8px; }
  .screen-body { flex: 1; overflow-y: auto; padding: 20px 32px 40px; }

  .card-grid { display: grid; grid-template-columns: repeat(auto-fill, minmax(300px, 1fr)); gap: 12px; }
  /* Panel layout */
  .panel-layout { display: grid; grid-template-columns: 280px 1fr; height: 100%; }
  .panel-col { position: relative; display: flex; flex-direction: column; min-height: 0; height: 100%; }
  .panel-content { overflow-y: auto; flex: 1; min-height: 0; }
  .pf-form { display: flex; flex-direction: column; gap: 12px; margin-top: 14px; }
  .cf-checks { display: flex; flex-direction: column; gap: 4px; padding: 6px 8px; border: 1px solid var(--border); border-radius: var(--r-sm); background: var(--bg-surface); }
  .cf-check { display: flex; align-items: center; gap: 10px; padding: 3px 0; cursor: pointer; }
  .cf-none { font-size: 12px; color: var(--text-3, var(--text-2)); }
  .cf-about { margin: -4px 0 0; font-size: 13px; color: var(--text-2); line-height: 1.5; }
  .cf-mem { align-items: flex-start; text-transform: none; letter-spacing: 0; font-weight: 400; }
  .cf-mem span { display: flex; flex-direction: column; gap: 2px; font-size: 13px; }
  .cf-mem small { color: var(--text-3); font-size: 12px; }
  .cf-review { display: grid; grid-template-columns: 110px 1fr; gap: 9px 14px; margin: 0; font-size: 13px; }
  .cf-review dt { color: var(--text-3); }
  .cf-review dd { margin: 0; color: var(--text-1); }
  .dep-row { display: flex; flex-wrap: wrap; align-items: flex-end; gap: 10px; font-size: 13px; color: var(--text-2); }
  .dep-pick { flex: 1 1 260px; width: auto; }
  .mono { font-family: var(--font-mono); }

  /* Panel forms */
  .pf { padding: 24px 32px; max-width: 704px; }
  .pf.wide { max-width: 980px; }
  .pf-hint { font-size: 12px; color: var(--text-4); margin-bottom: 12px; }


</style>
