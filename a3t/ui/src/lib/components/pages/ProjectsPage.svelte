<script lang="ts">
  import DetailActionsBar from '$lib/shell/DetailActionsBar.svelte';
  import { trackActiveItem } from '$lib/stores/activeItem.svelte.ts';
  import WizardSteps, { type WizardStep } from '$lib/components/shared/WizardSteps.svelte';
  import WizardNav from '$lib/components/shared/WizardNav.svelte';
  import WizardHead from '$lib/components/shared/WizardHead.svelte';
  /// Projects: namespaces owned by you (<handle>-<name>) or by an org
  /// (<org>-<name>), who is in each, and the rules it runs under. Everything goes
  /// through the a3t-projects / a3t-profile flows, which decide on the server who
  /// may see and change what; this page only asks. Namespaces from before
  /// projects (such as `default`) have no owner and stay open to everyone.
  import SubmenuToggle from '$lib/components/shared/SubmenuToggle.svelte';
  import NewButton from '$lib/components/shared/NewButton.svelte';
  import PageBanner from '$lib/components/shared/PageBanner.svelte';
  import FoldSection from '$lib/components/shared/FoldSection.svelte';
  import EntityCard from '$lib/components/shared/EntityCard.svelte';
  import SlidePanel from '$lib/components/shared/SlidePanel.svelte';
  import AppHome from '$lib/shell/AppHome.svelte';
  import { onMount, untrack } from 'svelte';
  import { getActivePanel, navigateToDetail, navigateTo } from '$lib/stores/panels.svelte.ts';
  import { currentUser } from '$lib/stores/auth';
  import {
    getProjects, getShowPlatform, loadProjects, setProject, roleIn, isMine, getProfile, getMyOrgs, type Project,
  } from '$lib/stores/context.svelte.ts';
  import {
    projectsApi, profileApi, loadDirectory, releasesApi, mcpCall, canvasApi, executions, opsApi,
    type ProjectMember, type ProjectRole, type DirectoryUser,
  } from '$lib/services/api';
  import { categorize } from '$lib/core/resources/categories';

  const ROLES: ProjectRole[] = ['owner', 'developer', 'operator', 'viewer'];
  const ROLE_HINT: Record<ProjectRole, string> = {
    owner: 'Everything, including members and rules',
    developer: 'Build and run flows, agents and resources',
    operator: 'Deploy and run; no authoring',
    viewer: 'Read only',
  };
  /// Rules a project adds on top of the org baseline. Saved with the project;
  /// enforced once project-scoped Cedar is on.
  const POLICY_PRESETS: { key: string; label: string; hint: string }[] = [
    { key: 'prod_deploy_owner_only', label: 'Only owners deploy to prod', hint: 'Developers and operators deploy to dev and staging.' },
    { key: 'destructive_owner_only', label: 'Destructive tools need an owner', hint: 'Deletes and drops run only when an owner asks.' },
    { key: 'agent_memory_per_user', label: 'Agents remember per user', hint: "An agent's memory of one member isn't visible to another." },
  ];
  const ENVIRONMENTS = ['dev', 'staging', 'prod'];
  const SLUG = /^[a-z0-9]([a-z0-9-]{0,36}[a-z0-9])?$/;
  const HANDLE = /^[a-z0-9]([a-z0-9-]{0,22}[a-z0-9])?$/;

  const visible = $derived(getProjects().filter((p) => !p.platform || getShowPlatform()));
  const mine = $derived(visible.filter((p) => isMine(p)));
  const shared = $derived(visible.filter((p) => !isMine(p)));
  const profile = $derived(getProfile());
  const orgs = $derived(getMyOrgs());

  // ── Panel state ──
  let panel = $state<'closed' | 'create' | 'detail'>('closed');
  /// Detail: what it is (Overview), what is in it (Contents), what runs where and
  /// each environment's variables (Environments), whether it is healthy (Health), who may do what (Access &
  /// policy), and its setup (Settings). Create: Settings → Access & policy → Review.
  type Tab = 'overview' | 'contents' | 'environments' | 'health' | 'access' | 'settings' | 'review';
  let tab = $state<Tab>('overview');
  // Releases: each flow in the project, its versions and the active one.
  let releases = $state<{ name: string; versions: string[]; active: string }[]>([]);
  let releasesLoading = $state(false);
  async function loadReleases(ns: string) {
    releasesLoading = true;
    const r = await mcpCall('list_flows', { namespace: ns });
    const flows: any[] = r.ok ? (r.data?.workflows ?? []) : [];
    const out = [];
    for (const f of flows) {
      const v = await releasesApi.versions(ns, f.name);
      out.push({ name: f.name, versions: v.ok ? (v.data?.versions ?? []) : [f.version], active: v.ok ? v.data?.active : f.version });
    }
    releases = out;
    releasesLoading = false;
  }
  async function activate(name: string, version: string) {
    if (!info) return;
    const r = await releasesApi.activate(info.namespace, name, version);
    say(r.ok ? `${name}: callers without a pin now get ${version}.` : (r.error ?? 'Could not activate'), r.ok);
    await loadReleases(info.namespace);
  }
  // Contents shows each flow's releases, with the active one switchable.
  $effect(() => { if (tab === 'contents' && panel === 'detail' && info) void loadReleases(info.namespace); });


  // ── What the project holds and how it is doing (Overview, Contents, Health) ──
  type Row = { name: string; id?: string; type?: string; status?: string; sub?: string };
  type Insight = {
    ns: string;
    agents: Row[]; flows: Row[]; templates: Row[]; resources: Row[]; triggers: Row[]; canvases: Row[];
    runs: any[]; deployments: any[];
  };
  let insight = $state<Insight | null>(null);
  let insightFor = '';
  const listOf = (d: any, ...keys: string[]): any[] => {
    if (Array.isArray(d)) return d;
    for (const k of keys) if (Array.isArray(d?.[k])) return d[k];
    return [];
  };
  async function loadInsight(ns: string) {
    insightFor = ns;
    const [fl, rs, tr, ag, tp, cv, rn, ops] = await Promise.all([
      mcpCall('list_flows', { namespace: ns }), mcpCall('list_resources', { namespace: ns }), mcpCall('list_triggers', {}),
      mcpCall('list_agents', {}), mcpCall('list_agent_templates', {}), canvasApi.list(),
      executions.list({ namespace: ns, limit: 200 }), opsApi.overview(),
    ]);
    if (insightFor !== ns) return;
    const inNs = (r: any) => (r?.namespace ?? '') === ns;
    insight = {
      ns,
      flows: listOf(fl.ok ? fl.data : null, 'workflows', 'flows').map((f) => ({ name: f.name, sub: f.version ? `v${f.version}` : '', status: f.enabled === false ? 'disabled' : undefined })),
      resources: listOf(rs.ok ? rs.data : null, 'resources').filter((r) => r.type !== 'project').map((r) => ({ name: r.name, type: r.type ?? r.type_ref ?? '', sub: r.type ?? r.type_ref ?? '' })),
      triggers: listOf(tr.ok ? tr.data : null, 'triggers').filter(inNs).map((t) => ({ name: t.name ?? t.id, id: t.id, sub: t.config?.type ?? '', status: t.enabled === false ? 'paused' : undefined })),
      agents: listOf(ag.ok ? ag.data : null, 'agents').filter(inNs).map((a) => ({ name: a.name ?? a.id, id: a.id, sub: a.mode ?? '', status: a.status })),
      templates: listOf(tp.ok ? tp.data : null, 'templates').filter(inNs).map((t) => ({ name: t.name, id: t.id, sub: t.version ? `v${t.version}` : '' })),
      canvases: listOf(cv.ok ? cv.data : null, 'canvases', 'resources').filter(inNs).map((c) => ({ name: c.name, sub: c.config?.flowRef ?? '' })),
      runs: listOf(rn.ok ? rn.data : null, 'executions', 'runs'),
      deployments: ((ops.ok ? ops.data?.runtimes : null) ?? []).flatMap((r: any) => r.deployments ?? [])
        .filter((d: any) => d.deployment?.namespace === ns || String(d.deployment?.target ?? '').startsWith(`${ns}/`)),
    };
  }
  $effect(() => {
    const ns = panel === 'detail' ? info?.namespace : undefined;
    untrack(() => { if (ns && ns !== insightFor) { insight = null; void loadInsight(ns); } });
  });
  const DAY = 24 * 3600e3;
  const recentRuns = $derived((insight?.runs ?? []).filter((r) => Date.now() - Date.parse(r.started_at ?? r.created_at ?? 0) < DAY));
  const failedRecent = $derived(recentRuns.filter((r) => /fail|error/i.test(String(r.status ?? ''))));
  /// The server deletes a project only when it is empty: what is still in it.
  const blockers = $derived(insight ? [
    insight.flows.length && `${insight.flows.length} flow${insight.flows.length > 1 ? 's' : ''}`,
    insight.resources.length && `${insight.resources.length} resource${insight.resources.length > 1 ? 's' : ''}`,
    insight.triggers.length && `${insight.triggers.length} trigger${insight.triggers.length > 1 ? 's' : ''}`,
  ].filter(Boolean) as string[] : []);
  const when = (t: string | undefined) => {
    if (!t) return '';
    const s = Math.round((Date.now() - Date.parse(t)) / 1000);
    return s < 60 ? 'just now' : s < 3600 ? `${Math.round(s / 60)} min ago` : s < 86400 ? `${Math.round(s / 3600)} h ago` : new Date(t).toLocaleDateString();
  };
  /// Open an item of the project in its own app.
  function openItem(kind: 'agent' | 'flow' | 'template' | 'resource' | 'trigger' | 'canvas', r: Row) {
    const ns = info?.namespace ?? '';
    const [type, title, id] = kind === 'agent' ? ['agents', 'Agents', r.name]
      : kind === 'flow' ? ['resources', 'Resources', `flows:${ns}/${r.name}`]
      : kind === 'template' ? ['templates', 'Templates', r.name]
      : kind === 'resource' ? ['resources', 'Resources', `${categorize(r.type ?? '')}:${ns}/${r.name}`]
      : kind === 'trigger' ? ['triggers', 'Triggers', r.id ?? r.name]
      : ['canvas', 'Canvas', r.name];
    navigateToDetail(type, title, id);
  }
  let selectedName = $state('');
  let message = $state('');
  let messageOk = $state(false);
  let saving = $state(false);
  // Create
  let owner = $state('');            // "user:<handle>" or "org:<handle>"
  let newName = $state('');
  let newDisplay = $state('');
  let newDesc = $state('');
  let newPrivate = $state(false);
  let handleDraft = $state('');
  // Detail drafts
  let dName = $state('');
  let dDesc = $state('');
  let dPrivate = $state(false);
  let dMembers = $state<ProjectMember[]>([]);
  let dPolicy = $state<Record<string, any>>({});
  // Members
  let directory = $state<DirectoryUser[]>([]);
  let addWho = $state('');
  let addRole = $state<ProjectRole>('developer');
  // Move
  let moveOrg = $state('');
  let moveReport = $state<any>(null);

  const selected = $derived(getProjects().find((p) => p.name === selectedName));
  const info = $derived(selected?.info);
  const ownerHandle = $derived(owner.split(':')[1] ?? '');
  const previewNs = $derived(ownerHandle && newName ? `${ownerHandle}-${newName}` : '');
  /// The project the tabs edit: the one open, or — while creating — the one the
  /// form describes, so every tab works before it exists.
  const view = $derived<any>(panel === 'create'
    ? { owner_kind: owner.startsWith('org:') ? 'org' : 'user', owner: owner.split(':')[1] ?? '', namespace: previewNs || '<owner>-<name>', role: 'owner', environments: [] }
    : info);
  /// Exposure: the system org exposes to a3t only; the platform orgs label it
  /// shared or training, every other org's is community.
  const isSystem = $derived(view?.owner_kind === 'org' && view?.owner === 'system');
  const isPlatformOrg = $derived(view?.owner_kind === 'org' && (view?.owner === 'a3t' || isSystem));
  const ex = $derived<any>(dPolicy.exposure ?? { audience: 'none' });
  /// The same rule the server applies to changing and deleting a project.
  const canEdit = $derived(panel === 'create' || (!!info && (info.can_admin || info.role === 'owner')));
  const adminOrgs = $derived(orgs.filter((o) => o.role === 'admin'));
  /// Orgs this project can move to: those you administer, other than its own.
  const targetOrgs = $derived(adminOrgs.filter((o) => !(info?.owner_kind === 'org' && o.handle === info?.owner)));

  // ── Environments: what runs where, and each environment's variables ──
  // SAMPLE until the server has them (UI limitations, SRV-5…7): the project
  // record does not store variables, a canvas does not pass them to its pods,
  // and nothing yet says which version runs in which environment. The tab says
  // so; nothing here is presented as live.
  const envNames = $derived<string[]>(info?.environments?.length ? info.environments : ENVIRONMENTS);
  type EnvVar = { name: string; secret: boolean; values: Record<string, string> };
  let envVars = $state<EnvVar[]>([]);
  let envVarsFor = '';
  function sampleVars(ns: string): EnvVar[] {
    return [
      { name: 'LOG_LEVEL', secret: false, values: { dev: 'debug', staging: 'info', prod: 'warn' } },
      { name: 'LLM_MODEL', secret: false, values: { dev: 'claude-haiku-5-5', staging: 'claude-sonnet-5-5', prod: 'claude-sonnet-5-5' } },
      { name: 'ORDERS_DB', secret: false, values: { dev: `${ns}/orders-db-dev`, staging: `${ns}/orders-db-staging`, prod: `${ns}/orders-db` } },
      { name: 'PAGER_TOKEN', secret: true, values: { dev: `${ns}/pager-dev`, staging: '', prod: `${ns}/pager-prod` } },
    ];
  }
  $effect(() => {
    const ns = panel === 'detail' ? info?.namespace : undefined;
    untrack(() => { if (ns && ns !== envVarsFor) { envVarsFor = ns; envVars = sampleVars(ns); } });
  });
  function addVar() { envVars = [...envVars, { name: '', secret: false, values: {} }]; }
  function removeVar(i: number) { envVars = envVars.filter((_, j) => j !== i); }
  const missingValues = $derived(envVars.reduce((n, v) => n + envNames.filter((e) => v.name.trim() && !(v.values[e] ?? '').trim()).length, 0));
  /// Which version of each flow and agent runs in each environment (sample).
  const whereRuns = $derived.by(() => {
    if (!insight) return [];
    const rows = [
      ...insight.flows.map((f) => ({ kind: 'flow', name: f.name, latest: releases.find((r) => r.name === f.name)?.active ?? f.sub?.replace(/^v/, '') ?? '' })),
      ...insight.agents.map((a) => ({ kind: 'agent', name: a.name, latest: '1.0.0' })),
    ];
    return rows.map((r, i) => {
      const at: Record<string, string> = {};
      envNames.forEach((e, k) => {
        // dev runs the latest, later environments a step behind; the last row
        // has not reached the last environment.
        if (i === rows.length - 1 && k === envNames.length - 1 && rows.length > 1) return;
        at[e] = k === envNames.length - 1 && r.latest ? previous(r.latest) : r.latest;
      });
      return { ...r, at };
    });
  });
  function previous(v: string) {
    const m = v.match(/^(\d+)\.(\d+)\.(\d+)$/);
    if (!m) return v;
    return Number(m[2]) > 0 ? `${m[1]}.${Number(m[2]) - 1}.0` : Number(m[1]) > 1 ? `${Number(m[1]) - 1}.0.0` : v;
  }

  function say(text: string, ok = false) { message = text; messageOk = ok; }

  /// The pop-in's steps. A new project goes in order: it needs an owner and a
  /// valid name; members and policy can be left as they are; it has no releases yet.
  const pjSteps = $derived.by((): WizardStep[] => {
    const nameOk = !!owner && SLUG.test(newName.trim());
    if (panel === 'create') return [
      { id: 'settings', icon: ICON.settings, label: 'Project', hint: 'Owner, name and who can see it', ok: nameOk,
        need: !owner ? (profile ? 'Choose an owner.' : 'Claim your handle first.') : 'Name the project: 2–38 lowercase letters, digits and dashes.' },
      { id: 'access', icon: ICON.access, label: `Access & policy`, hint: 'Members, where it runs, who else may use it, its rules', optional: true },
      { id: 'review', icon: ICON.review, label: 'Review', hint: 'What will be created' },
    ];
    return [
      { id: 'overview', icon: ICON.overview, label: 'Overview', hint: 'What it is and how it is doing' },
      { id: 'contents', icon: ICON.contents, label: `Contents${insight ? ` (${insight.agents.length + insight.flows.length + insight.templates.length + insight.resources.length + insight.triggers.length + insight.canvases.length})` : ''}`, hint: 'Agents, flows, templates, resources, triggers and canvases in it' },
      { id: 'environments', icon: ICON.environments, label: 'Environments', hint: 'What runs where, and the variables each environment gives its deployments' },
      { id: 'health', icon: ICON.health, label: 'Health', hint: "Its deployments and recent failures" },
      { id: 'access', icon: ICON.access, label: `Access & policy`, hint: 'Members, where it runs, who else may use it, its rules' },
      { id: 'settings', icon: ICON.settings, label: 'Settings', hint: 'Name, visibility, environments, ownership' },
    ];
  });
  const ICON = {
    overview: '<rect x="2" y="2" width="12" height="12" rx="2"/><path d="M5 6h6M5 9h4"/>',
    contents: '<path d="M2 4h12M2 8h12M2 12h8"/>',
    environments: '<path d="M8 1L1 5l7 4 7-4z"/><path d="M1 8l7 4 7-4"/><path d="M1 11l7 4 7-4"/>',
    health: '<path d="M1 8h3l2-4 3 8 2-4h4"/>',
    access: '<path d="M8 1L2 4v4c0 4 3 6.5 6 7.5 3-1 6-3.5 6-7.5V4z"/>',
    settings: '<circle cx="8" cy="8" r="2.5"/><path d="M8 1v2M8 13v2M1 8h2M13 8h2M3 3l1.5 1.5M11.5 11.5L13 13M3 13l1.5-1.5M11.5 4.5L13 3"/>',
    review: '<path d="M3 8l3 3 7-7"/>',
  };

  function card(p: Project) {
    const role = roleIn(p);
    return {
      name: p.info?.display_name || p.name,
      description: p.info?.description || (p.platform ? 'Platform namespace' : p.info ? `${p.info.owner_kind === 'org' ? 'Org' : 'Personal'} project of ${p.info.owner}` : 'From before projects — open to everyone'),
      tags: [role ?? (p.platform ? 'platform' : 'open'), ...(p.info?.private ? ['private'] : [])],
      meta: `${p.flows ?? 0} flows · ${p.resources ?? 0} resources · ${p.triggers ?? 0} triggers`,
    };
  }

  function openDetail(name: string) { navigateToDetail('projects', 'Projects', name); }

  function showDetail(name: string) {
    const p = getProjects().find((x) => x.name === name);
    if (!p) return;
    selectedName = name;
    dName = p.info?.display_name ?? '';
    dDesc = p.info?.description ?? '';
    dPrivate = !!p.info?.private;
    dMembers = structuredClone($state.snapshot(p.info?.members ?? [])) as ProjectMember[];
    dPolicy = { ...(p.info?.policy ?? {}) };
    moveOrg = ''; moveReport = null;
    say('');
    tab = 'overview';
    panel = 'detail';
    formRev++;   // the form now holds the saved project
  }
  let formRev = $state(0);

  // The chat's context: the project open (or being created), its tab, and its
  // settings as edited.
  trackActiveItem(() => panel === 'detail' && selectedName ? {
    kind: 'project', name: selectedName, section: tab,
    text: JSON.stringify({ display_name: dName, description: dDesc, private: dPrivate, members: dMembers, policy: dPolicy }, null, 2),
    rev: formRev,
  } : panel === 'create' ? {
    kind: 'project', name: newName || 'new project', id: '(new)', section: tab,
    text: JSON.stringify({ owner, name: newName, display_name: newDisplay, description: newDesc, private: newPrivate, members: dMembers }, null, 2),
    rev: 'new',
  } : null);

  function openCreate() {
    owner = profile ? `user:${profile.handle}` : (orgs[0] ? `org:${orgs[0].handle}` : '');
    newName = ''; newDisplay = ''; newDesc = ''; newPrivate = false;
    dMembers = []; dPolicy = {}; tab = 'settings';
    handleDraft = profile ? '' : (($currentUser?.email ?? '').split('@')[0] ?? '').toLowerCase().replace(/[^a-z0-9-]+/g, '-').replace(/^-+|-+$/g, '').slice(0, 24);
    say('');
    panel = 'create';
  }

  function closePanel() {
    panel = 'closed';
    if (getActivePanel()?.props?.selectedId || getActivePanel()?.props?.create) navigateTo('projects', 'Projects');
  }

  // The URL drives the panel: /projects/<name> opens it; the switcher's
  // "+ New project" arrives with { create: true }.
  $effect(() => {
    const props = getActivePanel()?.props ?? {};
    const id = props.selectedId as string | undefined;
    const create = !!props.create;
    const n = getProjects().length;
    untrack(() => {
      if (create && panel !== 'create') openCreate();
      else if (id && n && (panel !== 'detail' || selectedName !== id)) showDetail(id);
      else if (!id && !create && panel !== 'closed') panel = 'closed';
    });
  });

  async function claimHandle() {
    const h = handleDraft.trim();
    if (!HANDLE.test(h)) { say('A handle is 2–24 lowercase letters, digits and dashes.'); return; }
    saving = true;
    const res = await profileApi.claim(h, $currentUser?.email, $currentUser?.name);
    saving = false;
    if (!res.ok) { say(res.error ?? 'Could not claim the handle'); return; }
    await loadProjects(true);
    owner = `user:${h}`;
    say(`Your handle is @${h}.`, true);
  }

  async function create() {
    const [kind, handle] = owner.split(':') as ['user' | 'org', string];
    if (!kind || !handle) { say(profile ? 'Choose an owner.' : 'Choose your handle first.'); return; }
    if (!SLUG.test(newName.trim())) { say('A project name is 2–38 lowercase letters, digits and dashes.'); return; }
    saving = true; say('');
    const res = await projectsApi.create({
      owner_kind: kind, owner: handle, name: newName.trim(),
      display_name: newDisplay.trim() || newName.trim(), description: newDesc.trim(), private: newPrivate,
    });
    if (!res.ok) { saving = false; say(res.error ?? 'Create failed'); return; }
    const ns = res.data?.project?.namespace ?? `${handle}-${newName.trim()}`;
    // Members and policy set in the other tabs go in with it (one step).
    if (dMembers.length || Object.keys(dPolicy).length) {
      const up = await projectsApi.update(ns, {
        display_name: newDisplay.trim() || newName.trim(), description: newDesc.trim(), private: newPrivate,
        members: $state.snapshot(dMembers) as ProjectMember[], policy: { ...dPolicy },
      });
      if (!up.ok) say(`Created ${ns}, but its members and policy were not saved: ${up.error ?? 'update failed'}`);
    }
    saving = false;
    await loadProjects(true);
    setProject(ns);
    navigateToDetail('projects', 'Projects', ns);
  }

  async function save() {
    if (!info || !canEdit) return;
    saving = true; say('');
    const res = await projectsApi.update(info.namespace, {
      display_name: dName.trim() || info.name, description: dDesc, private: dPrivate,
      members: $state.snapshot(dMembers) as ProjectMember[], policy: { ...dPolicy },
    });
    saving = false;
    if (!res.ok) { say(res.error ?? 'Save failed'); return; }
    await loadProjects(true);
    formRev++;
    say('Saved.', true);
  }

  async function remove() {
    if (!info || !canEdit) return;
    if (!confirm(`Delete project “${info.namespace}”? Its members and rules are removed.`)) return;
    const res = await projectsApi.remove(info.namespace);
    if (!res.ok) { say(res.error ?? 'Delete failed'); return; }
    await loadProjects(true);
    setProject('');
    closePanel();
  }

  async function previewMove() {
    if (!info || !moveOrg) return;
    saving = true; say(''); moveReport = null;
    const res = await projectsApi.move(info.namespace, moveOrg, true);
    saving = false;
    if (!res.ok) { say(res.error ?? 'Preview failed'); return; }
    moveReport = res.data?.report ?? {};
  }

  async function doMove() {
    if (!info || !moveOrg || !moveReport) return;
    saving = true; say('');
    const res = await projectsApi.move(info.namespace, moveOrg, false);
    saving = false;
    if (!res.ok) { say(res.error ?? 'Move failed'); return; }
    const ns = res.data?.project?.namespace ?? moveReport.to;
    await loadProjects(true);
    setProject(ns);
    navigateToDetail('projects', 'Projects', ns);
  }

  function addMember() {
    const q = addWho.trim().toLowerCase();
    const u = directory.find((d) => d.email.toLowerCase() === q || d.id === addWho.trim());
    if (!u) { say('Pick someone from the directory — members are matched by account, not by typed email.'); return; }
    if (dMembers.some((m) => m.id === u.id)) { say(`${u.email || u.id} is already a member.`); return; }
    dMembers = [...dMembers, { kind: 'user', id: u.id, email: u.email, name: u.name, role: addRole }];
    addWho = '';
    say('');
  }

  function removeMember(m: ProjectMember) {
    dMembers = dMembers.filter((x) => x.id !== m.id);
  }

  onMount(async () => {
    await loadProjects(true);
    directory = await loadDirectory();
  });
</script>

<div class="screen-header">
  <div class="breadcrumb">Projects <span class="sep">/</span> All</div>
  <div class="screen-title-row">
    <div class="title-block">
      <SubmenuToggle appType="projects" />
      <h1 class="screen-title">Projects</h1>
    </div>
    <div class="screen-actions">
      <NewButton label="New project" accent="spec" onclick={openCreate} />
    </div>
  </div>
</div>

<div class="screen-body">
  <PageBanner
    headline="Organise work into projects."
    highlightWord="projects"
    subtitle="Yours, or your organisation's — each one a namespace with its members and rules."
    description="A project you create lives under your handle (you-orders); an organisation's under its name (acme-orders). Make it private to keep it to yourself — or, in an org, to its members and admins. Pick one in the top bar and every page shows only its work."
    accent="spec"
  />
  <AppHome appType="projects" embedded />

  {#snippet projectCards(list: Project[])}
    <div class="card-grid">
      {#each list as p (p.name)}
        {@const c = card(p)}
        <EntityCard
          icon={(p.info?.display_name || p.name).slice(0, 2).toUpperCase()}
          color={p.platform ? '#777' : p.info?.owner_kind === 'org' ? '#1a7158' : '#5530a8'}
          name={c.name}
          description={c.description}
          tags={c.tags}
          meta={c.meta}
          project={p.name}
          actionLabel="Work in this project"
          onclick={() => openDetail(p.name)}
          onaction={() => setProject(p.name)}
        />
      {/each}
    </div>
  {/snippet}

  <FoldSection title="My Projects" count={mine.length} hint={profile ? `Yours (@${profile.handle}) and the ones you're a member of` : "Yours and the ones you're a member of"}>
    {@render projectCards(mine)}
  </FoldSection>
  <FoldSection title="Shared Projects" count={shared.length} hint="Your orgs' projects, and namespaces open to everyone">
    {@render projectCards(shared)}
  </FoldSection>
</div>

<SlidePanel open={panel !== 'closed'} onclose={closePanel} onsave={panel === 'create' ? create : panel === 'detail' && canEdit ? save : undefined}>
  {@const stepped = (panel === 'detail' && !!info) || panel === 'create'}
  <div class="pj-wiz" class:stepped>
    {#if stepped}
      <aside class="pj-rail">
        <div class="pj-head">
          <div class="pj-icon">{(panel === 'create' ? (newName || 'NP') : (info?.display_name || selectedName)).slice(0, 2).toUpperCase()}</div>
          <div>
            <h2 class="pj-title">{panel === 'create' ? 'New project' : (info?.display_name || selectedName)}</h2>
            <div class="pj-sub">
              {#if panel === 'create'}Yours or an organisation's{:else}
                <span class="mono">{selectedName}</span> · {info ? `${info.owner_kind === 'org' ? 'org' : 'personal'} · ${info.role}${info.private ? ' · private' : ''}` : (selected?.platform ? 'platform' : 'open to everyone')}
              {/if}
            </div>
          </div>
          <DetailActionsBar />
        </div>

        <WizardSteps steps={pjSteps} active={tab} wizard={panel === 'create'} onselect={(id) => (tab = id as any)} />
      </aside>
    {/if}
    <div class="pj-main">
  <div class="pj-scroll"><div class="pj">
    {#if !stepped}
    <div class="pj-head">
      <div class="pj-icon">{(panel === 'create' ? (newName || 'NP') : (info?.display_name || selectedName)).slice(0, 2).toUpperCase()}</div>
      <div>
        <h2 class="pj-title">{panel === 'create' ? 'New project' : (info?.display_name || selectedName)}</h2>
        <div class="pj-sub">
          {#if panel === 'create'}Yours or an organisation's{:else}
            <span class="mono">{selectedName}</span> · {info ? `${info.owner_kind === 'org' ? 'org' : 'personal'} · ${info.role}${info.private ? ' · private' : ''}` : (selected?.platform ? 'platform' : 'open to everyone')}
          {/if}
        </div>
      </div>
    </div>

    {/if}

    {#if stepped}<WizardHead step={pjSteps.find((x) => x.id === tab)} />{/if}
    {#if message}<div class="pj-msg" class:ok={messageOk}>{message}</div>{/if}

    {#snippet membersBlock()}
      <div class="pj-form">
        <table class="pj-table">
          <thead><tr><th>Member</th><th>Role</th><th></th></tr></thead>
          <tbody>
            {#each dMembers as m (m.id)}
              <tr>
                <td>{m.name || m.email || m.id}<div class="dim mono">{m.email || m.id}</div></td>
                <td><select bind:value={m.role} disabled={!canEdit}>{#each ROLES as r}<option value={r}>{r}</option>{/each}</select></td>
                <td>{#if canEdit}<button class="pj-x" onclick={() => removeMember(m)} title="Remove">×</button>{/if}</td>
              </tr>
            {:else}
              <tr><td colspan="3" class="dim">{view.owner_kind === 'org' ? "No named members — the org's admins (and, unless private, all its members) see it." : 'Only you so far.'}</td></tr>
            {/each}
          </tbody>
        </table>
        {#if canEdit}
          <div class="pj-add">
            <input bind:value={addWho} list="pj-users" placeholder="name or email" onkeydown={(e) => { if (e.key === 'Enter') addMember(); }} />
            <datalist id="pj-users">{#each directory as u}<option value={u.email || u.id}>{u.name}{u.source === 'sso' ? ' (SSO)' : ''}</option>{/each}</datalist>
            <select bind:value={addRole} title={ROLE_HINT[addRole]}>{#each ROLES as r}<option value={r}>{r}</option>{/each}</select>
            <button class="pj-btn" onclick={addMember}>Add</button>
          </div>
          <ul class="pj-roles">{#each ROLES as r}<li><b>{r}</b> — {ROLE_HINT[r]}</li>{/each}</ul>
        {/if}
        <p class="pj-note">Members decide who sees the project now. What each role may do inside it takes effect once project-scoped Cedar is on (next step); until then the org-wide roles decide. Save to keep changes.</p>
      </div>
    {/snippet}
    {#snippet policyBlock()}
      <div class="pj-form">
        <h4>Runtime</h4>
        <label><span>Where this project's flows run</span>
          <select value={dPolicy.runtime ?? ''} disabled={!canEdit}
                  onchange={(e) => { const v = e.currentTarget.value; dPolicy = { ...dPolicy }; if (v) dPolicy.runtime = v; else delete dPolicy.runtime; }}>
            <option value="">{view.owner_kind === 'org' ? "The organisation's setting" : 'Shared (default)'}</option>
            <option value="isolated">Isolated — only on {view.owner_kind === 'org' ? `${view.owner}'s own runtimes` : 'its own runtime'}</option>
            <option value="shared">Shared runtimes</option>
          </select>
          <em>{view.owner_kind === 'org'
            ? `Isolated runs only on runtimes labelled a3t.io/org=${view.owner}; never on another org's.`
            : `Isolated runs only on a runtime labelled a3t.io/project=${view.namespace}.`} Enforced by FlowR when a flow is deployed.</em>
        </label>
        <h4>Exposure</h4>
        <label><span>Who else may use this project — read, clone and run its flows, resources, triggers and templates; never change them</span>
          <select value={ex.audience ?? 'none'} disabled={!canEdit}
                  onchange={(e) => { const a = e.currentTarget.value; dPolicy = { ...dPolicy, exposure: a === 'none' ? { audience: 'none' }
                    : { ...ex, audience: a, orgs: isSystem ? ['a3t'] : (ex.orgs ?? []), kind: ex.kind ?? (isPlatformOrg ? 'shared' : 'community') } }; }}>
            <option value="none">Not exposed</option>
            <option value="orgs">{isSystem ? 'Org a3t' : 'Selected orgs'}</option>
            {#if !isSystem}<option value="public">All orgs</option>{/if}
          </select>
          {#if isSystem}<em>The system org exposes to a3t only; a3t decides what it passes on.</em>{/if}
        </label>
        {#if ex.audience === 'orgs' && !isSystem}
          <label><span>Org handles</span>
            <input class="mono" value={(ex.orgs ?? []).join(', ')} disabled={!canEdit} placeholder="acme, globex"
                   onchange={(e) => { dPolicy = { ...dPolicy, exposure: { ...ex, orgs: e.currentTarget.value.split(',').map((x) => x.trim()).filter(Boolean) } }; }} />
          </label>
        {/if}
        {#if ex.audience !== 'none' && isPlatformOrg}
          <label><span>Shown to them as</span>
            <select value={ex.kind ?? 'shared'} disabled={!canEdit}
                    onchange={(e) => { dPolicy = { ...dPolicy, exposure: { ...ex, kind: e.currentTarget.value } }; }}>
              <option value="shared">Shared — to use or wire</option>
              <option value="training">Training — examples to learn from and clone</option>
            </select>
          </label>
        {/if}
        <p class="pj-note">{ex.audience === 'none' ? 'Only this project\'s members and its org see it.' : 'Publish items into this project from their cards (Publish); others see them under Community, or From A3T / Training for the platform. Save to apply.'}</p>
        <h4>Project rules</h4>
        {#each POLICY_PRESETS as rule}
          <label class="pj-check">
            <input type="checkbox" disabled={!canEdit} checked={!!dPolicy[rule.key]}
                   onchange={(e) => { dPolicy = { ...dPolicy, [rule.key]: e.currentTarget.checked }; }} />
            <span><b>{rule.label}</b><em>{rule.hint}</em></span>
          </label>
        {/each}
        <p class="pj-note">Saved with the project, enforced once project-scoped Cedar is on (next step). Always in force, from the org baseline: deny by default, agents act only on behalf of a user, destructive tools need an admin, prod deploys need an admin, and InfoSec owns Cedar policy resources — see Security → Policies.</p>
      </div>
    {/snippet}

    {#if panel === 'create' && tab === 'settings'}
      <div class="pj-form">
        {#if !profile}
          <div class="pj-box">
            <h4>Choose your handle</h4>
            <p class="pj-note">Your personal projects are named after it: <span class="mono">{handleDraft || 'you'}-orders</span>. It's yours for good, shared with no org.</p>
            <div class="pj-row">
              <input class="mono" bind:value={handleDraft} placeholder="your-handle" />
              <button class="pj-btn primary" onclick={claimHandle} disabled={saving || !handleDraft.trim()}>Claim</button>
            </div>
          </div>
        {/if}
        <label><span>Owner</span>
          <select bind:value={owner}>
            {#if profile}<option value={`user:${profile.handle}`}>Me — @{profile.handle}</option>{/if}
            {#each orgs as o}<option value={`org:${o.handle}`}>{o.handle} (org, {o.role})</option>{/each}
          </select>
        </label>
        <label><span>Project name</span>
          <input class="mono" bind:value={newName} placeholder="orders" autocomplete="off" />
          <em>Namespace: <span class="mono">{previewNs || '<owner>-<name>'}</span> — lowercase letters, digits and dashes.</em>
        </label>
        <label><span>Display name</span><input bind:value={newDisplay} placeholder={newName || 'Orders'} /></label>
        <label><span>Description</span><textarea rows="3" bind:value={newDesc} placeholder="What this project delivers"></textarea></label>
        <label class="pj-check">
          <input type="checkbox" bind:checked={newPrivate} />
          <span><b>Private</b><em>{owner.startsWith('org:') ? "Only the project's members and the org's admins and security admins see it." : 'Only you see it.'}</em></span>
        </label>
      </div>

    {:else if panel === 'create' && tab === 'access'}
      <h3 class="pj-sec">Members</h3>
      {@render membersBlock()}
      <h3 class="pj-sec">Policy</h3>
      {@render policyBlock()}

    {:else if panel === 'create' && tab === 'review'}
      <div class="pj-form">
        <div class="pj-facts">
          <div><span>Namespace</span><span class="mono">{previewNs || '—'}</span></div>
          <div><span>Owner</span>{owner.startsWith('org:') ? `org ${ownerHandle}` : ownerHandle ? `@${ownerHandle}` : '—'}</div>
          <div><span>Display name</span>{newDisplay.trim() || newName.trim() || '—'}</div>
          <div><span>Description</span>{newDesc.trim() || '—'}</div>
          <div><span>Visibility</span>{newPrivate ? 'Private' : owner.startsWith('org:') ? 'Visible to the organisation' : 'Visible to you'}</div>
          <div><span>Members</span>{dMembers.length ? dMembers.map((m) => `${m.name || m.email || m.id} (${m.role})`).join(', ') : 'None besides the owner'}</div>
          <div><span>Runs on</span>{dPolicy.runtime === 'isolated' ? 'Its own runtimes only' : dPolicy.runtime === 'shared' ? 'Shared runtimes' : 'The default'}</div>
          <div><span>Exposed to</span>{ex.audience === 'none' ? 'Nobody else' : ex.audience === 'public' ? 'All orgs' : (ex.orgs ?? []).join(', ') || 'Selected orgs'}</div>
          <div><span>Rules</span>{POLICY_PRESETS.filter((r) => dPolicy[r.key]).map((r) => r.label).join(' · ') || 'The org baseline only'}</div>
        </div>
        <p class="pj-note">Contents, delivery and health appear once it has flows and agents.</p>
      </div>

    {:else if panel === 'detail' && !info}
      <div class="pj-form">
        <p class="pj-note">{selected?.platform
          ? 'A platform namespace: the shared LLMs, memory and MCP servers every project uses, and A3T’s own flows.'
          : 'This namespace is from before projects, so it has no owner and is open to everyone. New work belongs in a project of yours or your organisation’s.'}</p>
        <div class="pj-facts"><div><span>Holds</span>{selected ? card(selected).meta : ''}</div></div>
        <div class="pj-row"><button class="pj-btn" onclick={() => setProject(selectedName)}>Work in this namespace</button></div>
      </div>

    {:else if tab === 'overview' && info}
      <div class="pj-form">
        <!-- About: what it is, then the four facts as a grid — read at a glance. -->
        <section class="pj-card">
          {#if info.description}<p class="pj-lead">{info.description}</p>
          {:else}
            <div class="pj-empty">
              <span>No description yet — say what this project delivers, so others know what they are looking at.</span>
              {#if canEdit}<button class="pj-btn" onclick={() => (tab = 'settings')}>Add a description</button>{/if}
            </div>
          {/if}
          <dl class="pj-kv">
            <div><dt>Owner</dt><dd>{info.owner_kind === 'org' ? `Organisation ${info.owner}` : `@${info.owner}`}</dd></div>
            <div><dt>Visibility</dt><dd>{info.private ? 'Private' : info.owner_kind === 'org' ? 'The organisation' : 'Only you'}</dd></div>
            <div><dt>Your role</dt><dd>{info.role ?? '—'}</dd></div>
            <div><dt>Created</dt><dd>{info.created_at ? new Date(info.created_at).toLocaleDateString(undefined, { day: 'numeric', month: 'short', year: 'numeric' }) : '—'}</dd></div>
          </dl>
        </section>
        {#if !insight}
          <!-- the header bar shows loading -->
        {:else}
          <div class="pj-tiles">
            <button class="pj-tile" onclick={() => (tab = 'contents')}><b>{insight.agents.length}</b><span>{insight.agents.length === 1 ? 'agent' : 'agents'}</span></button>
            <button class="pj-tile" onclick={() => (tab = 'contents')}><b>{insight.flows.length}</b><span>{insight.flows.length === 1 ? 'flow' : 'flows'}</span></button>
            <button class="pj-tile" onclick={() => (tab = 'contents')}><b>{insight.resources.length}</b><span>{insight.resources.length === 1 ? 'resource' : 'resources'}</span></button>
            <button class="pj-tile" onclick={() => (tab = 'health')}><b>{insight.deployments.length}</b><span>deployed</span></button>
            <button class="pj-tile" onclick={() => (tab = 'health')}><b>{recentRuns.length}</b><span>{recentRuns.length === 1 ? 'run today' : 'runs today'}</span></button>
            <button class="pj-tile" class:bad={failedRecent.length > 0} onclick={() => (tab = 'health')}><b>{failedRecent.length}</b><span>failed today</span></button>
          </div>
          <div class="pj-h"><h4>Latest runs</h4>{#if insight.runs.length}<button class="link" onclick={() => navigateTo('executions', 'Executions')}>All runs</button>{/if}</div>
          {#if insight.runs.length === 0}<p class="dim">Nothing has run in this project yet.</p>
          {:else}
            <div class="pj-list">
              {#each insight.runs.slice(0, 5) as r (r.instance_id)}
                <button class="pj-li" onclick={() => navigateToDetail('executions', 'Executions', r.instance_id)}>
                  <span class="mono">{r.name ?? r.instance_id}</span><span class="pj-st {/fail|error/i.test(r.status) ? 'bad' : /complet|succe/i.test(r.status) ? 'good' : ''}">{r.status}</span><span class="dim">{when(r.started_at)}</span>
                </button>
              {/each}
            </div>
          {/if}
        {/if}
        <div class="pj-row"><button class="pj-btn" onclick={() => setProject(info.namespace)}>Work in this project</button></div>
      </div>

    {:else if tab === 'contents' && info}
      <div class="pj-form">
        {#if !insight}
          <!-- the header bar shows loading -->
        {:else}
          {#each [['agent', 'Agents', insight.agents], ['flow', 'Flows', insight.flows], ['template', 'Templates', insight.templates], ['resource', 'Resources', insight.resources], ['trigger', 'Triggers', insight.triggers], ['canvas', 'Canvases', insight.canvases]] as [kind, label, rows] (kind)}
            {@const list = rows as Row[]}
            <h4>{label} <span class="pj-count">{list.length}</span></h4>
            {#if list.length === 0}<p class="dim">None yet.</p>
            {:else}
              <div class="pj-list">
                {#each list as r (r.id ?? r.name)}
                  {@const rel = kind === 'flow' ? releases.find((x) => x.name === r.name) : undefined}
                  <div class="pj-li-row">
                    <button class="pj-li" onclick={() => openItem(kind as any, r)} title="Open in its app">
                      <span class="mono">{r.name}</span><span class="dim">{rel ? '' : (r.sub ?? '')}</span>{#if r.status}<span class="pj-st">{r.status}</span>{/if}
                    </button>
                    <!-- Which release callers without a pinned version get. -->
                    {#if rel}
                      <select class="pj-ver-pick" value={rel.active} disabled={!canEdit || rel.versions.length < 2}
                        title="The active release: what callers get unless they pin a version"
                        onchange={(e) => activate(rel.name, e.currentTarget.value)}>
                        {#each rel.versions as v}<option value={v}>v{v}{v === rel.active ? ' · active' : ''}</option>{/each}
                      </select>
                    {/if}
                  </div>
                {/each}
              </div>
            {/if}
          {/each}
        {/if}
      </div>

    {:else if tab === 'environments' && info}
      <div class="pj-form">
        <div class="pj-sample"><b>Sample</b> Environments are not stored or deployed yet — this shows how the screen will work. Edits here are not saved.</div>

        <div class="pj-h"><h4>What runs where</h4><button class="link" onclick={() => navigateTo('runtime', 'Runtimes')}>Details in Runtimes</button></div>
        {#if !insight}
          <!-- the header bar shows loading -->
        {:else if whereRuns.length === 0}<p class="dim">Nothing in this project to deploy yet.</p>
        {:else}
          <table class="pj-table pj-matrix">
            <thead><tr><th></th>{#each envNames as e}<th><span class="pj-env {e}">{e}</span></th>{/each}</tr></thead>
            <tbody>
              {#each whereRuns as m (m.kind + m.name)}
                <tr>
                  <td><span class="mono">{m.name}</span> <span class="dim">{m.kind}</span></td>
                  {#each envNames as e}<td>{#if m.at[e]}<span class="pj-ver">v{m.at[e]}</span>{:else}<span class="dim">not deployed</span>{/if}</td>{/each}
                </tr>
              {/each}
            </tbody>
          </table>
        {/if}

        <div class="pj-h"><h4>Variables</h4>{#if canEdit}<button class="pj-btn" onclick={addVar}>Add variable</button>{/if}</div>
        <p class="pj-note">A canvas deploying into an environment hands that environment's values to its pods. A secret's value is a Secret resource to read it from, never the secret itself.</p>
        <table class="pj-table pj-vars">
          <thead><tr><th>Name</th>{#each envNames as e}<th><span class="pj-env {e}">{e}</span></th>{/each}<th></th></tr></thead>
          <tbody>
            {#each envVars as v, i (i)}
              <tr>
                <td>
                  <input class="mono" bind:value={v.name} placeholder="NAME" disabled={!canEdit} />
                  <label class="pj-secret"><input type="checkbox" bind:checked={v.secret} disabled={!canEdit} /> secret</label>
                </td>
                {#each envNames as e}
                  <td class:missing={!!v.name.trim() && !(v.values[e] ?? '').trim()}>
                    <input class="mono" bind:value={v.values[e]} placeholder={v.secret ? 'project/secret' : 'not set'} disabled={!canEdit} />
                  </td>
                {/each}
                <td>{#if canEdit}<button class="pj-x" onclick={() => removeVar(i)} title="Remove {v.name || 'this variable'}" aria-label="Remove">×</button>{/if}</td>
              </tr>
            {:else}
              <tr><td colspan={envNames.length + 2} class="dim">No variables yet.</td></tr>
            {/each}
          </tbody>
        </table>
        {#if missingValues}<p class="pj-warn">{missingValues} value{missingValues > 1 ? 's are' : ' is'} not set — a deployment there gets nothing for {missingValues > 1 ? 'them' : 'it'}.</p>{/if}
      </div>

    {:else if tab === 'health' && info}
      <div class="pj-form">
        {#if !insight}
          <!-- the header bar shows loading -->
        {:else}
          <h4>Deployments <span class="pj-count">{insight.deployments.length}</span></h4>
          {#if insight.deployments.length === 0}<p class="dim">Nothing from this project is deployed.</p>
          {:else}
            <table class="pj-table">
              <thead><tr><th>Deployed</th><th>Runtime</th><th>State</th><th>Runs / h</th><th>Errors</th><th>p95</th></tr></thead>
              <tbody>
                {#each insight.deployments as row (row.deployment.id)}
                  <tr class="pj-click" onclick={() => navigateToDetail('runtime', 'Runtimes', `dep:${row.deployment.id}`)}>
                    <td class="mono">{row.deployment.target}<div class="dim">{row.deployment.kind}</div></td>
                    <td class="mono">{row.deployment.runtime ?? '—'}</td>
                    <td><span class="pj-st {row.serving === 'serving' ? 'good' : /paused|hibernat/.test(row.serving) ? '' : 'bad'}">{String(row.serving ?? '').replace('-', ' ')}</span></td>
                    <td>{row.stats?.runsLastHour ?? '—'}</td>
                    <td class:bad={(row.stats?.failed ?? 0) > 0}>{row.stats ? `${Math.round((row.stats.errorRate ?? 0) * 100)}%` : '—'}</td>
                    <td>{row.stats?.p95Ms != null ? `${Math.round(row.stats.p95Ms)} ms` : '—'}</td>
                  </tr>
                {/each}
              </tbody>
            </table>
          {/if}
          <h4>Failed in the last 24 hours <span class="pj-count">{failedRecent.length}</span></h4>
          {#if failedRecent.length === 0}<p class="dim">No failures.</p>
          {:else}
            <div class="pj-list">
              {#each failedRecent.slice(0, 10) as r (r.instance_id)}
                <button class="pj-li" onclick={() => navigateToDetail('executions', 'Executions', r.instance_id)}>
                  <span class="mono">{r.name ?? r.instance_id}</span><span class="dim">{r.error ? String(r.error).slice(0, 80) : ''}</span><span class="dim">{when(r.started_at)}</span>
                </button>
              {/each}
            </div>
          {/if}
          <p class="pj-note">Totals are live. History over time (UI-8) and runtime events such as restarts (UI-9) need their APIs — see the UI limitations.</p>
        {/if}
      </div>

    {:else if tab === 'access' && info}
      <h3 class="pj-sec">Members</h3>
      {@render membersBlock()}
      <h3 class="pj-sec">Policy</h3>
      {@render policyBlock()}
      <div class="pj-form"><div class="pj-row">
        <button class="pj-btn" onclick={() => navigateTo('logs', 'Decision log', { q: info.namespace })}>Decisions in this project (allowed and blocked)</button>
      </div></div>

    {:else if tab === 'settings' && info}
      <div class="pj-form">
        <label><span>Display name</span><input bind:value={dName} disabled={!canEdit} /></label>
        <label><span>Description</span><textarea rows="3" bind:value={dDesc} disabled={!canEdit}></textarea></label>
        <label class="pj-check">
          <input type="checkbox" bind:checked={dPrivate} disabled={!canEdit} />
          <span><b>Private</b><em>{info.owner_kind === 'org' ? "Only its members and the org's admins and security admins see it." : 'Only you see it.'}</em></span>
        </label>
        <div class="pj-facts">
          <div><span>Namespace</span><span class="mono">{info.namespace}</span></div>
          <div><span>Owner</span>{info.owner_kind === 'org' ? `org ${info.owner}` : `@${info.owner}`}</div>
        </div>
        <!-- Change the owner: a personal project moves into an org (the server
             does this); an org's project to another org needs the server first
             (UI limitations, SRV-8) — shown, and said, not pretended. -->
        {#if (info.owner_kind === 'user' && info.role === 'owner') || (info.owner_kind === 'org' && canEdit)}
          <div class="pj-box">
            <h4>{info.owner_kind === 'org' ? 'Change organisation' : 'Move to an organisation'}</h4>
            {#if targetOrgs.length === 0}
              <p class="pj-note">You need the admin role in {info.owner_kind === 'org' ? 'another' : 'an'} organisation to move a project into it.</p>
            {:else if info.owner_kind === 'org'}
              <p class="pj-note">Now owned by <b>{info.owner}</b>. Moving it makes the namespace <span class="mono">&lt;org&gt;-{info.name}</span>, with its flows, resources and triggers.</p>
              <div class="pj-row">
                <select bind:value={moveOrg}>
                  <option value="">Choose an organisation…</option>
                  {#each targetOrgs as o}<option value={o.handle}>{o.handle}</option>{/each}
                </select>
                <button class="pj-btn" disabled>Preview</button>
              </div>
              <p class="pj-warn">Not available yet: the server moves only a personal project into an organisation, not between organisations.</p>
            {:else}
              <p class="pj-note">The namespace becomes <span class="mono">&lt;org&gt;-{info.name}</span>. Flows, resources and triggers move with it, and references inside the project are rewritten. Run history stays under the old name.</p>
              <div class="pj-row">
                <select bind:value={moveOrg} onchange={() => (moveReport = null)}>
                  <option value="">Choose an organisation…</option>
                  {#each targetOrgs as o}<option value={o.handle}>{o.handle}</option>{/each}
                </select>
                <button class="pj-btn" onclick={previewMove} disabled={!moveOrg || saving}>Preview</button>
              </div>
              {#if moveReport}
                <div class="pj-report">
                  <div><b>{moveReport.from}</b> → <b>{moveReport.to}</b></div>
                  <div>{moveReport.flows?.length ?? 0} flows, {moveReport.resources?.length ?? 0} resources, {moveReport.triggers?.length ?? 0} triggers move.</div>
                  {#if moveReport.external_references?.length}
                    <div class="warn">These refer to it from outside and will break: {moveReport.external_references.join(', ')}</div>
                  {:else}
                    <div class="dim">Nothing outside the project refers to it.</div>
                  {/if}
                  <button class="pj-btn primary" onclick={doMove} disabled={saving}>{saving ? 'Moving…' : `Move to ${moveOrg}`}</button>
                </div>
              {/if}
            {/if}
          </div>
        {/if}

        <div class="pj-danger">
          <h4>Delete project</h4>
          {#if !canEdit}
            <p class="pj-note">Only the project's owner or an admin of its organisation can delete it.</p>
          {:else if !insight}
            <!-- the header bar shows loading -->
          {:else if blockers.length}
            <p class="pj-note">It still holds {blockers.join(', ')}. Move them to another project or delete them first — a project is deleted only when it is empty.</p>
            <button class="pj-btn danger" disabled>Delete project</button>
          {:else}
            <p class="pj-note">Removes the project, its members and its rules. Run history stays.</p>
            <button class="pj-btn danger" onclick={remove}>Delete project</button>
          {/if}
        </div>
      </div>
    {/if}

  </div></div>
    {#if stepped}
      <WizardNav steps={pjSteps} active={tab} wizard={panel === 'create'} onselect={(id) => (tab = id as any)}
        final={panel === 'create' ? create : canEdit ? save : undefined}
        finalLabel={panel === 'create' ? 'Create project' : 'Save'} finalBusy={saving} busyLabel={panel === 'create' ? 'Creating…' : 'Saving…'} />
    {/if}
    </div>
  </div>
</SlidePanel>

<style>
  .screen-header { padding: 18px 32px 12px; border-bottom: 1px solid var(--border); background: var(--bg-page); }
  .pj-wiz { height: 100%; display: grid; grid-template-columns: 1fr; }
  .pj-wiz.stepped { grid-template-columns: 280px 1fr; }
  .pj-rail { display: flex; flex-direction: column; min-height: 0; background: var(--bg-surface); border-right: 1px solid var(--border); }
  .pj-rail .pj-head { display: flex; align-items: center; gap: 12px; min-height: 72px; padding: 12px 20px; box-sizing: border-box; border-bottom: 1px solid var(--border); margin: 0; }
  .pj-rail .pj-icon { width: 32px; height: 32px; border-radius: 8px; font-size: 11px; }
  .pj-rail .pj-title { font-family: var(--font-serif); font-size: 15px; font-weight: 500; line-height: 1.25; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; margin: 0; }
  .pj-rail .pj-sub { font-size: 11px; color: var(--text-4); margin-top: 2px; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .pj-main { display: flex; flex-direction: column; min-height: 0; min-width: 0; }
  .pj-scroll { flex: 1; min-height: 0; overflow-y: auto; }
  .breadcrumb { font-size: 12px; color: var(--text-3); margin-bottom: 8px; font-family: var(--font-mono); }
  .breadcrumb .sep { color: var(--text-5); }
  .screen-title-row { display: flex; align-items: center; gap: 18px; }
  .title-block { flex: 1; display: flex; align-items: center; gap: 10px; }
  .screen-title { font-family: var(--font-serif); font-size: 24px; font-weight: 500; letter-spacing: -0.02em; }
  .screen-actions { display: flex; gap: 8px; }
  .screen-body { flex: 1; overflow-y: auto; padding: 20px 32px 40px; }
  .card-grid { display: grid; grid-template-columns: repeat(auto-fill, minmax(300px, 1fr)); gap: 12px; }

  .pj { max-width: 704px; padding: 24px 32px 40px; }
  .pj-head { display: flex; align-items: center; gap: 14px; margin-bottom: 18px; }
  .pj-icon { width: 44px; height: 44px; border-radius: var(--r-sm); background: #5530a8; color: white; display: grid; place-items: center; font: 700 13px var(--font-mono); }
  .pj-title { margin: 0; font-family: var(--font-serif); font-size: 22px; font-weight: 500; }
  .pj-sub { font-size: 12px; color: var(--text-4); margin-top: 2px; }
  .pj-tabs { display: flex; gap: 4px; border-bottom: 1px solid var(--border); margin-bottom: 18px; }
  .pj-tab { padding: 8px 12px; border: none; background: none; font-size: 13px; color: var(--text-3); cursor: pointer; border-bottom: 2px solid transparent; margin-bottom: -1px; }
  .pj-tab.active { color: var(--spec); border-bottom-color: var(--spec); font-weight: 550; }
  .pj-msg { padding: 8px 12px; border-radius: var(--r-sm); background: var(--err-bg, #fde8e8); color: var(--err, #b42318); font-size: 12.5px; margin-bottom: 14px; }
  .pj-msg.ok { background: var(--spec-bg); color: var(--spec); }
  .pj-form { display: flex; flex-direction: column; gap: 14px; }
  .pj-form h4 { margin: 0; font-size: 13px; font-weight: 600; color: var(--text-2); }
  .pj-form label { display: flex; flex-direction: column; gap: 5px; font-size: 12px; color: var(--text-3); }
  .pj-form label > span { font-weight: 550; color: var(--text-2); }
  .pj-form label em { font-style: normal; font-size: 11px; color: var(--text-4); }
  .pj-form input, .pj-form textarea, .pj-form select {
    padding: 7px 9px; border: 1px solid var(--border); border-radius: var(--r-sm);
    background: var(--bg-surface); color: var(--text-1); font-size: 13px; font-family: inherit;
  }
  .mono { font-family: var(--font-mono); }
  .dim { color: var(--text-4); font-size: 11px; }
  .warn { color: var(--warn, #b45309); font-size: 12px; }
  .pj-box { border: 1px solid var(--border); border-radius: var(--r); padding: 14px 16px; display: flex; flex-direction: column; gap: 10px; background: var(--bg-surface); }
  .pj-report { display: flex; flex-direction: column; gap: 6px; font-size: 12.5px; color: var(--text-2); }
  .pj-report .pj-btn { align-self: flex-start; }
  .pj-facts { display: grid; gap: 6px; font-size: 12.5px; color: var(--text-2); }
  /* Facts as rows: label, value, a hairline between — not running text. */
  .pj-facts { gap: 0 !important; border: 1px solid var(--border); border-radius: 10px; background: var(--bg-surface); overflow: hidden; }
  .pj-facts > div { display: flex; gap: 12px; padding: 9px 14px; border-bottom: 1px solid var(--divider, var(--border)); }
  .pj-facts > div:last-child { border-bottom: 0; }
  .pj-facts > div > span:first-child { flex: 0 0 110px; color: var(--text-4); }
  .pj-card { display: flex; flex-direction: column; gap: 16px; padding: 18px 20px; border: 1px solid var(--border); border-radius: 12px; background: var(--bg-surface); }
  .pj-empty { display: flex; align-items: center; gap: 14px; flex-wrap: wrap; padding: 12px 14px; border: 1px dashed var(--border-strong, #d9d4c6); border-radius: 10px; font-size: 13px; color: var(--text-3); }
  .pj-empty span { flex: 1 1 240px; }
  .pj-kv { display: grid; grid-template-columns: repeat(auto-fit, minmax(118px, 1fr)); gap: 14px 18px; margin: 0; }
  .pj-kv dt { font: 500 10.5px var(--font-mono); letter-spacing: 0.06em; text-transform: uppercase; color: var(--text-4); margin-bottom: 4px; }
  .pj-kv dd { margin: 0; font-size: 13.5px; color: var(--text-1); }
  .pj-h { display: flex; align-items: baseline; justify-content: space-between; margin-top: 6px; }
  .pj-row { display: flex; gap: 8px; flex-wrap: wrap; align-items: center; }
  .pj-btn { padding: 7px 14px; border-radius: var(--r-sm); border: 1px solid var(--border); background: var(--bg-surface); font-size: 12.5px; color: var(--text-1); cursor: pointer; }
  .pj-btn:hover:not(:disabled) { border-color: var(--spec); color: var(--spec); }
  .pj-btn.primary { background: var(--spec); border-color: var(--spec); color: white; }
  .pj-btn.primary:hover:not(:disabled) { color: white; opacity: 0.9; }
  .pj-btn.danger:hover:not(:disabled) { border-color: var(--err, #d44); color: var(--err, #d44); }
  .pj-btn:disabled { opacity: 0.5; cursor: default; }
  .pj-note { margin: 0; font-size: 11.5px; color: var(--text-4); line-height: 1.5; }
  .pj-table { width: 100%; border-collapse: collapse; font-size: 12.5px; }
  .pj-table th { text-align: left; font-weight: 550; color: var(--text-4); font-size: 11px; padding: 6px 8px; border-bottom: 1px solid var(--border); }
  .pj-table td { padding: 8px; border-bottom: 1px solid var(--divider, var(--border)); vertical-align: middle; }
  .pj-x { border: none; background: none; color: var(--text-4); font-size: 16px; cursor: pointer; }
  .pj-x:hover { color: var(--err, #d44); }
  .pj-add { display: grid; grid-template-columns: 1fr 120px auto; gap: 8px; }
  .pj-roles { margin: 0; padding-left: 18px; font-size: 11.5px; color: var(--text-3); line-height: 1.6; }
  .pj-check { flex-direction: row !important; align-items: flex-start; gap: 10px !important; }
  .pj-check span { display: flex; flex-direction: column; gap: 2px; font-size: 13px; color: var(--text-1); }
  .pj-check em { font-style: normal; font-size: 11.5px; color: var(--text-4); }
  /* Overview, Contents, Health, Access, Settings */
  .pj-sec { margin: 6px 0 10px; font: 400 17px var(--font-serif); color: var(--text-1); }
  .pj-sec:not(:first-child) { margin-top: 26px; }
  .pj-lead { margin: 0; font-size: 14px; color: var(--text-1); line-height: 1.5; }
  .pj-tiles { display: grid; grid-template-columns: repeat(auto-fill, minmax(96px, 1fr)); gap: 8px; }
  .pj-tile { display: flex; flex-direction: column; align-items: flex-start; gap: 2px; padding: 10px 12px; border: 1px solid var(--border); border-radius: 10px; background: var(--bg-surface); cursor: pointer; text-align: left; font: inherit; }
  .pj-tile:hover { border-color: var(--spec); }
  .pj-tile b { font: 400 22px var(--font-serif); color: var(--text-1); }
  .pj-tile span { font-size: 12px; color: var(--text-3); }
  .pj-tile.bad b { color: var(--err, #b42318); }
  .pj-count { font: 500 10.5px var(--font-mono); color: var(--text-4); background: var(--bg-subtle); border-radius: 4px; padding: 1px 6px; margin-left: 4px; }
  .pj-list { display: flex; flex-direction: column; gap: 6px; }
  .pj-li { display: flex; align-items: center; gap: 10px; padding: 8px 12px; border: 1px solid var(--border); border-radius: 8px; background: var(--bg-surface); cursor: pointer; text-align: left; font: inherit; font-size: 12.5px; color: var(--text-1); }
  .pj-li:hover { border-color: var(--spec); }
  .pj-li > :first-child { flex: 1 1 auto; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .pj-st { font: 500 10.5px var(--font-mono); letter-spacing: 0.05em; text-transform: uppercase; color: var(--text-3); }
  .pj-st.good { color: var(--green, #15803d); }
  .pj-st.bad, td.bad { color: var(--err, #b42318); }
  .pj-click { cursor: pointer; }
  .pj-click:hover td { background: var(--bg-subtle); }
  .pj-danger { margin-top: 18px; padding: 14px 16px; border: 1px solid var(--err-bg, #fde8e8); border-radius: 10px; display: flex; flex-direction: column; gap: 8px; align-items: flex-start; }
  .pj-danger h4 { color: var(--err, #b42318) !important; }
  .link { border: 0; background: none; padding: 0; color: var(--spec); cursor: pointer; font: inherit; }
  .pj-sample { display: flex; gap: 10px; align-items: baseline; padding: 9px 12px; border: 1px dashed var(--warn, #b45309); border-radius: 8px; background: var(--warn-bg, #fbf0e0); font-size: 12.5px; color: var(--text-2); }
  .pj-sample b { font: 600 10.5px var(--font-mono); letter-spacing: 0.06em; text-transform: uppercase; color: var(--warn, #b45309); }
  .pj-matrix td, .pj-matrix th { text-align: left; }
  .pj-ver { font: 500 12px var(--font-mono); color: var(--text-1); background: var(--bg-subtle); border: 1px solid var(--border); border-radius: 4px; padding: 1px 6px; }
  .pj-vars td { padding: 5px 6px; }
  .pj-vars input.mono { width: 100%; box-sizing: border-box; font-size: 12px; }
  .pj-vars td.missing input { border-color: var(--warn, #b45309); background: var(--warn-bg, #fbf0e0); }
  .pj-secret { display: inline-flex !important; flex-direction: row !important; align-items: center; gap: 4px; margin-top: 4px; font-size: 11px !important; }
  .pj-warn { margin: 0; font-size: 12px; color: var(--warn, #b45309); }
  .pj-li-row { display: flex; align-items: center; gap: 8px; }
  .pj-li-row .pj-li { flex: 1 1 auto; min-width: 0; }
  .pj-ver-pick { flex: 0 0 auto; font: 12px var(--font-mono); }
  .pj-envs { display: flex; gap: 8px; }
  .pj-env { padding: 4px 12px; border-radius: 12px; font: 12px var(--font-mono); border: 1px solid var(--border); }
  .pj-env.dev { color: #15803d; border-color: #86efac; }
  .pj-env.staging { color: #b45309; border-color: #fcd34d; }
  .pj-env.prod { color: #b91c1c; border-color: #fca5a5; }
</style>
