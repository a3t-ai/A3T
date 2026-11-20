<script lang="ts">
  /// Security › an org: its identity and its policy files, in one place.
  ///   Overview — the org at a glance, and who looks after what;
  ///   People — its members and everyone who reaches its projects, and how (audit);
  ///   Members — who is in it, and as what (its admins change it);
  ///   Sign-in — its own identity provider and which groups get which role;
  ///   Policy files — its roles, rules and access, kept as files in A3T (edited in Studio).
  /// Its admins change members and sign-in; its security admins read them. Both
  /// manage the policy files (the org's security). SuperAdmin does everything;
  /// SecurityAdmin reads members and sign-in and manages policy files. The
  /// server checks every one of these; this page only offers what will work.
  import { untrack } from 'svelte';
  import * as YAML from 'yaml';
  import PropertiesTree from '$lib/components/shared/PropertiesTree.svelte';
  import WizardHead from '$lib/components/shared/WizardHead.svelte';
  import { POLICY_STARTERS, type PolicyDir } from '$lib/studio-ui';
  import { openInStudio } from '$lib/stores/panels.svelte.ts';
  import { orgsApi, orgPolicy, securityPolicies, type PolicyBroken, type PolicyChange, type ReplayResult, loadDirectory, people as peopleApi, type OrgInfo, type OrgRole, type OrgSso, type DirectoryUser, type AuditPeople, type OrgPolicyFiles } from '$lib/services/api';
  import { isInfosec } from '$lib/stores/auth';
  import { loadProjects } from '$lib/stores/context.svelte.ts';
  import { bumpItems } from '$lib/stores/apps.svelte.ts';

  /// `project`: open People filtered to who reaches that project.
  /// `section`: identity (who people are: sign-in, members, roles and groups)
  /// or policy (what is allowed: the org's policy files). Two pages, one org.
  let { org: handle, section = 'identity', project = '', tab = $bindable('overview') }: { org: string; section?: 'identity' | 'policy'; project?: string; tab?: string } = $props();

  const ROLES: OrgRole[] = ['member', 'admin', 'securityadmin'];
  const ROLE_LABEL: Record<string, string> = { member: 'Member', admin: 'Admin', securityadmin: 'Security admin' };
  const CAP_LABEL: Record<string, string> = {
    owner: 'Owner', developer: 'Developer', operator: 'Operator', viewer: 'Viewer', 'org-admin': 'Org admin',
    'org-securityadmin': 'Org security admin', 'org-member': 'Org member', superadmin: 'SuperAdmin', exposed: 'Shared with them',
  };
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
  const canAdmin = $derived(!!org?.can_admin);
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
    org = found; fillSso(found);
    aud = a.ok ? a.data ?? null : null; audErr = a.ok ? '' : (a.error ?? `HTTP ${a.status}`);
    files = f.ok ? f.data ?? null : null; filesErr = f.ok ? '' : (f.error ?? `HTTP ${f.status}`);
    pickFrom(files);
    if (found.can_admin && !directory.length) directory = await loadDirectory();
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

  // ── People (audit) ──
  let fQ = $state(''); let fRole = $state(''); let fProject = $state(''); let fCap = $state('');
  let openRow = $state('');
  const rows = $derived.by(() => {
    if (!aud) return [];
    const q = fQ.trim().toLowerCase();
    return aud.users.filter((u) => {
      if (q && ![u.email, u.name, u.id].some((x) => (x ?? '').toLowerCase().includes(q))) return false;
      const mine = u.orgs.find((o) => o.org === handle)?.role ?? '';
      if (fRole === '-' ? !!mine : fRole && mine !== fRole) return false;
      if (fProject) {
        const here = u.projects.find((p) => p.namespace === fProject);
        if (!here) return false;
        if (fCap && !here.capacities.includes(fCap as any)) return false;
      }
      return true;
    }).map((u) => ({ ...u, orgRole: u.orgs.find((o) => o.org === handle)?.role ?? '', here: fProject ? u.projects.find((p) => p.namespace === fProject) : undefined }));
  });
  const fProjectInfo = $derived(aud?.projects.find((p) => p.namespace === fProject));
  const outsiders = $derived(aud ? aud.users.filter((u) => !u.orgs.some((o) => o.org === handle)).length : 0);

  // ── Members ──
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
  async function setRole(m: OrgInfo['members'][number], role: OrgRole) {
    if (!org) return;
    busy = 'member';
    const r = await orgsApi.setMember(org.handle, { user_id: m.user_id, email: m.email, name: m.name }, role);
    busy = '';
    if (!r.ok) say(r.error ?? 'Could not change the role.'); else say(`${m.name || m.email || m.user_id} is now ${ROLE_LABEL[role].toLowerCase()}.`, true);
    await refreshOrg(); void loadProjects(true);
  }
  async function removeMember(m: OrgInfo['members'][number]) {
    if (!org || !confirm(`Remove ${m.name || m.email || m.user_id} from ${org.handle}?`)) return;
    busy = 'member';
    const r = await orgsApi.removeMember(org.handle, m.user_id);
    busy = '';
    if (!r.ok) say(r.error ?? 'Could not remove them.'); else say(`Removed ${m.name || m.email || m.user_id}.`, true);
    await refreshOrg(); void loadProjects(true);
  }

  // ── Sign-in ──
  let sso = $state({ alias: '', domains: '', map: '', dflt: 'member' as 'member' | '' });
  function fillSso(o: OrgInfo) {
    const s = o.sso;
    sso = { alias: s?.idp_alias ?? '', domains: (s?.domains ?? []).join(', '),
      map: Object.entries(s?.role_map ?? {}).map(([g, r]) => `${g} = ${r}`).join('\n'), dflt: s ? s.default_role : 'member' };
  }
  async function saveSso() {
    if (!org) return;
    const role_map: Record<string, OrgRole> = {};
    for (const line of sso.map.split('\n').map((l) => l.trim()).filter(Boolean)) {
      const [g, r] = line.split('=').map((x) => x.trim());
      if (!g || !ROLES.includes(r as OrgRole)) { say(`"${line}": write each line as group = member, admin or securityadmin.`); return; }
      role_map[g] = r as OrgRole;
    }
    const body: OrgSso = { idp_alias: sso.alias.trim(), domains: sso.domains.split(',').map((d) => d.trim().toLowerCase()).filter(Boolean), role_map, default_role: sso.dflt };
    busy = 'sso';
    const r = await orgsApi.setSso(org.handle, body);
    busy = '';
    if (!r.ok) { say(r.error ?? 'Sign-in not saved.'); return; }
    say(body.idp_alias ? `${org.handle} now signs in through "${body.idp_alias}".` : `Sign-in through the org's own provider is off for ${org.handle}.`, true);
    await refreshOrg(); if (org) fillSso(org);
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
    { id: 'people', label: 'People', icon: '<circle cx="6" cy="5" r="2.2"/><circle cx="11.5" cy="6" r="1.8"/><path d="M2 13a4 4 0 018 0M9.5 12.5a3 3 0 015 0"/>', hint: 'Its members and everyone who reaches its projects, and how' },
    { id: 'members', label: 'Members', icon: '<circle cx="8" cy="5" r="2.5"/><path d="M3 13a5 5 0 0110 0"/>', hint: 'Who is in the org, and as what' },
    { id: 'signin', label: 'Sign-in', icon: '<rect x="3" y="7" width="10" height="7" rx="1.5"/><path d="M5.5 7V5a2.5 2.5 0 015 0v2"/>', hint: "The org's own identity provider, and which groups get which role" },
    { id: 'roles', label: 'Roles & groups', icon: '<circle cx="5" cy="5" r="2"/><circle cx="11" cy="5" r="2"/><path d="M1.5 13a3.5 3.5 0 017 0M7.5 13a3.5 3.5 0 017 0"/>', hint: 'The roles the org has, and which groups and people hold them' },
    { id: 'files', label: 'Policy files', icon: '<path d="M4 1.5h5l3 3v10H4z"/><path d="M9 1.5v3h3M6 8h4M6 10.5h4"/>', hint: 'Its roles, rules and access, as files: switch each on or off, edit in Studio' },
    { id: 'changes', label: 'Changes', icon: '<circle cx="5" cy="8" r="2"/><circle cx="11" cy="8" r="2"/><path d="M7 8h2"/>', hint: 'A second approver for policy changes, and the changes waiting for one' },
  ];
  const IDENTITY_TABS = ['overview', 'people', 'members', 'signin', 'roles'];
  const STEPS = $derived(section === 'policy' ? ALL_STEPS.filter((x) => x.id === 'files' || x.id === 'changes') : ALL_STEPS.filter((x) => IDENTITY_TABS.includes(x.id)));
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

  // ── Changes: a second approver (GOV-3) ──
  let fourEyes = $state(false);
  let changes = $state<PolicyChange[]>([]);
  let changeNote = $state<Record<string, string>>({});
  let changeErr = $state<Record<string, string[]>>({});
  async function loadChanges() {
    const r = await orgPolicy.changes(handle);
    if (r.ok) { fourEyes = !!r.data?.four_eyes; changes = r.data?.changes ?? []; }
  }
  $effect(() => { if (tab === 'changes' && org) untrack(() => void loadChanges()); });
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
  <PropertiesTree icon={section === 'policy' ? 'PF' : 'ID'} color="#5530a8" name={section === 'policy' ? 'Policy files' : 'Identity'} meta={`org ${org?.display_name ?? handle} · ${section === 'policy' ? 'what is allowed' : 'who people are'}`} nodes={[]} steps={STEPS} activeNode={tab} onselect={(id) => (tab = id)} />
  <div class="panel-col"><div class="panel-content"><div class="pf">
    <WizardHead step={STEPS.find((s) => s.id === tab)} />
    <section class="agt">
    {#if err}<div class="note bad">{err}</div>
    {:else if !org}<p class="dim">Reading {handle}…</p>
    {:else}
      {#if msg}<div class="note" class:bad={!msgOk} class:okn={msgOk}>{msg}</div>{/if}
      {#if broken}<div class="note bad"><b>Its policy files no longer compile</b> (since {new Date(broken.since).toLocaleString()}), so its projects ({broken.closed.join(', ') || 'none'}) refuse every call until they are fixed in Policy files.
        <ul>{#each broken.errors as e (e)}<li class="mono">{e}</li>{/each}</ul></div>{/if}

      {#if tab === 'overview'}
        <div class="card">
          <div class="card-head"><h3>1 · {org.display_name}</h3><span>you are {org.my_role ? ROLE_LABEL[org.my_role].toLowerCase() : infosec ? 'platform InfoSec' : '—'} here</span></div>
          <div class="tiles">
            <div class="tile"><span class="v">{org.members.length}</span><span class="k">members</span></div>
            <div class="tile"><span class="v">{memberCount('admin')}</span><span class="k">admins</span></div>
            <div class="tile"><span class="v">{memberCount('securityadmin')}</span><span class="k">security admins</span></div>
            <div class="tile"><span class="v">{aud?.projects.length ?? '…'}</span><span class="k">projects</span></div>
          </div>
          <ul class="rows">
            <li><span class="lbl">People</span><span class="val">{aud ? `${aud.count} can reach its projects or are in it${outsiders ? `, ${outsiders} from outside the org` : ''}` : audErr || '…'}</span><button class="btn2" onclick={() => (tab = 'people')}>Audit</button></li>
            <li><span class="lbl">Members</span><span class="val">{org.members.length} · {memberCount('admin')} admin{memberCount('admin') === 1 ? '' : 's'}, {memberCount('securityadmin')} security admin{memberCount('securityadmin') === 1 ? '' : 's'}</span><button class="btn2" onclick={() => (tab = 'members')}>{canAdmin ? 'Manage' : 'See'}</button></li>
            <li><span class="lbl">Sign-in</span><span class="val">{org.sso?.idp_alias ? `through ${org.sso.idp_alias}${org.sso.domains?.length ? ` for ${org.sso.domains.join(', ')}` : ''}` : "A3T's own sign-in"}</span><button class="btn2" onclick={() => (tab = 'signin')}>{canAdmin ? 'Set up' : 'See'}</button></li>
            <li><span class="lbl">Roles</span><span class="val">{BASELINE.length} baseline{orgRoles.length ? ` + ${orgRoles.length} of its own (${orgRoles.map((r) => r.y.name ?? r.f.path).join(', ')})` : ''} · {accessFiles.length} project{accessFiles.length === 1 ? '' : 's'} with group mappings</span><button class="btn2" onclick={() => (tab = 'roles')}>See</button></li>
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

      {:else if tab === 'people'}
        <div class="card">
          <div class="card-head"><h3>People</h3><span>members of {handle}, and everyone who reaches its projects</span></div>
          {#if !aud}<p class="dim">{audErr ? `Could not read who reaches its projects: ${audErr}` : 'Reading…'}</p>
          {:else}
            <div class="uf">
              <input class="tin grow" type="search" bind:value={fQ} placeholder="Find by name or email" aria-label="Find a person" />
              <select class="tin" bind:value={fRole} aria-label="Role in the org">
                <option value="">In the org or not</option>
                {#each ROLES as r}<option value={r}>Org {ROLE_LABEL[r].toLowerCase()}</option>{/each}
                <option value="-">Not in the org</option>
              </select>
              <select class="tin" bind:value={fProject} onchange={() => (fCap = '')} aria-label="Reaches project">
                <option value="">Any project</option>{#each aud.projects as p (p.namespace)}<option value={p.namespace}>{p.namespace}</option>{/each}
              </select>
              {#if fProject}<select class="tin" bind:value={fCap} aria-label="Their capacity there"><option value="">Any capacity</option>{#each Object.keys(CAP_LABEL) as c}<option value={c}>{CAP_LABEL[c]}</option>{/each}</select>{/if}
            </div>
            <p class="dim small">{rows.length} of {aud.count} {aud.count === 1 ? 'person' : 'people'}{fProject ? ` can reach ${fProject}` : ''}{fProjectInfo?.everyone ? ', and everyone signed in can run it' : ''}{fProjectInfo?.private ? ' (private)' : ''}.</p>
            <ul class="rows wide ul">
              {#each rows as u (u.id)}
                <li class="urow">
                  <button class="uwho" onclick={() => (openRow = openRow === u.id ? '' : u.id)} aria-expanded={openRow === u.id}>
                    <b>{u.name || u.email || u.id}</b><small>{[u.name ? u.email : '', !u.account ? 'no sign-in account' : ''].filter(Boolean).join(' · ')}</small>
                  </button>
                  <span class="uchips">
                    {#if u.orgRole}<span class="chip" class:sec={u.orgRole !== 'member'}>Org {ROLE_LABEL[u.orgRole]?.toLowerCase() ?? u.orgRole}</span>{:else}<span class="chip out">outside the org</span>{/if}
                    {#if u.here}<span class="dim small here-lbl">· in {fProject}:</span>{#each u.here.capacities as c}<span class="chip here">{CAP_LABEL[c] ?? c}</span>{/each}<span class="dim small">{ACCESS_WORD[u.here.access] ?? u.here.access}</span>
                    {:else}<span class="dim small">{u.projects.length} project{u.projects.length === 1 ? '' : 's'}</span>{/if}
                  </span>
                  {#if openRow === u.id}
                    <div class="udetail">
                      <ul class="uprojects">{#each u.projects as p (p.namespace)}
                        <li><button class="link" onclick={() => { fProject = p.namespace; fCap = ''; openRow = ''; }}>{p.namespace}</button>
                          <span>{p.capacities.map((c) => CAP_LABEL[c] ?? c).join(', ')}</span><span class="dim">{ACCESS_WORD[p.access] ?? p.access}</span></li>
                      {:else}<li class="dim">reaches none of its projects</li>{/each}</ul>
                    </div>
                  {/if}
                </li>
              {:else}<li><span class="dim">Nobody matches.</span><span></span><span></span></li>{/each}
            </ul>
          {/if}
        </div>

      {:else if tab === 'members'}
        <div class="card">
          <div class="card-head"><h3>Members</h3><span>{canAdmin ? 'you can change them' : 'read only: its admins change them'}</span></div>
          <ul class="olist">
            {#each org.members as m (m.user_id)}
              <li class="mrow">
                <span class="mwho"><span class="mname"><b>{m.name || m.email || m.user_id}</b>{#if m.source === 'sso'}<span class="chip" title="From the org's sign-in: its groups set the role at every sign-in">SSO</span>{/if}</span><small>{m.email || m.user_id}</small></span>
                {#if canAdmin}
                  <select class="tin" value={m.role} disabled={!!busy} onchange={(e) => setRole(m, e.currentTarget.value as OrgRole)} aria-label="Role">
                    {#each ROLES as r}<option value={r}>{ROLE_LABEL[r]}</option>{/each}
                  </select>
                  <button class="btn2 quiet" disabled={!!busy} onclick={() => removeMember(m)}>Remove</button>
                {:else}<span class="mrole">{ROLE_LABEL[m.role] ?? m.role}</span>{/if}
              </li>
            {:else}<li class="dim">No members yet.</li>{/each}
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

      {:else if tab === 'signin'}
        <div class="card">
          <div class="card-head"><h3>Sign-in</h3><span>{canAdmin ? "the org's own identity provider" : 'read only: its admins change it'}</span></div>
          <p class="dim">People sign in through the org's own identity provider (Entra ID, Okta, Google, its own Keycloak…), connected to A3T's Keycloak under the name below. They land in this org only, with the role their groups give them, checked at every sign-in. Members added by hand are not changed by sign-in.</p>
          <div class="fields">
            <label class="fld">Identity provider<input class="mono" bind:value={sso.alias} readonly={!canAdmin} placeholder="its name in Keycloak, e.g. acme (empty: off)" /></label>
            <label class="fld">Email domains<input bind:value={sso.domains} readonly={!canAdmin} placeholder="acme.com, acme.co.uk" /></label>
            <label class="fld full">Groups → role <small>one per line: group = member, admin or securityadmin</small><textarea class="mono" rows="4" bind:value={sso.map} readonly={!canAdmin} placeholder={'a3t-admins = admin\nsecurity = securityadmin'}></textarea></label>
            <label class="fld">Anyone else<select bind:value={sso.dflt} disabled={!canAdmin}><option value="member">Member</option><option value="">No access</option></select></label>
          </div>
          {#if canAdmin}<div class="acts"><button class="btn2 primary" onclick={saveSso} disabled={!!busy}>{busy === 'sso' ? 'Saving…' : 'Save sign-in'}</button></div>{/if}
        </div>

      {:else if tab === 'roles'}
        <div class="card">
          <div class="card-head"><h3>1 · Roles</h3><span>what each role may do is in the policy files</span></div>
          <ul class="olist">
            {#each BASELINE as r (r.name)}<li class="rrow"><b>{r.name}</b><span class="dim">baseline · {r.what}</span><span></span></li>{/each}
            {#each orgRoles as r (r.f.path)}
              <li class="rrow"><b>{r.y.name ?? r.f.path}</b><span class="dim">{handle}'s own · {r.y.extends ?? '—'}{(r.y.remove ?? []).length ? ` without ${(r.y.remove ?? []).join(', ')}` : ''}{r.f.enforced ? '' : ' · switched off'}{r.y.description ? ` · ${r.y.description}` : ''}</span>
                <button class="lnk2" onclick={() => openFileInStudio(r.f.path)}>Edit in Studio</button></li>
            {/each}
          </ul>
        </div>
        <div class="card">
          <div class="card-head"><h3>2 · Groups and people</h3><span>who holds which role</span></div>
          <h4 class="sub">In the org, at sign-in</h4>
          {#if org.sso?.idp_alias}
            <ul class="olist">
              {#each pairs(org.sso.role_map) as [g, r] (g)}<li class="rrow"><span class="mono">{g}</span><span>org {ROLE_LABEL[r]?.toLowerCase() ?? r}</span><span></span></li>{/each}
              <li class="rrow"><span class="dim">anyone else from {org.sso.idp_alias}</span><span>{org.sso.default_role ? `org ${ROLE_LABEL[org.sso.default_role]?.toLowerCase()}` : 'no access'}</span><button class="lnk2" onclick={() => (tab = 'signin')}>Sign-in</button></li>
            </ul>
          {:else}<p class="dim">No sign-in of its own: people are added under Members.</p>{/if}
          <h4 class="sub">On its projects</h4>
          {#each accessFiles as a (a.f.path)}
            <div class="proj"><span class="mono">{a.y.project ?? '?'}</span>{#if !a.f.enforced}<span class="dim"> · switched off</span>{/if}<button class="lnk2" onclick={() => openFileInStudio(a.f.path)}>Edit in Studio</button></div>
            <ul class="olist">
              {#each pairs(a.y.groups) as [g, r] (g)}<li class="rrow"><span class="mono">{g}</span><span>{r}</span><span class="dim">group</span></li>{/each}
              {#each pairs(a.y.users) as [u, r] (u)}<li class="rrow"><span class="mono">{u}</span><span>{r}</span><span class="dim">person</span></li>{/each}
            </ul>
          {:else}<p class="dim">No access files: people hold project roles as project members (Projects › Access).</p>{/each}
          {#if managesFiles}<p class="dim small">Group mappings are access files: create one under Policy files.</p>{/if}
        </div>

      {:else if tab === 'files'}
        {#if filesErr && !files}<div class="note bad">{filesErr}</div>{/if}
        <div class="card">
          <div class="card-head"><h3>1 · What they are</h3><span>files kept in A3T, every save a version</span></div>
          <p class="dim">An org's rules (<span class="mono">policies/</span>), its own roles (<span class="mono">roles/</span>) and who holds which role on its projects (<span class="mono">access/</span>), as YAML files. Open one to edit it in Studio: it's checked against the org's other files as you type, and a save that would break them is refused.</p>
          {#if managesFiles}
            <div class="uf">
              <select class="tin" bind:value={nDir} aria-label="Kind of file"><option value="policies">Rule (policies/)</option><option value="roles">Role (roles/)</option><option value="access">Access (access/)</option></select>
              <input class="tin grow mono" bind:value={nName} placeholder={nDir === 'policies' ? 'no-external-models' : nDir === 'roles' ? 'support-lead' : 'support-team'} aria-label="File name" onkeydown={(e) => { if (e.key === 'Enter') newFile(); }} />
              {#if nDir === 'access'}<select class="tin" bind:value={nProject} aria-label="Project"><option value="">Project…</option>{#each files?.projects ?? [] as p}<option value={p}>{p}</option>{/each}</select>{/if}
              <button class="btn2 primary" disabled={!!busy || !nName.trim()} onclick={newFile}>{busy === 'new' ? 'Creating…' : 'New file'}</button>
            </div>
            <p class="dim small">It starts as a working example of its kind and opens in Studio.</p>
          {/if}
        </div>
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
  .ul .urow { display: grid; grid-template-columns: minmax(0, 1.2fr) minmax(0, 1.5fr); align-items: center; gap: 10px; }
  .uwho, .mwho { display: flex; flex-direction: column; align-items: flex-start; min-width: 0; padding: 0; border: 0; background: none; text-align: left; font: inherit; color: inherit; }
  .uwho { cursor: pointer; }
  .uwho b, .mwho b { font-size: 13.5px; color: var(--text-1); }
  .uwho small, .mwho small { font-size: 12px; color: var(--text-4); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; max-width: 100%; }
  .mrow { display: grid; grid-template-columns: minmax(0, 1fr) 170px auto; align-items: center; gap: 10px; }
  .mrow select { width: 100%; }
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
  .mrole { font-size: 13px; color: var(--text-2); }
  .frow .fsrc { grid-column: 1 / -1; }
  .fpick input { width: 16px; height: 16px; margin: 0; }
  .fstate { font-size: 11.5px; color: var(--text-4); text-align: right; }
  .fstate.on { color: var(--ok, #1a7158); }
  .rrow { display: grid; grid-template-columns: minmax(120px, 0.6fr) minmax(0, 1.4fr) auto; gap: 10px; align-items: baseline; }
  .sub { margin: 6px 0 0; font: 600 12.5px var(--font-sans); color: var(--text-2); }
  .proj { display: flex; gap: 8px; align-items: baseline; margin-top: 6px; font-size: 13px; }
  .proj .lnk2 { margin-left: auto; }
  .stop { display: flex; flex-direction: column; gap: 8px; padding-top: 10px; border-top: 1px solid var(--border); }
  textarea.mono { font-family: var(--font-mono); font-size: 12.5px; }
  @container details (max-width: 560px) {
    .ul .urow, .mrow, .rrow { grid-template-columns: 1fr; }
    .frow { grid-template-columns: 22px minmax(0, 1fr) 56px; }
    .frow .fkind { grid-column: 2 / -1; grid-row: 2; }
    .uprojects li { grid-template-columns: 1fr; }
  }
</style>
