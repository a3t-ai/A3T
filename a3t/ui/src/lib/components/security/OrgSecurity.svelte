<script lang="ts">
  /// Security › an org: its identity and its policy files, in one place.
  ///   Overview — the org at a glance, and who looks after what;
  ///   Members — its people and groups, the role each holds in it, and who
  ///     else reaches its projects and how (its admins change roles);
  ///   Single sign-on — its own identity provider, domains and SCIM;
  ///   Policy files — its roles, rules and access, kept as files in A3T (edited in Studio).
  /// Its admins change members and sign-in; its security admins read them. Both
  /// manage the policy files (the org's security). SuperAdmin does everything;
  /// SecurityAdmin reads members and sign-in and manages policy files. The
  /// server checks every one of these; this page only offers what will work.
  import { untrack } from 'svelte';
  import { roles, roleLabel } from '$lib/core/auth/roles.svelte.ts';
  import { draftPolicy, checkDraft, POLICY_AGENT, type PolicyDraft } from './policyAgent';
  import AccessReviews from './AccessReviews.svelte';
  import RulesList from './rules/RulesList.svelte';
  import RuleEditor from './rules/RuleEditor.svelte';
  import GovernSettings from './rules/GovernSettings.svelte';
  import * as YAML from 'yaml';
  import PropertiesTree from '$lib/components/shared/PropertiesTree.svelte';
  import WizardHead from '$lib/components/shared/WizardHead.svelte';
  import { POLICY_STARTERS, type PolicyDir } from '$lib/studio-ui';
  import { openInStudio } from '$lib/stores/panels.svelte.ts';
  import { orgsApi, orgPolicy, securityPolicies, securityCatalogue, type PolicyBroken, type PolicyChange, type ReplayResult, type AuditVerify, audit as auditApi, loadDirectory, people as peopleApi, type OrgInfo, type OrgRole, type OrgIdp, type DirectoryUser, type AuditPeople, type OrgPolicyFiles, type OrgSettings, type OrgGovernance } from '$lib/services/api';
  import { isInfosec } from '$lib/stores/auth';
  import { API_BASE } from '$lib/config';
  import { loadProjects } from '$lib/stores/context.svelte.ts';
  import { bumpItems } from '$lib/stores/apps.svelte.ts';

  /// `project`: open Members filtered to who reaches that project.
  /// `section`: identity (who people are: sign-in, members, roles and groups)
  /// or policy (what is allowed: the org's policy files). Two pages, one org.
  let { org: handle, section = 'identity', project = '', tab = $bindable('overview') }: { org: string; section?: 'identity' | 'policy'; project?: string; tab?: string } = $props();

  const ROLES: OrgRole[] = ['member', 'admin', 'securityadmin'];
  // Names from the role table (ID-1).
  const ROLE_LABEL: Record<string, string> = $derived(Object.fromEntries(['member', 'admin', 'securityadmin'].map((r) => [r, roleLabel('org', r)])));
  const CAP_LABEL: Record<string, string> = $derived({
    ...Object.fromEntries(roles('project').map((r) => [r.id, r.label])),
    'org-admin': roleLabel('org', 'admin', 'Org admin'), 'org-securityadmin': roleLabel('org', 'securityadmin', 'Org security admin'),
    'org-member': roleLabel('org', 'member', 'Org member'), superadmin: 'SuperAdmin', exposed: 'Shared with them',
  });
  const ACCESS_WORD: Record<string, string> = { write: 'builds and changes', run: 'runs', read: 'reads' };

  let org = $state<OrgInfo | null>(null);
  let aud = $state<AuditPeople | null>(null);
  let files = $state<OrgPolicyFiles | null>(null);
  let err = $state('');
  let audErr = $state('');
  let filesErr = $state('');
  let msg = $state(''); let msgOk = $state(false);
  let busy = $state('');
  let directory = $state<DirectoryUser[]>([]);

  const infosec = isInfosec();
  // Members and sign-in: the org's admins and security admins, and InfoSec —
  // admins not when the org lets only its security admins change identity.
  const canAdmin = $derived(!!(org?.can_identity ?? org?.can_admin));
  // Policy files are the org's security: its admins and security admins, and InfoSec.
  const managesFiles = $derived(infosec || org?.my_role === 'admin' || org?.my_role === 'securityadmin');
  const say = (t: string, ok = false) => { msg = t; msgOk = ok; };

  let loadedFor = '';
  $effect(() => {
    const h = handle; const p = project;
    untrack(() => { if (h !== loadedFor) { loadedFor = h; void load(p); } else { fProject = p; fCap = ''; } });
  });

  async function load(p = '') {
    err = ''; msg = ''; stopName = ''; org = null; aud = null; files = null; openRow = ''; fQ = ''; fProject = p; fCap = ''; fRole = '';
    const h = handle;
    const [o, a, f, hl] = await Promise.all([orgsApi.list(), peopleApi.audit(h), orgPolicy.files(h), securityPolicies.health()]);
    if (h !== handle) return;
    healthAll = hl.ok ? hl.data?.broken ?? [] : [];
    const found = (o.ok ? o.data?.orgs ?? [] : []).find((x: OrgInfo) => x.handle === h) ?? null;
    if (!found || !found.can_audit) { err = o.ok ? `No org ${h} whose security you look after.` : (o.error ?? 'Organisations could not be read.'); return; }
    org = found;
    aud = a.ok ? a.data ?? null : null; audErr = a.ok ? '' : (a.error ?? `HTTP ${a.status}`);
    files = f.ok ? f.data ?? null : null; filesErr = f.ok ? '' : (f.error ?? `HTTP ${f.status}`);
    pickFrom(files);
    if ((found.can_identity ?? found.can_admin) && !directory.length) directory = await loadDirectory();
  }
  // GOV-4: this org's files that no longer compile (its projects are closed).
  let healthAll = $state<PolicyBroken[]>([]);
  const broken = $derived(healthAll.find((b) => b.kind === 'org' && org && (b.owner === org.id || b.owner === org.handle)) ?? null);

  async function refreshOrg() {
    const o = await orgsApi.list();
    const found = (o.ok ? o.data?.orgs ?? [] : []).find((x: OrgInfo) => x.handle === handle) ?? null;
    if (found) org = found;
    const a = await peopleApi.audit(handle);
    if (a.ok) aud = a.data ?? null;
    bumpItems();
  }

  // ── Members: the org's members and everyone else who reaches its projects ──
  // One list. A member's role comes from the org (members table); projects
  // reached come from the audit. People outside the org are listed so an
  // admin sees who reaches its projects without being in it.
  type Person = { id: string; name: string; email: string; account: boolean; member?: OrgInfo['members'][number]; orgRole: string; projects: AuditPeople['users'][number]['projects'] };
  let fQ = $state(''); let fRole = $state(''); let fProject = $state(''); let fCap = $state('');
  let openRow = $state('');
  const everyone = $derived.by((): Person[] => {
    const by = new Map<string, Person>();
    const byEmail = new Map<string, Person>();
    for (const m of org?.members ?? []) {
      const x: Person = { id: m.user_id, name: m.name ?? '', email: m.email ?? '', account: true, member: m, orgRole: m.role, projects: [] };
      by.set(m.user_id, x); if (x.email) byEmail.set(x.email.toLowerCase(), x);
    }
    for (const u of aud?.users ?? []) {
      const x = by.get(u.id) ?? (u.email ? byEmail.get(u.email.toLowerCase()) : undefined);
      if (x) { x.projects = u.projects; x.account = u.account; x.name ||= u.name; x.email ||= u.email; continue; }
      by.set(u.id, { id: u.id, name: u.name, email: u.email, account: u.account, orgRole: u.orgs.find((o) => o.org === handle)?.role ?? '', projects: u.projects });
    }
    const rank = (r: string) => (r === 'securityadmin' ? 0 : r === 'admin' ? 1 : r ? 2 : 3);
    return [...by.values()].sort((x, y) => rank(x.orgRole) - rank(y.orgRole) || (x.name || x.email).localeCompare(y.name || y.email));
  });
  const rows = $derived.by(() => {
    const q = fQ.trim().toLowerCase();
    return everyone.filter((u) => {
      if (q && ![u.email, u.name, u.id].some((x) => (x ?? '').toLowerCase().includes(q))) return false;
      if (fRole === '-' ? !!u.orgRole : fRole && u.orgRole !== fRole) return false;
      if (fProject) {
        const here = u.projects.find((p) => p.namespace === fProject);
        if (!here) return false;
        if (fCap && !here.capacities.includes(fCap as any)) return false;
      }
      return true;
    }).map((u) => ({ ...u, here: fProject ? u.projects.find((p) => p.namespace === fProject) : undefined }));
  });
  const fProjectInfo = $derived(aud?.projects.find((p) => p.namespace === fProject));
  const outsiders = $derived(everyone.filter((u) => !u.orgRole).length);

  let addWho = $state(''); let addRole = $state<OrgRole>('member');
  async function addMember() {
    if (!org) return;
    const q = addWho.trim().toLowerCase();
    const u = directory.find((d) => d.email.toLowerCase() === q || d.id === addWho.trim());
    if (!u) { say('Pick someone from the list: members are matched by account, not by a typed email.'); return; }
    busy = 'member';
    const r = await orgsApi.setMember(org.handle, { user_id: u.id, email: u.email, name: u.name }, addRole);
    busy = '';
    if (!r.ok) { say(r.error ?? 'Could not add them.'); return; }
    addWho = ''; say(`Added ${u.name || u.email} as ${ROLE_LABEL[addRole].toLowerCase()}.`, true);
    await refreshOrg(); void loadProjects(true);
  }
  async function setRole(m: { user_id: string; email?: string; name?: string }, role: OrgRole) {
    if (!org) return;
    busy = 'member';
    const r = await orgsApi.setMember(org.handle, { user_id: m.user_id, email: m.email, name: m.name }, role);
    busy = '';
    if (!r.ok) say(r.error ?? 'Could not change the role.'); else say(`${m.name || m.email || m.user_id} is now ${ROLE_LABEL[role].toLowerCase()}.`, true);
    await refreshOrg(); void loadProjects(true);
  }
  async function removeMember(m: { user_id: string; email?: string; name?: string }) {
    if (!org || !confirm(`Remove ${m.name || m.email || m.user_id} from ${org.handle}?`)) return;
    busy = 'member';
    const r = await orgsApi.removeMember(org.handle, m.user_id);
    busy = '';
    if (!r.ok) say(r.error ?? 'Could not remove them.'); else say(`Removed ${m.name || m.email || m.user_id}.`, true);
    await refreshOrg(); void loadProjects(true);
  }

  // ── ID-10: connect the org's own provider; SCIM keeps its people in step ──
  let idp = $state<OrgIdp | null>(null);
  let conn = $state({ issuer: '', client_id: '', client_secret: '', groups_claim: 'groups', domains: '' });
  let scimToken = $state<{ token: string; base_url: string } | null>(null);
  async function loadIdp() {
    if (!org) return;
    const r = await orgPolicy.idp(org.handle);
    if (!r.ok) return;
    idp = r.data!;
    const s = idp.sso;
    conn = { issuer: s?.issuer ?? '', client_id: s?.client_id ?? '', client_secret: '', groups_claim: s?.groups_claim ?? 'groups', domains: (s?.domains ?? []).join(', ') };
  }
  async function connectIdp() {
    if (!org) return;
    busy = 'idp';
    const r = await orgPolicy.connectIdp(org.handle, { issuer: conn.issuer.trim(), client_id: conn.client_id.trim(), client_secret: conn.client_secret || undefined,
      groups_claim: conn.groups_claim.trim() || undefined, domains: conn.domains.split(',').map((d) => d.trim().toLowerCase()).filter(Boolean) });
    busy = '';
    if (!r.ok) { say(r.error ?? 'Not connected.'); return; }
    say(`${org.handle} now signs in through its own provider (${r.data!.issuer}).`, true);
    await refreshOrg(); await loadIdp();
  }
  async function disconnectIdp() {
    if (!org) return;
    busy = 'idp';
    const r = await orgPolicy.disconnectIdp(org.handle);
    busy = '';
    if (!r.ok) { say(r.error ?? 'Not disconnected.'); return; }
    say(`Sign-in through ${org.handle}'s own provider is off.`, true);
    await refreshOrg(); await loadIdp();
  }
  async function makeScimToken() {
    if (!org) return;
    busy = 'scim';
    const r = await orgPolicy.newScimToken(org.handle);
    busy = '';
    if (!r.ok) { say(r.error ?? 'No token made.'); return; }
    scimToken = { token: r.data!.token, base_url: `${API_BASE.replace(/\/$/, '')}${r.data!.base_url}` };
    await loadIdp();
  }
  async function scimOff() {
    if (!org) return;
    busy = 'scim';
    const r = await orgPolicy.scimOff(org.handle);
    busy = '';
    if (!r.ok) { say(r.error ?? 'SCIM not turned off.'); return; }
    scimToken = null; say('SCIM is off: the token no longer works.', true); await loadIdp();
  }
  $effect(() => { if (tab === 'signin' && org && canAdmin) untrack(() => void loadIdp()); });

  // GOV-5: which groups get which org role is the policy file access/org.yaml.
  const ORG_ACCESS = 'access/org.yaml';
  const orgAccess = $derived.by(() => {
    const f = (files?.files ?? []).find((x) => x.path === ORG_ACCESS || x.path === 'access/org.yml');
    if (!f) return null;
    try { return { f, y: (YAML.parse(f.content) ?? {}) as { groups?: Record<string, string>; default?: string } }; } catch { return { f, y: {} }; }
  });
  async function newOrgAccess() {
    busy = 'oa';
    const r = await orgPolicy.save(handle, ORG_ACCESS, `kind: a3t/org-access\ndescription: Which of ${handle}'s sign-in groups get which org role.\n# group: admin, securityadmin or member\ngroups:\n  ${handle}-admins: admin\n# Everyone else who signs in through the org's provider (leave out: no access).\ndefault: member\n`);
    busy = '';
    if (!r.ok) { say(((r as any).detail?.errors ?? [r.error]).join('; ')); return; }
    await reloadFiles();
    openFileInStudio(ORG_ACCESS);
  }

  // ── Policy files ──
  let openFile = $state('');
  async function reloadFiles() {
    const f = await orgPolicy.files(handle);
    files = f.ok ? f.data ?? null : files; filesErr = f.ok ? '' : (f.error ?? `HTTP ${f.status}`);
    pickFrom(files);
  }
  // ── New file: a working example of its kind, saved (it must pass the check),
  // then opened in Studio to change.
  let nDir = $state<PolicyDir>('policies');
  let nName = $state('');
  let nProject = $state('');
  async function newFile() {
    const name = nName.trim();
    if (!/^[A-Za-z0-9_-]{1,80}$/.test(name)) { say('Name it with letters, digits, - and _.'); return; }
    const path = `${nDir}/${name}.yaml`;
    if (files?.files.some((f) => f.path === path)) { say(`${path} already exists.`); return; }
    let text = POLICY_STARTERS[nDir](name, handle);
    if (nDir === 'access') {
      if (!nProject) { say('Pick the project it gives access to.'); return; }
      text = text.replace(`project: ${handle}-<project>`, `project: ${nProject}`).replace(/^users:\n  someone@example.com: Viewer\n/m, '');
    }
    busy = 'new';
    const r = await orgPolicy.save(handle, path, text);
    busy = '';
    if (!r.ok) { const errs = ((r as any).detail?.errors ?? []) as string[]; say(`Not created: ${r.error ?? `HTTP ${r.status}`}${errs.length ? ` (${errs.join('; ')})` : ''}`); return; }
    nName = '';
    await reloadFiles();
    openInStudio('policy', `org:${handle}`, path);
  }
  const openFileInStudio = (path: string) => openInStudio('policy', `org:${handle}`, path);

  // Which files are enforced: ticked here, applied together. Turning any off
  // needs the org's handle typed (it drops rules); turning them on doesn't.
  let picked = $state<Record<string, boolean>>({});
  let stopName = $state('');
  let enforceErrors = $state<string[]>([]);
  function pickFrom(f: OrgPolicyFiles | null) {
    picked = Object.fromEntries((f?.files ?? []).map((x) => [x.path, x.enforced !== false]));
    stopName = ''; enforceErrors = [];
  }
  const turningOff = $derived((files?.files ?? []).filter((f) => f.enforced !== false && !picked[f.path]).map((f) => f.path));
  const turningOn = $derived((files?.files ?? []).filter((f) => f.enforced === false && picked[f.path]).map((f) => f.path));
  const enforcedNow = $derived((files?.files ?? []).filter((f) => f.enforced !== false).length);
  const pickAll = (on: boolean) => { picked = Object.fromEntries((files?.files ?? []).map((x) => [x.path, on])); };
  async function applyEnforced() {
    if (turningOff.length && stopName.trim() !== handle) return;
    busy = 'files'; enforceErrors = [];
    const want = (files?.files ?? []).filter((f) => picked[f.path]).map((f) => f.path);
    const r = await orgPolicy.setEnforced(handle, want);
    busy = '';
    if (!r.ok) {
      enforceErrors = ((r as any).detail?.errors ?? []) as string[];
      say(`Not changed: ${r.error ?? `HTTP ${r.status}`}.`);
      return;
    }
    if (r.data?.pending) { say(r.data.message ?? 'Held for a second approver.', true); await loadChanges(); return; }
    say(`${want.length} of ${files?.files.length ?? 0} file${files?.files.length === 1 ? '' : 's'} enforced.`, true);
    await reloadFiles();
  }
  const KIND_LABEL: Record<string, string> = { policy: 'rule', role: 'role', access: 'access' };
  const when = (s: string) => { const t = Date.parse(s); return Number.isFinite(t) ? new Date(t).toLocaleString(undefined, { month: 'short', day: 'numeric', hour: '2-digit', minute: '2-digit' }) : '—'; };

  const ALL_STEPS = [
    { id: 'overview', label: 'Overview', icon: '<rect x="2" y="2" width="12" height="12" rx="2"/><path d="M5 6h6M5 9h4"/>', hint: "The org's people at a glance, and who looks after what" },
    { id: 'signin', label: 'Single sign-on', icon: '<rect x="3" y="7" width="10" height="7" rx="1.5"/><path d="M5.5 7V5a2.5 2.5 0 015 0v2"/>', hint: "Connect the org's identity provider so its people sign in with their work account; SCIM keeps them in step" },
    { id: 'members', label: 'Org members', icon: '<circle cx="6" cy="5" r="2.2"/><circle cx="11.5" cy="6" r="1.8"/><path d="M2 13a4 4 0 018 0M9.5 12.5a3 3 0 015 0"/>', hint: 'Its people and groups, the role each holds, and who else reaches its projects' },
    { id: 'reviews', label: 'Access reviews', icon: '<path d="M3 2h10v12H3z"/><path d="M5.5 6l1.5 1.5L10.5 4M5.5 10.5h5"/>', hint: 'Confirm or revoke, item by item, who holds which role and key' },
    { id: 'roles', label: 'Roles', icon: '<circle cx="5" cy="5" r="2"/><circle cx="11" cy="5" r="2"/><path d="M1.5 13a3.5 3.5 0 017 0M7.5 13a3.5 3.5 0 017 0"/>', hint: 'The roles the org has and what each may do' },
    { id: 'rules', label: 'Rules', icon: '<path d="M3 4h10M3 8h10M3 12h6"/>', hint: 'Every rule of the org, one row each: open one to read, try or change it, or add a new one' },
    { id: 'settings', label: 'Settings', icon: '<circle cx="8" cy="8" r="2.2"/><path d="M8 1.5v2M8 12.5v2M1.5 8h2M12.5 8h2M3.4 3.4l1.4 1.4M11.2 11.2l1.4 1.4M3.4 12.6l1.4-1.4M11.2 4.8l1.4-1.4"/>', hint: 'Who may change the org\'s policies and identity, and who approves a change' },
    { id: 'files', label: 'Files', icon: '<path d="M4 1.5h5l3 3v10H4z"/><path d="M9 1.5v3h3M6 8h4M6 10.5h4"/>', hint: 'Its roles, rules and access, as files: switch each on or off, edit in Studio' },
    { id: 'changes', label: 'Changes', icon: '<circle cx="5" cy="8" r="2"/><circle cx="11" cy="8" r="2"/><path d="M7 8h2"/>', hint: 'A second approver for policy changes, and the changes waiting for one' },
  ];
  const IDENTITY_TABS = ['overview', 'signin', 'members', 'roles', 'reviews'];
  const STEPS = $derived(section === 'policy' ? ['rules', 'changes', 'settings', 'files'].map((id) => ALL_STEPS.find((x) => x.id === id)!) : ALL_STEPS.filter((x) => IDENTITY_TABS.includes(x.id)));
  // A tab from the other page (an old link) lands on this page's first tab.
  $effect(() => { if (!STEPS.some((x) => x.id === tab)) tab = STEPS[0].id; });

  // ── Shadow mode and replay (GOV-2) ──
  let replays = $state<Record<string, ReplayResult>>({});
  async function toggleShadow(path: string, on: boolean) {
    busy = `sh:${path}`; const r = await orgPolicy.setShadow(handle, path, on); busy = '';
    if (!r.ok) { say(`Not changed: ${((r as any).detail?.errors ?? [r.error]).join('; ')}`); return; }
    say(on ? `${path} runs in shadow mode: what it would refuse goes to the decision log (Shadow), nothing is refused.` : `${path} is off.`, true);
    await reloadFiles();
  }
  async function replayFile(path: string, switchOn: boolean) {
    busy = `rp:${path}`;
    const r = await orgPolicy.replay(handle, switchOn ? { switch_on: [path] } : { removed: [path] });
    busy = '';
    if (!r.ok) { say(`Replay failed: ${((r as any).detail?.errors ?? [r.error]).join('; ')}`); return; }
    replays = { ...replays, [path]: r.data! };
  }

  // ── Describe a rule (GOV-1) ──
  let pDesc = $state('');
  let pBusy = $state(false);
  let pErr = $state('');
  let pDraft = $state<PolicyDraft | null>(null);
  let pReplay = $state<ReplayResult | null>(null);
  let pAbort: AbortController | null = null;
  async function askPolicy() {
    pBusy = true; pErr = ''; pDraft = null; pReplay = null; pAbort = new AbortController();
    const roles = ['Admin', 'Operator', 'Viewer', ...orgRoles.map((r) => String(r.y.name ?? '')).filter(Boolean)];
    const r = await draftPolicy(handle, pDesc.trim(), { projects: files?.projects ?? [], files: (files?.files ?? []).map((f) => f.path), roles }, pAbort.signal);
    pBusy = false;
    if (!r.ok) { pErr = r.error; return; }
    pDraft = r.draft;
  }
  async function recheck() {
    if (!pDraft) return;
    busy = 'pchk';
    const errors = await checkDraft(handle, pDraft.files);
    busy = '';
    pDraft = { ...pDraft, ok: errors.length === 0, errors };
  }
  async function replayDraft() {
    if (!pDraft) return;
    busy = 'prp';
    const r = await orgPolicy.replay(handle, { files: pDraft.files.map((f) => ({ path: f.path, content: f.yaml })) });
    busy = '';
    if (!r.ok) { pErr = ((r as any).detail?.errors ?? [r.error]).join('; '); return; }
    pReplay = r.data!;
  }
  async function saveDraft() {
    if (!pDraft) return;
    busy = 'psave'; pErr = '';
    const saved: string[] = [], held: string[] = [];
    for (const f of pDraft.files) {
      const r = await orgPolicy.save(handle, f.path, f.yaml);
      if (!r.ok) { busy = ''; pErr = `${f.path}: ${((r as any).detail?.errors ?? [r.error]).join('; ')}`; await reloadFiles(); return; }
      ((r.data as any)?.pending ? held : saved).push(f.path);
    }
    busy = '';
    say([saved.length ? `Saved ${saved.join(', ')}.` : '', held.length ? `Held for a second approver: ${held.join(', ')}.` : ''].filter(Boolean).join(' '), true);
    pDraft = null; pDesc = ''; pReplay = null;
    await reloadFiles();
  }

  // ── Changes: a second approver (GOV-3) ──
  let fourEyes = $state(false);
  let changes = $state<PolicyChange[]>([]);
  let changeNote = $state<Record<string, string>>({});
  let changeErr = $state<Record<string, string[]>>({});
  let settings = $state<OrgSettings | null>(null);
  async function loadChanges() {
    const [r, k, st] = await Promise.all([orgPolicy.changes(handle), orgPolicy.retention(handle), orgPolicy.settings(handle)]);
    if (r.ok) { fourEyes = !!r.data?.four_eyes; changes = r.data?.changes ?? []; }
    if (st.ok) settings = st.data ?? null;
    if (k.ok) { keepDays = k.data?.retention_days ?? 0; hold = !!k.data?.legal_hold; keepPick = keepDays; }
  }
  // ── AUD-6: how long its decisions are kept, and a legal hold ──
  let keepDays = $state(0);
  let keepPick = $state(0);
  let hold = $state(false);
  // AUD-4: check its log for gaps and edits.
  let checked = $state<AuditVerify | null>(null);
  async function checkLog() {
    if (!org) return;
    busy = 'verify';
    const r = await auditApi.verify(org.id, 30);
    busy = '';
    if (!r.ok) { say(`Not checked: ${r.error ?? `HTTP ${r.status}`}.`); return; }
    checked = r.data!;
  }
  async function saveRetention(body: { retention_days?: number; legal_hold?: boolean }) {
    busy = 'ret';
    const r = await orgPolicy.setRetention(handle, body);
    busy = '';
    if (!r.ok) { say(`Not changed: ${r.error ?? `HTTP ${r.status}`}.`); return; }
    say(r.data?.pending ? (r.data.message ?? 'Held for a second approver.') : 'Saved.', true);
    await loadChanges();
  }
  $effect(() => { if ((tab === 'changes' || tab === 'settings' || tab === 'rules') && org) untrack(() => void loadChanges()); });
  async function saveGovernance(g: Partial<OrgGovernance>) {
    busy = 'gov'; const r = await orgPolicy.setGovernance(handle, g); busy = '';
    if (!r.ok) { say(`Not changed: ${r.error ?? `HTTP ${r.status}`}.`); return; }
    say('Saved: it applies at once.', true);
    await loadChanges();
  }

  // ── Rules (one row per rule) and the rule editor ──
  let ruleTarget = $state<{ path: string; index: number } | null | undefined>(undefined);
  let perms = $state<{ id: string; label: string }[]>([]);
  $effect(() => { if (tab === 'rules' && !perms.length) untrack(() => void securityCatalogue().then((c) => { if (c.ok) perms = (c.data?.permissions ?? []).map((p: any) => ({ id: p.id, label: p.label })); })); });
  $effect(() => { if (tab !== 'rules') untrack(() => (ruleTarget = undefined)); });
  const ruleRoles = $derived(['Admin', 'Operator', 'Viewer', ...(files?.files ?? []).filter((f) => f.kind === 'role').map((f) => String(parse(f.content)?.name ?? '')).filter(Boolean)]);
  async function ruleSaved(m: string) { ruleTarget = undefined; say(m, true); await reloadFiles(); }
  const pendingChanges = $derived(changes.filter((c) => c.status === 'pending'));
  async function toggleFourEyes() {
    busy = 'four'; const r = await orgPolicy.setFourEyes(handle, !fourEyes); busy = '';
    if (!r.ok) { say(`Not changed: ${r.error ?? `HTTP ${r.status}`}.`); return; }
    say(r.data?.pending ? (r.data.message ?? 'Held for a second approver.') : `The second approver is ${!fourEyes ? 'on' : 'off'}.`, true);
    await loadChanges();
  }
  async function decideChange(c: PolicyChange, yes: boolean) {
    busy = c.id; changeErr = { ...changeErr, [c.id]: [] };
    const r = yes ? await orgPolicy.approve(handle, c.id, changeNote[c.id] ?? '') : await orgPolicy.reject(handle, c.id, changeNote[c.id] ?? '');
    busy = '';
    if (!r.ok) { changeErr = { ...changeErr, [c.id]: ((r as any).detail?.errors ?? [r.error ?? `HTTP ${r.status}`]) as string[] }; return; }
    say(yes ? `Approved and applied: ${c.summary}.` : `Rejected: ${c.summary}.`, true);
    await Promise.all([loadChanges(), reloadFiles()]);
  }

  // ── Roles & groups: read from the org's role and access files (and sign-in) ──
  const BASELINE = [
    { name: 'Admin', what: 'builds and changes flows, agents and resources' },
    { name: 'Operator', what: 'runs them' },
    { name: 'Viewer', what: 'reads them' },
  ];
  const parse = (t: string): any => { try { return YAML.parse(t) ?? {}; } catch { return {}; } };
  const orgRoles = $derived((files?.files ?? []).filter((f) => f.kind === 'role').map((f) => ({ f, y: parse(f.content) })));
  const accessFiles = $derived((files?.files ?? []).filter((f) => f.kind === 'access').map((f) => ({ f, y: parse(f.content) })));
  const pairs = (m: any): [string, string][] => (m && typeof m === 'object' ? Object.entries(m).map(([k, v]) => [k, String(v)] as [string, string]) : []);
  const savedVersion = (v: string) => (/^v\d+$/.test(v) ? v : 'imported');

  const memberCount = (r: string) => (org?.members ?? []).filter((m) => m.role === r).length;
</script>

<div class="panel-layout">
  <PropertiesTree icon={section === 'policy' ? 'PO' : 'ID'} color="#5530a8" name={section === 'policy' ? 'Policies' : 'Identity'} meta={`org ${org?.display_name ?? handle} · ${section === 'policy' ? 'what is allowed' : 'who people are'}`} nodes={[]} steps={STEPS} activeNode={tab} onselect={(id) => (tab = id)} />
  <div class="panel-col"><div class="panel-content"><div class="pf">
    <WizardHead step={STEPS.find((s) => s.id === tab)} />
    <section class="agt">
    {#if err}<div class="note bad">{err}</div>
    {:else if !org}<p class="dim">Reading {handle}…</p>
    {:else}
      {#if msg}<div class="note" class:bad={!msgOk} class:okn={msgOk}>{msg}</div>{/if}
      {#if broken}<div class="note bad"><b>Its policy files no longer compile</b> (since {new Date(broken.since).toLocaleString()}), so its projects ({broken.closed.join(', ') || 'none'}) refuse every call until they are fixed in Policy files.
        <ul>{#each broken.errors as e (e)}<li class="mono">{e}</li>{/each}</ul></div>{/if}

      {#if tab === 'rules'}
        {#if ruleTarget === undefined}
          <RulesList org={handle} {files} projects={files?.projects ?? []} {perms} canEdit={managesFiles} onopen={(t) => (ruleTarget = t)} />
        {:else}
          {#key ruleTarget ? `${ruleTarget.path}#${ruleTarget.index}` : 'new'}
            <RuleEditor org={handle} {files} projects={files?.projects ?? []} roles={ruleRoles} {perms} canEdit={managesFiles} target={ruleTarget} timezone={settings?.governance.timezone ?? null} onclose={() => (ruleTarget = undefined)} onsaved={ruleSaved} />
          {/key}
        {/if}
      {:else if tab === 'settings'}
        <GovernSettings org={handle} {settings} busy={busy === 'four' || busy === 'gov' ? busy : ''} onFourEyes={toggleFourEyes} onSave={saveGovernance} />
      {:else if tab === 'overview'}
        <div class="card">
          <div class="card-head"><h3>1 · {org.display_name}</h3><span>you are {org.my_role ? ROLE_LABEL[org.my_role].toLowerCase() : infosec ? 'platform InfoSec' : '—'} here</span></div>
          <div class="tiles">
            <div class="tile"><span class="v">{org.members.length}</span><span class="k">members</span></div>
            <div class="tile"><span class="v">{memberCount('admin')}</span><span class="k">admins</span></div>
            <div class="tile"><span class="v">{memberCount('securityadmin')}</span><span class="k">security admins</span></div>
            <div class="tile"><span class="v">{aud?.projects.length ?? '…'}</span><span class="k">projects</span></div>
          </div>
          <ul class="rows">
            <li><span class="lbl">Members</span><span class="val">{org.members.length} · {memberCount('admin')} admin{memberCount('admin') === 1 ? '' : 's'}, {memberCount('securityadmin')} security admin{memberCount('securityadmin') === 1 ? '' : 's'}{outsiders ? ` · ${outsiders} from outside the org reach its projects` : ''}{pairs(orgAccess?.y.groups).length ? ` · ${pairs(orgAccess?.y.groups).length} group${pairs(orgAccess?.y.groups).length === 1 ? '' : 's'}` : ''}</span><button class="btn2" onclick={() => (tab = 'members')}>{canAdmin ? 'Manage' : 'See'}</button></li>
            <li><span class="lbl">Single sign-on</span><span class="val">{org.sso?.idp_alias ? `through ${org.sso.idp_alias}${org.sso.domains?.length ? ` for ${org.sso.domains.join(', ')}` : ''}` : "A3T's own sign-in"}</span><button class="btn2" onclick={() => (tab = 'signin')}>{canAdmin ? 'Set up' : 'See'}</button></li>
            <li><span class="lbl">Roles</span><span class="val">{BASELINE.length} baseline{orgRoles.length ? ` + ${orgRoles.length} of its own (${orgRoles.map((r) => r.y.name ?? r.f.path).join(', ')})` : ''}</span><button class="btn2" onclick={() => (tab = 'roles')}>See</button></li>
          </ul>
        </div>
        <div class="card">
          <div class="card-head"><h3>2 · Who looks after what</h3><span>the server checks each of these</span></div>
          <ul class="rows wide">
            <li><span class="lbl">Org admin</span><span class="val">members, sign-in and policy files; builds in every project of the org</span><span></span></li>
            <li><span class="lbl">Org security admin</span><span class="val">reads members, sign-in and who reaches each project; manages policy files; reads every project</span><span></span></li>
            <li><span class="lbl">SuperAdmin</span><span class="val">everything, every org</span><span></span></li>
            <li><span class="lbl">SecurityAdmin</span><span class="val">reads every org's people and sign-in; manages policy files; the platform's sign-in realm</span><span></span></li>
          </ul>
        </div>

      {:else if tab === 'members'}
        <div class="card">
          <div class="card-head"><h3>1 · People</h3><span>{canAdmin ? 'you can change their roles' : 'read only: its admins change them'}</span></div>
          <div class="uf">
            <input class="tin grow" type="search" bind:value={fQ} placeholder="Find by name or email" aria-label="Find a person" />
            <select class="tin" bind:value={fRole} aria-label="Role in the org">
              <option value="">Everyone</option>
              {#each ROLES as r}<option value={r}>{ROLE_LABEL[r]}s</option>{/each}
              <option value="-">Not in the org</option>
            </select>
            {#if aud?.projects.length}<select class="tin" bind:value={fProject} onchange={() => (fCap = '')} aria-label="Reaches project">
              <option value="">Any project</option>{#each aud.projects as p (p.namespace)}<option value={p.namespace}>{p.namespace}</option>{/each}
            </select>{/if}
            {#if fProject}<select class="tin" bind:value={fCap} aria-label="Their capacity there"><option value="">Any capacity</option>{#each Object.keys(CAP_LABEL) as c}<option value={c}>{CAP_LABEL[c]}</option>{/each}</select>{/if}
          </div>
          <p class="dim small">{rows.length} of {everyone.length}: {org.members.length} in the org{outsiders ? `, ${outsiders} outside it who reach its projects` : ''}{fProject ? ` · ${rows.length} reach ${fProject}` : ''}{fProjectInfo?.everyone ? ', and everyone signed in can run it' : ''}{fProjectInfo?.private ? ' (private)' : ''}.{audErr ? ` Projects reached could not be read: ${audErr}` : ''}</p>
          <ul class="olist">
            {#each rows as u (u.id)}
              <li class="prow">
                <button class="uwho" onclick={() => (openRow = openRow === u.id ? '' : u.id)} aria-expanded={openRow === u.id}>
                  <span class="mname"><b>{u.name || u.email || 'Unnamed account'}</b>{#if u.member?.source === 'sso'}<span class="chip" title="From the org's sign-in: its groups set the role at every sign-in">SSO</span>{/if}</span>
                  <small>{[u.name ? u.email : '', !u.name && !u.email ? u.id : '', !u.account ? 'no sign-in account' : ''].filter(Boolean).join(' · ')}</small>
                </button>
                <span class="uchips">
                  {#if u.here}{#each u.here.capacities as c}<span class="chip here">{CAP_LABEL[c] ?? c}</span>{/each}<span class="dim small">{ACCESS_WORD[u.here.access] ?? u.here.access}</span>
                  {:else}<span class="dim small">{u.projects.length} project{u.projects.length === 1 ? '' : 's'}</span>{/if}
                </span>
                {#if u.member && canAdmin}
                  <select class="tin" value={u.orgRole} disabled={!!busy} onchange={(e) => setRole(u.member!, e.currentTarget.value as OrgRole)} aria-label="Role in the org">
                    {#each ROLES as r}<option value={r}>{ROLE_LABEL[r]}</option>{/each}
                  </select>
                  <button class="btn2 quiet" disabled={!!busy} onclick={() => removeMember(u.member!)}>Remove</button>
                {:else if u.orgRole}<span class="chip" class:sec={u.orgRole !== 'member'}>{ROLE_LABEL[u.orgRole] ?? u.orgRole}</span><span></span>
                {:else}<span class="chip out">outside the org</span>{#if canAdmin && u.account}<button class="btn2 quiet" disabled={!!busy} onclick={() => setRole({ user_id: u.id, email: u.email, name: u.name }, 'member')}>Add</button>{:else}<span></span>{/if}{/if}
                {#if openRow === u.id}
                  <div class="udetail">
                    <ul class="uprojects">{#each u.projects as p (p.namespace)}
                      <li><button class="link" onclick={() => { fProject = p.namespace; fCap = ''; openRow = ''; }}>{p.namespace}</button>
                        <span>{p.capacities.map((c) => CAP_LABEL[c] ?? c).join(', ')}</span><span class="dim">{ACCESS_WORD[p.access] ?? p.access}</span></li>
                    {:else}<li class="dim">reaches none of its projects</li>{/each}</ul>
                  </div>
                {/if}
              </li>
            {:else}<li class="dim">{everyone.length ? 'Nobody matches.' : 'No members yet.'}</li>{/each}
          </ul>
          {#if canAdmin}
            <div class="uf">
              <input class="tin grow" bind:value={addWho} list="org-sec-users" placeholder="Add someone: name or email" onkeydown={(e) => { if (e.key === 'Enter') addMember(); }} aria-label="Person to add" />
              <datalist id="org-sec-users">{#each directory as u}<option value={u.email || u.id}>{u.name}{u.source === 'sso' ? ' (SSO)' : ''}</option>{/each}</datalist>
              <select class="tin" bind:value={addRole} aria-label="As">{#each ROLES as r}<option value={r}>{ROLE_LABEL[r]}</option>{/each}</select>
              <button class="btn2 primary" onclick={addMember} disabled={!!busy || !addWho.trim()}>Add</button>
            </div>
            <p class="dim small">Members see the org's projects that aren't private; admins build in all of them; security admins read all of them and look after the org's security. People who sign in through the org's provider are added at sign-in.</p>
          {/if}
        </div>

        <div class="card">
          <div class="card-head"><h3>2 · Groups</h3><span>groups from the org's sign-in, and the role each gives</span></div>
          <h4 class="sub">In the org, at sign-in</h4>
          {#if org.sso?.idp_alias}
            <ul class="olist">
              {#each pairs(orgAccess?.y.groups) as [g, r] (g)}<li class="rrow"><span class="mono">{g}</span><span>{ROLE_LABEL[r] ?? r}</span><span></span></li>{/each}
              <li class="rrow"><span class="dim">anyone else from {org.sso.idp_alias}</span><span>{orgAccess?.y.default ? `${ROLE_LABEL[orgAccess.y.default] ?? orgAccess.y.default}` : 'no access'}</span><button class="lnk2" onclick={() => (tab = 'signin')}>Single sign-on →</button></li>
            </ul>
          {:else}<p class="dim">No identity provider connected, so no groups: people are added above.</p>{/if}
          {#if org.sso?.idp_alias}
            {#if orgAccess}<p class="dim small">From the policy file <button class="lnk2 mono" onclick={() => openFileInStudio(orgAccess!.f.path)}>{orgAccess.f.path}</button>{orgAccess.f.enforced === false ? ' (switched off: nobody gets an org role at sign-in)' : ''}: versioned, checked and approved like the org's other policy files.</p>
            {:else}<p class="dim small">No <span class="mono">{ORG_ACCESS}</span> yet: people who sign in get no org role until it says which groups get which.</p>
              {#if managesFiles}<div class="acts"><button class="btn2" disabled={!!busy} onclick={newOrgAccess}>Create {ORG_ACCESS}</button></div>{/if}{/if}
          {/if}
          <h4 class="sub">On its projects</h4>
          {#each accessFiles as a (a.f.path)}
            <div class="proj"><span class="mono">{a.y.project ?? '?'}</span>{#if !a.f.enforced}<span class="dim"> · switched off</span>{/if}<button class="lnk2" onclick={() => openFileInStudio(a.f.path)}>Edit in Studio</button></div>
            <ul class="olist">
              {#each pairs(a.y.groups) as [g, r] (g)}<li class="rrow"><span class="mono">{g}</span><span>{r}</span><span class="dim">group</span></li>{/each}
              {#each pairs(a.y.users) as [u, r] (u)}<li class="rrow"><span class="mono">{u}</span><span>{r}</span><span class="dim">person</span></li>{/each}
            </ul>
          {:else}<p class="dim">No group mappings on its projects: people hold project roles as project members (Projects › Access).</p>{/each}
          {#if managesFiles}<p class="dim small">Group mappings are access files: create one under Policies › Files.</p>{/if}
        </div>

      {:else if tab === 'signin'}
        <div class="card">
          <div class="card-head"><h3>1 · Your identity provider</h3><span>{idp?.sso?.issuer ? `connected: ${idp.sso.issuer}` : org.sso?.idp_alias ? `linked: ${org.sso.idp_alias}` : 'not connected: people use A3T\'s own sign-in'}</span></div>
          <p class="dim">Connect the org's own provider (Entra ID, Okta, Google, Keycloak… over OpenID Connect) and its people sign in with their work account. Anyone with an email in its domains lands in {org.handle} only, with the org role their groups give them (see Org members › Groups), checked at every sign-in. Members added by hand are not changed.</p>
          {#if canAdmin}
            <p class="dim">At your provider, register A3T as an app with the redirect URI <span class="mono">{idp?.redirect_uri ?? '…'}</span>, then fill in its details.</p>
            <div class="fields">
              <label class="fld">Issuer<input class="mono" bind:value={conn.issuer} placeholder="https://login.example.com/realms/acme" /></label>
              <label class="fld">Client id<input class="mono" bind:value={conn.client_id} placeholder="a3t" /></label>
              <label class="fld">Client secret<input class="mono" type="password" bind:value={conn.client_secret} placeholder={idp?.sso?.issuer ? 'unchanged' : 'from your provider'} autocomplete="off" /></label>
              <label class="fld">Groups claim<input class="mono" bind:value={conn.groups_claim} placeholder="groups" /></label>
              <label class="fld">Email domains<input bind:value={conn.domains} placeholder="acme.com, acme.co.uk" /></label>
            </div>
            <div class="acts">
              <button class="btn2 primary" disabled={!!busy || !conn.issuer.trim() || !conn.client_id.trim()} onclick={connectIdp}>{busy === 'idp' ? 'Connecting…' : idp?.sso?.issuer ? 'Save' : 'Connect'}</button>
              {#if idp?.sso?.issuer || org.sso?.idp_alias}<button class="btn2 quiet" disabled={!!busy} onclick={disconnectIdp}>Disconnect</button>{/if}
            </div>
          {:else}
            <ul class="olist">
              <li class="rrow"><span class="lbl">Provider</span><span class="mono">{idp?.sso?.issuer ?? org.sso?.idp_alias ?? '—'}</span><span></span></li>
              <li class="rrow"><span class="lbl">Email domains</span><span>{(org.sso?.domains ?? []).join(', ') || '—'}</span><span></span></li>
            </ul>
            <p class="dim small">Read only: {org.handle}'s admins change it.</p>
          {/if}
        </div>
        {#if canAdmin}
          <div class="card">
            <div class="card-head"><h3>2 · Provisioning (SCIM)</h3><span>{idp?.scim.on ? `on since ${idp.scim.since?.slice(0, 10) ?? '—'} · ${idp.scim.users} people, ${idp.scim.deactivated} deactivated` : 'off · optional'}</span></div>
            <p class="dim">Optional. Your provider tells A3T when someone joins, changes or leaves, instead of A3T learning it only when they sign in. Someone deactivated or removed there loses {org.handle} at once: their org membership, their roles on its projects and their sessions.</p>
            <h4 class="sub">To set it up</h4>
            <ol class="steps">
              <li>{#if idp?.scim.on}Your provider already holds a token (A3T can't show it again; replace it if you need a new one).{:else}Make a token below. It is shown only once, so copy it straight away.{/if}</li>
              <li>In your provider, open the app you registered for A3T and turn on provisioning (Entra ID: <i>Provisioning › Automatic</i>; Okta: <i>Provisioning › SCIM</i>).</li>
              <li>Paste the base URL as the tenant or connector URL, and the token as the secret or bearer token. Test the connection there and start provisioning.</li>
            </ol>
            <ul class="olist">
              <li class="rrow"><span class="lbl">Base URL</span><span class="mono">{idp ? `${API_BASE.replace(/\/$/, '')}${idp.scim.base_url}` : '…'}</span><span></span></li>
              {#if scimToken}<li class="rrow"><span class="lbl">Token</span><span class="mono">{scimToken.token}</span><span class="dim">shown once</span></li>{/if}
            </ul>
            <div class="acts">
              <button class="btn2" disabled={!!busy} onclick={makeScimToken}>{busy === 'scim' ? 'Making…' : idp?.scim.on ? 'Replace token' : 'Make a token'}</button>
              {#if idp?.scim.on}<button class="btn2 quiet" disabled={!!busy} onclick={scimOff}>Turn off</button>{/if}
            </div>
            {#if idp?.scim.on}<p class="dim small">Replace the token if it was lost or may have leaked. The old one stops working at once, so paste the new one into your provider straight away or provisioning stops.</p>{/if}
          </div>
        {/if}

      {:else if tab === 'reviews'}
        <AccessReviews org={handle} canReview={managesFiles} />

      {:else if tab === 'roles'}
        <div class="card">
          <div class="card-head"><h3>Roles</h3><span>what each role may do is in the policy files</span></div>
          <ul class="olist">
            {#each BASELINE as r (r.name)}<li class="rrow"><b>{r.name}</b><span class="dim">baseline · {r.what}</span><span></span></li>{/each}
            {#each orgRoles as r (r.f.path)}
              <li class="rrow"><b>{r.y.name ?? r.f.path}</b><span class="dim">{handle}'s own · {r.y.extends ?? '—'}{(r.y.remove ?? []).length ? ` without ${(r.y.remove ?? []).join(', ')}` : ''}{r.f.enforced ? '' : ' · switched off'}{r.y.description ? ` · ${r.y.description}` : ''}</span>
                <button class="lnk2" onclick={() => openFileInStudio(r.f.path)}>Edit in Studio</button></li>
            {/each}
          </ul>
        </div>

      {:else if tab === 'files'}
        {#if filesErr && !files}<div class="note bad">{filesErr}</div>{/if}
        <div class="card">
          <div class="card-head"><h3>1 · What they are</h3><span>files kept in A3T, every save a version</span></div>
          <p class="dim">An org's rules (<span class="mono">policies/</span>), its own roles (<span class="mono">roles/</span>) and who holds which role on its projects (<span class="mono">access/</span>), as YAML files. Open one to edit it in Studio: it's checked against the org's other files as you type, and a save that would break them is refused.</p>
          {#if managesFiles}
            <div class="uf">
              <select class="tin" bind:value={nDir} aria-label="Kind of file"><option value="policies">Rule (policies/)</option><option value="roles">Role (roles/)</option><option value="access">Access (access/)</option><option value="egress">Calls out (egress/)</option></select>
              <input class="tin grow mono" bind:value={nName} placeholder={nDir === 'policies' ? 'no-external-models' : nDir === 'roles' ? 'support-lead' : nDir === 'egress' ? 'allowed-hosts' : 'support-team'} aria-label="File name" onkeydown={(e) => { if (e.key === 'Enter') newFile(); }} />
              {#if nDir === 'access'}<select class="tin" bind:value={nProject} aria-label="Project"><option value="">Project…</option>{#each files?.projects ?? [] as p}<option value={p}>{p}</option>{/each}</select>{/if}
              <button class="btn2 primary" disabled={!!busy || !nName.trim()} onclick={newFile}>{busy === 'new' ? 'Creating…' : 'New file'}</button>
            </div>
            <p class="dim small">It starts as a working example of its kind and opens in Studio.</p>
          {/if}
        </div>
        {#if managesFiles}
          <div class="card">
            <div class="card-head"><h3>Describe a rule</h3><span>{POLICY_AGENT} writes it; you check and save it</span></div>
            <label class="fld full">The rule, in words<textarea rows="3" bind:value={pDesc} placeholder="Operators in acme-web may not delete flows. Agents need a person's OK before merging a pull request."></textarea></label>
            <div class="acts"><button class="btn2 primary" disabled={pBusy || pDesc.trim().length < 8} onclick={askPolicy}>{pBusy ? 'Writing and checking…' : 'Write it'}</button>{#if pBusy}<button class="btn2 quiet" onclick={() => pAbort?.abort()}>Stop</button>{/if}</div>
            {#if pErr}<div class="note bad">{pErr}</div>{/if}
            {#if pDraft}
              {#if pDraft.assumptions.length}<p class="dim">Assumed: {pDraft.assumptions.join('; ')}.</p>{/if}
              {#each pDraft.files as f, i (i)}
                <p class="dim">{f.explanation}</p>
                <label class="fld">File<input class="mono" bind:value={f.path} /></label>
                <textarea class="cedar" rows="10" spellcheck="false" bind:value={f.yaml}></textarea>
              {/each}
              {#if pDraft.ok}<div class="note okn">✓ {pDraft.files.length === 1 ? 'It works' : `The ${pDraft.files.length} files work`} with {handle}'s other files{pDraft.tries > 1 ? ` (asked again ${pDraft.tries - 1}×: the first ${pDraft.tries === 2 ? 'draft' : 'drafts'} didn't check)` : ''}. Nothing is enforced until you save.</div>
              {:else}<div class="note bad">It still doesn't check after {pDraft.tries} tries: {pDraft.errors.join('; ')}. Fix it here (Check again) or describe the rule differently.</div>{/if}
              {#if pReplay}<div class="note">{pReplay.replayed} decisions of the last {pReplay.days} days replayed: <b>{pReplay.flipped} would change</b>.
                {#if pReplay.flips.length}<ul>{#each pReplay.flips.slice(0, 15) as x (x.ts_ms + x.resource)}<li class="mono">{x.before} → {x.after} · {x.principal}{x.on_behalf_of ? ` for ${x.on_behalf_of}` : ''} · {x.action} {x.resource}</li>{/each}</ul>{/if}</div>{/if}
              <div class="acts">
                <button class="btn2" disabled={!!busy || pBusy} onclick={recheck}>Check again</button>
                <button class="btn2" disabled={!!busy || pBusy} onclick={replayDraft}>{busy === 'prp' ? 'Replaying…' : 'Replay the last 7 days'}</button>
                <button class="btn2 primary" disabled={!!busy || pBusy || !pDraft.ok} onclick={saveDraft}>{busy === 'psave' ? 'Saving…' : pDraft.files.length === 1 ? 'Save it' : `Save the ${pDraft.files.length} files`}</button>
                <button class="btn2 quiet" onclick={() => { pDraft = null; pReplay = null; }}>Discard</button>
              </div>
            {/if}
          </div>
        {/if}
        <div class="card">
          <div class="card-head"><h3>2 · Files</h3><span>{files ? `${enforcedNow} of ${files.files.length} enforced · for ${files.projects.length} project${files.projects.length === 1 ? '' : 's'}` : ''}</span><span class="grow"></span>{#if files?.files.length}<button class="btn2" onclick={() => openInStudio('policy', `org:${handle}`, files!.files[0].path)}>Open in Studio</button>{/if}</div>
          {#if files?.files.length}
            {#if managesFiles}<div class="acts"><button class="btn2 quiet" disabled={!!busy} onclick={() => pickAll(true)}>Enforce all</button><button class="btn2 quiet" disabled={!!busy} onclick={() => pickAll(false)}>Enforce none</button></div>{/if}
            <ul class="olist">
              {#each files.files as f (f.path)}
                <li class="frow" class:off={!picked[f.path]}>
                  <label class="fpick"><input type="checkbox" bind:checked={picked[f.path]} disabled={!managesFiles || !!busy} aria-label="Enforce {f.path}" /></label>
                  <button class="link mono" onclick={() => openFileInStudio(f.path)} title="Edit in Studio">{f.path}</button>
                  <span class="fkind"><button class="lnk2" onclick={() => (openFile = openFile === f.path ? '' : f.path)} aria-expanded={openFile === f.path}>{openFile === f.path ? 'hide' : 'show'}</button> · {KIND_LABEL[f.kind] ?? f.kind} · <span class="mono">{savedVersion(f.version)}</span></span>
                  <span class="fstate" class:on={f.enforced !== false}>{f.enforced !== false ? 'enforced' : (f as any).shadow ? 'shadow' : 'off'}</span>
                  {#if openFile === f.path}<pre class="fsrc">{f.content}</pre>{/if}
                  {#if managesFiles}<span class="facts">
                    {#if f.enforced === false && f.kind === 'policy'}<button class="lnk2" disabled={!!busy} onclick={() => toggleShadow(f.path, !(f as any).shadow)}>{(f as any).shadow ? 'stop shadow' : 'run in shadow'}</button> · {/if}
                    <button class="lnk2" disabled={!!busy} onclick={() => replayFile(f.path, f.enforced === false)}>{busy === `rp:${f.path}` ? 'replaying…' : f.enforced === false ? 'replay switching it on' : 'replay without it'}</button></span>{/if}
                  {#if replays[f.path]}{@const rp = replays[f.path]}
                    <div class="note">{rp.replayed} decisions of the last {rp.days} days replayed: <b>{rp.flipped} would change</b>{rp.skipped ? ` (${rp.skipped} events skipped)` : ''}.
                      {#if rp.flips.length}<ul>{#each rp.flips.slice(0, 20) as x (x.ts_ms + x.resource)}<li class="mono">{x.before} → {x.after} · {x.principal}{x.on_behalf_of ? ` for ${x.on_behalf_of}` : ''} · {x.action} {x.resource}{x.project ? ` · ${x.project}` : ''}</li>{/each}</ul>{/if}</div>
                  {/if}</li>
              {/each}
            </ul>
            {#if enforceErrors.length}<div class="note bad">Those files don't work on their own: {enforceErrors.join('; ')}</div>{/if}
            {#if managesFiles && (turningOff.length || turningOn.length)}
              <div class="stop">
                <p class="dim small">{[turningOn.length ? `turns on ${turningOn.length}` : '', turningOff.length ? `turns off ${turningOff.length}` : ''].filter(Boolean).join(', ')}.
                  {#if turningOff.length}A file switched off stops applying at once; it stays here, and stays off when the repo is read again, until you switch it back on.{/if}</p>
                <div class="uf">
                  {#if turningOff.length}<input class="tin grow" type="text" bind:value={stopName} placeholder="Type {handle} to switch them off" aria-label="Type the org's handle to confirm" />{/if}
                  <button class="btn2 {turningOff.length ? 'danger' : 'primary'}" disabled={!!busy || (turningOff.length > 0 && stopName.trim() !== handle)} onclick={applyEnforced}>{busy === 'files' ? 'Applying…' : turningOff.length ? `Switch off ${turningOff.length}${turningOn.length ? ` and on ${turningOn.length}` : ''}` : `Enforce ${turningOn.length}`}</button>
                  <button class="btn2 quiet" disabled={!!busy} onclick={() => pickFrom(files)}>Undo</button>
                </div>
              </div>
            {/if}
          {:else}<p class="empty">No files yet. Its projects follow the platform's rules and their own policies.</p>{/if}
        </div>
      {:else if tab === 'changes'}
        <div class="card">
          <div class="card-head"><h3>1 · Second approver</h3><span>{fourEyes ? 'on: every change waits for another admin' : 'off: a change applies at once'}</span><span class="grow"></span>
            {#if managesFiles}<button class="btn2" disabled={busy === 'four'} onclick={toggleFourEyes}>{fourEyes ? 'Turn off…' : 'Turn on'}</button>{/if}</div>
          <p class="dim">With it on, a save, removal or switch of {handle}'s policy files, and a change to one of its agents' own rules, is checked and then waits here until another of its admins or security admins approves it. Nobody approves their own change. Turning it off is a change too.</p>
        </div>
        <div class="card">
          <div class="card-head"><h3>Decision log</h3><span>{hold ? 'on legal hold: nothing is purged' : keepDays ? `kept ${keepDays} days` : 'kept forever'}</span></div>
          <p class="dim">How long {handle}'s decisions are kept in its log. Once a day older ones are forgotten (ADB keeps their history), unless the org is on legal hold. Shortening it, or lifting a hold, needs the second approver when it is on.</p>
          {#if managesFiles}
            <div class="acts">
              <select bind:value={keepPick} aria-label="Keep decisions for"><option value={0}>Forever</option><option value={90}>90 days</option><option value={365}>1 year</option><option value={730}>2 years</option><option value={2555}>7 years</option></select>
              <button class="btn2" disabled={busy === 'ret' || keepPick === keepDays} onclick={() => saveRetention({ retention_days: keepPick })}>Save</button>
              <button class="btn2" class:danger={hold} disabled={busy === 'ret'} onclick={() => saveRetention({ legal_hold: !hold })}>{hold ? 'Lift the legal hold' : 'Put on legal hold'}</button>
            </div>
          {/if}
          <div class="acts"><button class="btn2" disabled={busy === 'verify'} onclick={checkLog}>{busy === 'verify' ? 'Checking…' : 'Check its log (last 30 days)'}</button></div>
          {#if checked}
            <div class="note" class:bad={!checked.ok} class:okn={checked.ok}>
              {checked.ok ? '✓ Intact' : '✕ Not intact'}: {checked.records} decisions on {checked.chains} chain{checked.chains === 1 ? '' : 's'}{checked.unchained ? ` (${checked.unchained} from before chaining)` : ''}; {checked.anchors_checked} anchor{checked.anchors_checked === 1 ? '' : 's'} matched.
              {#if checked.gaps.length}<br />Gaps: {checked.gaps.map((g) => `${g.chain} ${g.after}→${g.before}`).join('; ')}{/if}
              {#if checked.edits.length}<br />Edited: {checked.edits.map((e) => `${e.chain} #${e.seq}`).join('; ')}{/if}
              {#if checked.broken_links.length}<br />Broken links: {checked.broken_links.length}{/if}
              {#if checked.anchor_mismatch.length}<br />Anchors that don't match: {checked.anchor_mismatch.length}{/if}
            </div>
          {/if}
        </div>
        <div class="card">
          <div class="card-head"><h3>2 · Waiting</h3><span>{pendingChanges.length}</span></div>
          {#if !pendingChanges.length}<p class="dim">Nothing waiting.</p>
          {:else}
            {#each pendingChanges as c (c.id)}
              <div class="review">
                <p><b>{c.summary}</b> · {c.by} · {when(c.at)}</p>
                {#if c.target === 'file' && c.op === 'save'}<pre class="mono cedar">{c.payload?.content ?? ''}</pre>{/if}
                {#if c.target === 'agent' && c.op === 'save'}<pre class="mono cedar">{c.payload?.policy ?? ''}</pre>{/if}
                {#if c.target === 'file' && c.op === 'enforce'}<p class="dim">Enforced after it: {(c.payload?.enforced ?? []).join(', ') || 'none'}</p>{/if}
                {#if (changeErr[c.id] ?? []).length}<ul class="note bad">{#each changeErr[c.id] as e (e)}<li>{e}</li>{/each}</ul>{/if}
                {#if c.can_decide}
                  <label class="fld full">Note <small>kept with the decision</small><input bind:value={changeNote[c.id]} maxlength="500" /></label>
                  <div class="acts"><button class="btn2 primary" disabled={!!busy} onclick={() => decideChange(c, true)}>{busy === c.id ? 'Applying…' : 'Approve and apply'}</button>
                    <button class="btn2 danger" disabled={!!busy} onclick={() => decideChange(c, false)}>Reject</button></div>
                {:else}<div class="acts"><span class="dim">Yours: another admin approves it.</span><button class="btn2 quiet" disabled={!!busy} onclick={() => decideChange(c, false)}>Withdraw</button></div>{/if}
              </div>
            {/each}
          {/if}
        </div>
        {#if changes.length > pendingChanges.length}
          <div class="card">
            <div class="card-head"><h3>3 · Decided</h3><span>latest first</span></div>
            <ul class="rows wide">
              {#each changes.filter((c) => c.status !== 'pending') as c (c.id)}
                <li><span class="lbl">{c.status}</span><span class="val">{c.summary} · by {c.by} · {c.status} by {c.decided_by} {when(c.decided_at ?? '')}{c.note ? ` · ${c.note}` : ''}</span><span></span></li>
              {/each}
            </ul>
          </div>
        {/if}
      {/if}
    {/if}
    </section>
  </div></div></div>
</div>

<style>
  .panel-layout { display: grid; grid-template-columns: 280px 1fr; height: 100%; }
  .panel-col { display: flex; flex-direction: column; min-height: 0; min-width: 0; height: 100%; }
  .panel-content { overflow-y: auto; flex: 1; min-height: 0; }
  .pf { padding: 24px 32px; max-width: var(--detail-w); box-sizing: border-box; }
  @container details (max-width: 560px) { .pf { padding: 16px 14px; } }
  .tin { height: 34px; padding: 0 10px; border: 1px solid var(--border); border-radius: 8px; background: var(--bg-surface); font: 13px var(--font-sans); min-width: 0; }
  .note.okn { background: color-mix(in srgb, var(--ok, #1a7158) 10%, transparent); color: var(--ok, #1a7158); }
  .uf { display: flex; flex-wrap: wrap; gap: 8px; }
  .uf .grow { flex: 1 1 220px; width: auto !important; }
  .uf select { flex: 0 1 170px; width: auto !important; min-width: 0; }
  .small { font-size: 12.5px; margin: 0; }
  .uwho { display: flex; flex-direction: column; align-items: flex-start; min-width: 0; padding: 0; border: 0; background: none; text-align: left; font: inherit; color: inherit; }
  .uwho { cursor: pointer; }
  .uwho b { font-size: 13.5px; color: var(--text-1); }
  .uwho small { font-size: 12px; color: var(--text-4); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; max-width: 100%; }
  .prow { display: grid; grid-template-columns: minmax(0, 1.2fr) minmax(0, 1fr) 170px 76px; align-items: center; gap: 10px; }
  .prow select { width: 100%; }
  .prow > .chip { justify-self: start; }
  .uchips { display: flex; flex-wrap: wrap; align-items: center; gap: 4px; min-width: 0; }
  .chip { padding: 2px 8px; border-radius: 999px; border: 1px solid var(--border); background: var(--bg-subtle); font-size: 11.5px; color: var(--text-2); white-space: nowrap; }
  .chip.sec { border-color: color-mix(in srgb, #5530a8 40%, var(--border)); color: #5530a8; }
  .chip.out { border-style: dashed; }
  .chip.here { background: var(--bg-surface); }
  .udetail { grid-column: 1 / -1; padding: 10px 12px; border-radius: 8px; background: var(--bg-page); }
  .uprojects { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 4px; }
  .uprojects li { display: grid; grid-template-columns: minmax(0, 1fr) minmax(0, 1.4fr) 120px; gap: 8px; font-size: 12.5px; }
  .link { border: 0; background: none; padding: 0; color: var(--accent, #5530a8); cursor: pointer; font: inherit; text-align: left; }
  .fsrc { flex: 1 1 100%; margin: 6px 0 0; padding: 10px 12px; border-radius: 8px; background: #111214; color: #f4f4f5; font: 12px/1.55 var(--font-mono); white-space: pre-wrap; overflow-x: auto; max-height: 360px; }
  /* Own lists, not the shared .rows grid (its label column and narrow-width
     reordering are for label/value rows). */
  .olist { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; }
  .olist > li { padding: 10px 0; border-top: 1px solid var(--divider, var(--border)); }
  .olist > li:first-child { border-top: 0; }
  .frow { display: grid; grid-template-columns: 22px minmax(0, 1fr) auto 64px; align-items: center; gap: 10px; }
  .frow .link { overflow-wrap: anywhere; }
  .frow .facts { grid-column: 2 / -1; font-size: 12px; color: var(--text-3); }
  .frow > .note { grid-column: 1 / -1; }
  .lnk2 { border: 0; background: none; padding: 0; color: var(--accent, #5530a8); cursor: pointer; font: inherit; }
  .fkind { font-size: 12.5px; color: var(--text-3); white-space: nowrap; }
  .frow.off .link, .frow.off .fkind { opacity: 0.55; }
  .mname { display: flex; align-items: center; gap: 6px; flex-wrap: wrap; }
  .frow .fsrc { grid-column: 1 / -1; }
  .fpick input { width: 16px; height: 16px; margin: 0; }
  .fstate { font-size: 11.5px; color: var(--text-4); text-align: right; }
  .fstate.on { color: var(--ok, #1a7158); }
  .rrow { display: grid; grid-template-columns: minmax(0, 1.2fr) minmax(0, 1fr) 110px; gap: 10px; align-items: baseline; }
  .rrow > :last-child { justify-self: end; }
  .steps { margin: 4px 0 8px; padding-left: 20px; font-size: 12.5px; color: var(--text-2); display: flex; flex-direction: column; gap: 4px; }
  .sub { margin: 6px 0 0; font: 600 12.5px var(--font-sans); color: var(--text-2); }
  .proj { display: flex; gap: 8px; align-items: baseline; margin-top: 6px; font-size: 13px; }
  .proj .lnk2 { margin-left: auto; }
  .stop { display: flex; flex-direction: column; gap: 8px; padding-top: 10px; border-top: 1px solid var(--border); }
  textarea.mono { font-family: var(--font-mono); font-size: 12.5px; }
  @container details (max-width: 560px) {
    .rrow { grid-template-columns: 1fr; }
    .prow { grid-template-columns: minmax(0, 1fr) auto; row-gap: 6px; }
    .prow > .uwho { grid-column: 1 / -1; }
    .prow > .uchips { grid-column: 1 / -1; }
    .prow select { width: auto; max-width: 100%; }
    .frow { grid-template-columns: 22px minmax(0, 1fr) 56px; }
    .frow .fkind { grid-column: 2 / -1; grid-row: 2; }
    .uprojects li { grid-template-columns: 1fr; }
  }
</style>
