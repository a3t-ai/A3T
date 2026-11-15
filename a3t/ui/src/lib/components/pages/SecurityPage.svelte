<script lang="ts">
  /// Security — for SuperAdmin and SecurityAdmin only (the server refuses anyone
  /// else). Items lists Identity (the SSO realm, platform-wide) and the Cedar
  /// policies of the projects in view, with the compiled baseline. The one picked
  /// opens here in the agents' frame:
  ///   a policy — Overview, Policy (its Cedar), Test (a dry run), Decisions (the
  ///   decision log entries it made);
  ///   Identity — Overview (the IdP connection), Users (realm users and roles).
  import { untrack } from 'svelte';
  import SlidePanel from '$lib/components/shared/SlidePanel.svelte';
  import PropertiesTree from '$lib/components/shared/PropertiesTree.svelte';
  import WizardHead from '$lib/components/shared/WizardHead.svelte';
  import '$lib/components/agent/agent.css';
  import '$lib/components/triggers/triggers.css';
  import { securityPolicies, idp, audit, getEdition, type PolicyTestBody } from '$lib/services/api';
  import { getActivePanel, navigateTo, navigateToDetail } from '$lib/stores/panels.svelte.ts';
  import { getProject, namespacesFor, allProjectNamespaces } from '$lib/stores/context.svelte.ts';
  import { bumpItems } from '$lib/stores/apps.svelte.ts';
  import { isInfosec } from '$lib/stores/auth';
  import { BASELINE_POLICY } from '$lib/data/baselinePolicy';

  type Policy = { namespace: string; name: string; cedar: string; description?: string | null; readOnly?: boolean };
  const BASELINE: Policy = { namespace: 'system', name: 'baseline', cedar: BASELINE_POLICY, readOnly: true,
    description: 'Deny-by-default guardrails compiled into the EE runtime. Always applies beneath every project policy; forbid overrides permit.' };

  let view = $state<'policy' | 'identity' | 'new' | null>(null);
  let pol = $state<Policy | null>(null);
  let edition = $state<'ce' | 'ee' | null>(null);
  let denied = $state('');
  let tab = $state('overview');

  let shown = '';
  $effect(() => {
    const p = getActivePanel()?.props ?? {};
    const id = p.selectedId as string | undefined;
    const create = p.create;
    untrack(() => {
      if (create && (!id || id === 'policies')) { void openNew(); return; }
      if (!id || id === shown) { if (!id) { shown = ''; view = null; } return; }
      shown = id; void open(id);
    });
  });

  async function ensureEdition() { if (!edition) { const e = await getEdition(); edition = e.ok && e.data?.edition === 'ee' ? 'ee' : 'ce'; } }
  async function open(id: string) {
    await ensureEdition(); denied = ''; tab = 'overview'; msg = ''; testRes = null; decisions = null;
    // InfoSec only: say so, and ask the server nothing it would refuse.
    if (!isInfosec()) { denied = 'Security is for SuperAdmin and SecurityAdmin.'; view = 'policy'; pol = null; return; }
    if (id === 'identity' || id === 'policies') { view = 'identity'; pol = null; void loadIdp(); return; }
    if (id === 'baseline') { view = 'policy'; pol = { ...BASELINE }; return; }
    const ref = id.replace(/^policy:/, '');
    const [ns, ...rest] = ref.split('/'); const name = rest.join('/');
    const r = await securityPolicies.list();
    if (!r.ok) { denied = r.status === 403 ? 'Security is for SuperAdmin and SecurityAdmin.' : (r.error ?? 'Policies could not be read.'); view = 'policy'; pol = null; return; }
    const p = ((r.data?.policies ?? []) as Policy[]).find((x) => x.namespace === ns && x.name === name);
    view = 'policy'; pol = p ? { ...p } : null; saved = p?.cedar ?? '';
    if (!p) denied = `No policy ${ref}.`;
  }
  const close = () => navigateTo('security', 'Security');

  // ── A policy ──
  let saved = $state('');
  let msg = $state('');
  let busy = $state('');
  const dirty = $derived(!!pol && !pol.readOnly && pol.cedar !== saved);
  /// What a Cedar text says, roughly: how many permits and forbids, and what they name.
  const shape = $derived.by(() => {
    const t = pol?.cedar ?? '';
    const body = t.replace(/\/\/.*$/gm, '');
    return {
      permits: (body.match(/\bpermit\s*\(/g) ?? []).length,
      forbids: (body.match(/\bforbid\s*\(/g) ?? []).length,
      actions: [...new Set([...body.matchAll(/Action::"([^"]+)"/g)].map((m) => m[1]))].slice(0, 8),
      roles: [...new Set([...body.matchAll(/"(SuperAdmin|SecurityAdmin|Admin|Operator|Viewer)"/g)].map((m) => m[1]))],
    };
  });
  async function save(apply = true) {
    if (!pol || pol.readOnly) return;
    busy = 'save'; msg = '';
    const r = await securityPolicies.update(pol.namespace, pol.name, pol.cedar, pol.description || undefined);
    if (!r.ok) { busy = ''; msg = r.error ?? 'Not saved.'; return; }
    saved = pol.cedar;
    if (apply) await reload(); else { busy = ''; msg = 'Saved — not applied yet.'; }
  }
  async function reload() {
    busy = 'apply';
    const r = await securityPolicies.reload();
    busy = '';
    msg = r.ok && r.data?.reloaded ? `Saved and applied: ${r.data.registry_policies} project polic${r.data.registry_policies === 1 ? 'y' : 'ies'} active.` : `Saved, but not applied: ${r.data?.reason ?? r.data?.error ?? r.error ?? 'refused'}`;
  }
  let delName = $state('');
  async function remove() {
    if (!pol || pol.readOnly || delName !== pol.name) return;
    busy = 'delete';
    const r = await securityPolicies.remove(pol.namespace, pol.name);
    if (r.ok) { await securityPolicies.reload(); busy = ''; bumpItems(); close(); } else { busy = ''; msg = r.error ?? 'Not deleted.'; }
  }

  // Test: a dry run against what is active, with this text in place of the saved one.
  const ROLES = ['SuperAdmin', 'SecurityAdmin', 'Admin', 'Operator', 'Viewer'];
  const ACTIONS = ['mcp:call', 'tool:invoke', 'resource:create', 'resource:update', 'resource:delete', 'llm:call', 'flow:start', 'flow:deploy'];
  const RES_TYPES = ['Tool', 'Resource', 'Llm', 'Flow', 'Runtime'];
  let tRole = $state('Operator'); let tAction = $state('mcp:call'); let tType = $state('Tool'); let tId = $state('list_flows'); let tDraft = $state(true);
  let testRes = $state<any>(null);
  async function runTest() {
    busy = 'test'; testRes = null;
    // Tried in the policy's own project: a project policy governs only the calls
    // that touch its project (the platform's, every call).
    const ns = pol && !pol.readOnly ? pol.namespace : undefined;
    const body: PolicyTestBody = { principal: { kind: 'human', user_id: 'dry-run', roles: [tRole] }, action: tAction,
      resource: { type: tType, id: tId, ...(ns ? { attributes: { namespace: ns } } : {}) },
      include_registry: true, draft_policies: tDraft && pol && !pol.readOnly ? [pol.cedar] : [], ...(ns ? { draft_namespace: ns } : {}) };
    const r = await securityPolicies.test(body);
    busy = ''; testRes = r.ok ? r.data : { error: r.error ?? `HTTP ${r.status}` };
  }

  // Decisions: the log entries this policy decided (by its id).
  let decisions = $state<any[] | null>(null);
  $effect(() => { if (tab === 'decisions' && pol && decisions === null) untrack(async () => {
    const r = await audit.recent(1000, false);
    const id = pol!.readOnly ? 'baseline' : pol!.name;
    decisions = r.ok ? (r.data?.records ?? []).filter((d: any) => (d.policy_ids ?? []).some((x: string) => x.includes(id))) : [];
  }); });

  // ── New policy ──
  let nName = $state(''); let nNs = $state(getProject() || 'default'); let nDesc = $state('');
  let nCedar = $state('// Forbid overrides permit; deny by default.\n// Example: block one tool for everyone in this project.\nforbid(principal, action, resource)\nwhen { resource is Tool && resource has name && resource.name == "some_tool" };\n');
  let nErr = $state('');
  async function openNew() { await ensureEdition(); if (!isInfosec()) { denied = 'Security is for SuperAdmin and SecurityAdmin.'; view = 'policy'; pol = null; return; } view = 'new'; pol = null; shown = ''; nName = ''; nErr = ''; nNs = getProject() || 'default'; }
  async function create() {
    nErr = '';
    const r = await securityPolicies.save(nNs, nName.trim(), nCedar, nDesc || undefined);
    if (!r.ok) { nErr = r.error ?? 'Not created.'; return; }
    await securityPolicies.reload(); bumpItems();
    navigateToDetail('security', 'Security', `policy:${nNs}/${nName.trim()}`);
  }

  // ── Identity ──
  let idpStatus = $state<any>(null);
  let users = $state<any[]>([]);
  async function loadIdp() {
    const st = await idp.status();
    if (st.status === 403) { denied = 'Security is for SuperAdmin and SecurityAdmin.'; return; }
    idpStatus = st.ok ? st.data : { error: st.error };
    if (idpStatus?.connected) { const u = await idp.users(); users = u.ok ? (u.data?.users ?? []) : []; }
  }
  let uEdit = $state<{ id: string | null; email: string; name: string; password: string; temporary: boolean; roles: string } | null>(null);
  let uMsg = $state('');
  async function editUser(u: any | null) {
    uMsg = '';
    if (!u) { uEdit = { id: null, email: '', name: '', password: '', temporary: true, roles: '' }; return; }
    uEdit = { id: u.id, email: u.email ?? u.username ?? '', name: '', password: '', temporary: false, roles: '' };
    const d = await idp.getUser(u.id);
    if (d.ok && uEdit?.id === u.id) { uEdit.name = d.data?.name ?? ''; uEdit.roles = (d.data?.roles ?? []).join(', '); }
  }
  async function saveUser() {
    if (!uEdit) return;
    const roles = uEdit.roles.split(',').map((r) => r.trim()).filter(Boolean);
    if (!/^[^\s@]+@[^\s@]+\.[^\s@]+$/.test(uEdit.email.trim())) { uMsg = 'The email is the sign-in name.'; return; }
    if (!uEdit.id && !uEdit.password) { uMsg = 'Give a first password.'; return; }
    busy = 'user';
    const r = uEdit.id
      ? await idp.updateUser(uEdit.id, { name: uEdit.name || undefined, password: uEdit.password || undefined, temporary: uEdit.temporary, roles })
      : await idp.createUser({ username: uEdit.email.trim(), email: uEdit.email.trim(), name: uEdit.name || undefined, password: uEdit.password, temporary: uEdit.temporary, roles });
    busy = '';
    if (!r.ok) { uMsg = r.error ?? 'Not saved.'; return; }
    uEdit = null; void loadIdp();
  }
  async function deleteUser(u: any) {
    if (!confirm(`Delete ${u.email ?? u.username}? They can no longer sign in.`)) return;
    const r = await idp.deleteUser(u.id);
    if (r.ok) void loadIdp(); else uMsg = r.error ?? 'Not deleted.';
  }

  const POLICY_STEPS = [
    { id: 'overview', label: 'Overview', icon: '<rect x="2" y="2" width="12" height="12" rx="2"/><path d="M5 6h6M5 9h4"/>', hint: 'What it permits and forbids, and where it applies' },
    { id: 'policy', label: 'Policy', icon: '<path d="M8 1l5.5 2.2v4c0 3.5-2.3 6.4-5.5 7.6C4.8 13.6 2.5 10.7 2.5 7.2v-4L8 1z"/>', hint: 'Its Cedar — saving applies it live, no restart' },
    { id: 'test', label: 'Test', icon: '<polygon points="4,2 13,8 4,14"/>', hint: 'A dry run: what would be decided, with this text in place' },
    { id: 'decisions', label: 'Decisions', icon: '<rect x="2" y="1" width="12" height="14" rx="1.5"/><path d="M5 5h6M5 8h6M5 11h3"/>', hint: 'What it decided recently' },
  ];
  const ID_STEPS = [
    { id: 'overview', label: 'Overview', icon: '<rect x="2" y="2" width="12" height="12" rx="2"/><path d="M5 6h6M5 9h4"/>', hint: 'The sign-in provider, and how its roles become A3T roles' },
    { id: 'users', label: 'Users', icon: '<circle cx="8" cy="5" r="2.5"/><path d="M3 13a5 5 0 0110 0"/>', hint: 'Who can sign in, and their roles' },
  ];
  const when = (ms: number) => (ms ? new Date(ms).toLocaleString(undefined, { month: 'short', day: 'numeric', hour: '2-digit', minute: '2-digit', second: '2-digit' }) : '—');
</script>

{#snippet ceNote()}
  <div class="note">Policies are an Enterprise feature. A Community runtime allows everything; connect an EE runtime (Settings → Runtime) to author and enforce them.</div>
{/snippet}

<SlidePanel open={view !== null} onclose={close}>
  {#if view === 'new'}
    <div class="panel-layout">
      <PropertiesTree icon="+" color="#5530a8" name={nName || 'New policy'} meta={`policy · ${nNs}`} nodes={[]} steps={[{ id: 'new', label: 'Policy', hint: '', icon: POLICY_STEPS[1].icon }]} activeNode="new" onselect={() => {}} />
      <div class="panel-col"><div class="panel-content"><div class="pf">
        <WizardHead step={{ id: 'new', label: 'New policy', hint: 'A Cedar policy for one project — created, then applied live' }} />
        <section class="agt">
          {#if edition === 'ce'}{@render ceNote()}{/if}
          <div class="card">
            <div class="card-head"><h3>1 · Name</h3><span>the project it governs</span></div>
            <div class="fields">
              <label class="fld">Name<input class="mono" bind:value={nName} placeholder="no-external-llm" /></label>
              <label class="fld">Project<select bind:value={nNs}>{#each [...new Set([nNs, ...namespacesFor('owned'), ...allProjectNamespaces(), 'default'].filter(Boolean))] as ns (ns)}<option value={ns}>{ns}</option>{/each}</select></label>
              <label class="fld full">What it is for<input bind:value={nDesc} placeholder="Agents in this project may not call models outside it" /></label>
            </div>
          </div>
          <div class="card">
            <div class="card-head"><h3>2 · Cedar</h3><span>forbid overrides permit; anything not permitted is denied</span></div>
            <textarea class="cedar" rows="12" spellcheck="false" bind:value={nCedar}></textarea>
          </div>
          {#if nErr}<div class="note bad">{nErr}</div>{/if}
          <div class="acts"><button class="btn2 primary" disabled={!/^[a-z0-9][a-z0-9-]{1,62}$/.test(nName.trim()) || edition === 'ce'} onclick={create}>Create and apply</button><button class="btn2 quiet" onclick={close}>Cancel</button></div>
        </section>
      </div></div></div>
    </div>

  {:else if view === 'policy'}
    <div class="panel-layout">
      <PropertiesTree icon={pol?.readOnly ? 'BL' : 'P'} color="#5530a8" name={pol?.name ?? 'Policy'} meta={pol ? (pol.readOnly ? 'compiled baseline · read only' : `policy · ${pol.namespace}`) : ''} nodes={[]} steps={POLICY_STEPS} activeNode={tab} onselect={(id) => (tab = id)} />
      <div class="panel-col"><div class="panel-content"><div class="pf">
        <WizardHead step={POLICY_STEPS.find((s) => s.id === tab)} />
        <section class="agt">
        {#if denied}<div class="note bad">{denied}</div>
        {:else if pol}
          {#if tab === 'overview'}
            <div class="card">
              <div class="card-head"><h3>1 · What it does</h3><span>{pol.readOnly ? 'compiled into the runtime' : `applies to project ${pol.namespace}`}</span></div>
              {#if pol.description}<p class="lead">{pol.description}</p>{/if}
              <div class="tiles">
                <div class="tile"><span class="v">{shape.permits}</span><span class="k">permit rules</span></div>
                <div class="tile" class:bad={shape.forbids > 0}><span class="v">{shape.forbids}</span><span class="k">forbid rules</span></div>
                <div class="tile"><span class="v">{shape.actions.length}</span><span class="k">actions named</span></div>
                <div class="tile"><span class="v">{shape.roles.length}</span><span class="k">roles named</span></div>
              </div>
              <ul class="rows">
                {#if shape.actions.length}<li><span class="lbl">Actions</span><span class="val mono">{shape.actions.join(', ')}</span><span></span></li>{/if}
                {#if shape.roles.length}<li><span class="lbl">Roles</span><span class="val">{shape.roles.join(', ')}</span><span></span></li>{/if}
                <li><span class="lbl">Where</span><span class="val">{pol.readOnly ? 'everywhere, beneath every project policy' : `project ${pol.namespace}`}</span><button class="btn2" onclick={() => (tab = 'policy')}>{pol.readOnly ? 'Read' : 'Edit'}</button></li>
              </ul>
            </div>
            <div class="card">
              <div class="card-head"><h3>2 · How decisions are made</h3><span>the same rule everywhere</span></div>
              <p class="dim">Every call is checked at its choke point: the compiled baseline and the project's policies together. Anything not permitted is denied, and a forbid beats any permit. Each decision is written to the Decision log.</p>
              <div class="acts"><button class="btn2" onclick={() => (tab = 'test')}>Try a decision</button><button class="btn2" onclick={() => navigateToDetail('logs', 'Decision log', 'all')}>Decision log</button></div>
            </div>
            {#if !pol.readOnly}
              <div class="card">
                <div class="card-head"><h3>3 · Delete</h3><span>its project falls back to the baseline</span></div>
                <div class="row"><input class="grow tin" type="text" bind:value={delName} placeholder="Type {pol.name} to delete it" />
                  <button class="btn2 danger" disabled={delName !== pol.name || !!busy} onclick={remove}>{busy === 'delete' ? 'Deleting…' : `Delete ${pol.name}`}</button></div>
              </div>
            {/if}

          {:else if tab === 'policy'}
            <div class="card">
              <div class="card-head"><h3>Cedar</h3><span>{pol.readOnly ? 'read only — compiled into the runtime' : 'forbid overrides permit; anything not permitted is denied'}</span></div>
              <textarea class="cedar" rows="20" spellcheck="false" bind:value={pol.cedar} readonly={pol.readOnly}></textarea>
              {#if !pol.readOnly}
                <div class="acts">
                  <button class="btn2 primary" disabled={!dirty || !!busy} onclick={() => save(true)}>{busy === 'save' || busy === 'apply' ? 'Applying…' : 'Save and apply'}</button>
                  <button class="btn2 quiet" disabled={!dirty || !!busy} onclick={() => (pol!.cedar = saved)}>Discard</button>
                  <button class="btn2 quiet" disabled={!dirty || !!busy} onclick={() => (tab = 'test')}>Test first</button>
                  {#if msg}<span class={/^Saved and applied/.test(msg) ? 'ok' : 'err'}>{msg}</span>{:else if dirty}<span class="dim">Not saved yet.</span>{/if}
                </div>
              {/if}
            </div>

          {:else if tab === 'test'}
            <div class="card">
              <div class="card-head"><h3>1 · The request</h3><span>who, doing what, to what</span></div>
              <div class="fields">
                <label class="fld">Role<select bind:value={tRole}>{#each ROLES as r (r)}<option>{r}</option>{/each}</select></label>
                <label class="fld">Action<select bind:value={tAction}>{#each ACTIONS as a (a)}<option>{a}</option>{/each}</select></label>
                <label class="fld">Resource type<select bind:value={tType}>{#each RES_TYPES as r (r)}<option>{r}</option>{/each}</select></label>
                <label class="fld">Resource<input class="mono" bind:value={tId} placeholder="list_flows" /></label>
              </div>
              {#if !pol.readOnly}<label class="fld chk"><input type="checkbox" bind:checked={tDraft} /> With this policy as written here {dirty ? '(unsaved)' : ''}</label>{/if}
              <div class="acts"><button class="btn2 primary" disabled={!!busy || edition === 'ce'} onclick={runTest}>{busy === 'test' ? 'Deciding…' : 'Decide'}</button>{#if edition === 'ce'}<span class="dim">Needs an EE runtime.</span>{/if}</div>
            </div>
            {#if testRes}
              <div class="card">
                <div class="card-head"><h3>2 · Decision</h3><span>nothing ran; nothing was recorded</span></div>
                {#if testRes.error}<div class="note bad">{testRes.error}</div>
                {:else}
                  {@const d = String(testRes.decision ?? testRes.result ?? '').toLowerCase()}
                  <div class="verdict {d.startsWith('allow') ? 'ok' : d.startsWith('ask') ? 'ask' : 'no'}">{d.startsWith('allow') ? 'Allowed' : d.startsWith('ask') ? 'Asks for approval' : 'Denied'}</div>
                  <ul class="rows">
                    {#if testRes.reason}<li><span class="lbl">Why</span><span class="val">{testRes.reason}</span><span></span></li>{/if}
                    {#if (testRes.policy_ids ?? []).length}<li><span class="lbl">Decided by</span><span class="val mono">{testRes.policy_ids.join(', ')}</span><span></span></li>{/if}
                  </ul>
                {/if}
              </div>
            {/if}

          {:else}
            <div class="card">
              <div class="card-head"><h3>Recent decisions</h3><span>naming this policy, on this FlowR replica</span></div>
              {#if decisions === null}<p class="dim">Reading the log…</p>
              {:else if decisions.length}
                <ul class="tl">{#each decisions.slice(0, 100) as d, i (i)}<li class="ev" class:err={d.decision === 'deny'}><span class="ic">{d.decision === 'deny' ? '✕' : d.decision === 'ask' ? '?' : '✓'}</span>
                  <div><div class="t">{d.principal_id}{d.on_behalf_of ? ` for ${d.on_behalf_of}` : ''}{d.impersonated_by ? ` · acted by ${d.impersonated_by}` : ''} · <span class="mono">{d.action}</span> {d.resource_type} <span class="mono">{d.resource_id}</span></div>{#if d.reason}<div class="m">{d.reason}</div>{/if}</div><time>{when(d.ts_ms)}</time></li>{/each}</ul>
              {:else}<p class="empty">Nothing recent names it. Allows usually name no policy; denials and approvals do.</p>{/if}
            </div>
          {/if}
        {/if}
        </section>
      </div></div></div>
    </div>

  {:else if view === 'identity'}
    <div class="panel-layout">
      <PropertiesTree icon="ID" color="#5530a8" name="Identity" meta="sign-in · platform-wide" nodes={[]} steps={ID_STEPS} activeNode={tab} onselect={(id) => (tab = id)} />
      <div class="panel-col"><div class="panel-content"><div class="pf">
        <WizardHead step={ID_STEPS.find((s) => s.id === tab)} />
        <section class="agt">
        {#if denied}<div class="note bad">{denied}</div>
        {:else if edition === 'ce'}<div class="note">Identity is an Enterprise feature: a Community runtime has no single sign-on.</div>
        {:else if tab === 'overview'}
          <div class="card">
            <div class="card-head"><h3>1 · Sign-in provider</h3><span>platform-wide — users and roles live in the SSO realm</span></div>
            <ul class="rows">
              <li><span class="lbl">State</span><span class="val">{#if !idpStatus}…{:else if idpStatus.connected}<span class="dot ok"></span> connected{:else}<span class="dot err"></span> not connected{idpStatus.error ? `: ${idpStatus.error}` : ''}{/if}</span><span></span></li>
              {#if idpStatus?.issuer}<li><span class="lbl">Issuer</span><span class="val mono">{idpStatus.issuer}</span><span></span></li>{/if}
              {#if idpStatus?.realm}<li><span class="lbl">Realm</span><span class="val mono">{idpStatus.realm}</span><span></span></li>{/if}
              <li><span class="lbl">Users</span><span class="val">{users.length}</span><button class="btn2" onclick={() => (tab = 'users')}>Manage</button></li>
            </ul>
          </div>
          <div class="card">
            <div class="card-head"><h3>2 · Roles</h3><span>realm roles become A3T roles at sign-in</span></div>
            <ul class="rows wide">
              <li><span class="lbl">SuperAdmin</span><span class="val">everything, every org — including Security and the Decision log</span><span></span></li>
              <li><span class="lbl">SecurityAdmin</span><span class="val">Security and the Decision log: policies, identity, every decision</span><span></span></li>
              <li><span class="lbl">Admin</span><span class="val">runs projects; does not see Security or the Decision log</span><span></span></li>
              <li><span class="lbl">Operator · Viewer</span><span class="val">work in their projects; read only for Viewer</span><span></span></li>
            </ul>
          </div>
        {:else if idpStatus && !idpStatus.connected}
          <div class="note warn">No sign-in provider is connected{idpStatus.error ? `: ${idpStatus.error}` : ''} — users and their roles live there. See Overview.</div>
        {:else}
          <div class="card">
            <div class="card-head"><h3>Users</h3><span>{users.length} in the realm</span><span class="grow"></span><button class="btn2 rm" onclick={() => editUser(null)}>+ User</button></div>
            {#if uEdit}
              <div class="review">
                <div class="fields">
                  <label class="fld">Email <small>the sign-in name</small><input class="mono" bind:value={uEdit.email} readonly={!!uEdit.id} /></label>
                  <label class="fld">Name<input bind:value={uEdit.name} /></label>
                  <label class="fld">{uEdit.id ? 'New password' : 'Password'} <small>{uEdit.id ? 'empty: unchanged' : ''}</small><input type="password" autocomplete="new-password" bind:value={uEdit.password} /></label>
                  <label class="fld">Roles <small>realm roles, comma-separated</small><input class="mono" bind:value={uEdit.roles} placeholder="operator" /></label>
                </div>
                <label class="fld chk"><input type="checkbox" bind:checked={uEdit.temporary} /> They change the password at first sign-in</label>
                <div class="acts"><button class="btn2 primary" disabled={busy === 'user'} onclick={saveUser}>{uEdit.id ? 'Save' : 'Create user'}</button><button class="btn2 quiet" onclick={() => (uEdit = null)}>Cancel</button>{#if uMsg}<span class="err">{uMsg}</span>{/if}</div>
              </div>
            {/if}
            <ul class="rows wide">
              {#each users as u (u.id)}
                <li><span class="lbl">{u.email ?? u.username}</span><span class="val">{u.name ?? ''} <span class="dim">{u.enabled === false ? '· disabled' : ''}</span></span>
                  <span class="acts"><button class="btn2" onclick={() => editUser(u)}>Edit</button><button class="btn2 quiet" onclick={() => deleteUser(u)}>Delete</button></span></li>
              {:else}<li><span class="dim">No users yet.</span><span></span><span></span></li>{/each}
            </ul>
          </div>
        {/if}
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
  .lead { margin: 0; font-size: 13.5px; color: var(--text-1); line-height: 1.55; }
  .cedar { width: 100%; box-sizing: border-box; padding: 12px 14px; border: 1px solid var(--border); border-radius: 8px; background: #111214; color: #f4f4f5; font: 12.5px/1.6 var(--font-mono); resize: vertical; }
  .tin { height: 34px; padding: 0 10px; border: 1px solid var(--border); border-radius: 8px; background: var(--bg-surface); font: 13px var(--font-sans); min-width: 0; }
  .verdict { align-self: flex-start; padding: 6px 14px; border-radius: 8px; font: 600 15px var(--font-sans); }
  .verdict.ok { background: color-mix(in srgb, var(--ok, #1a7158) 14%, transparent); color: var(--ok, #1a7158); }
  .verdict.no { background: color-mix(in srgb, var(--err) 12%, transparent); color: var(--err); }
  .verdict.ask { background: var(--warn-bg, #fbf0e0); color: var(--warn, #b45309); }
  .review { display: flex; flex-direction: column; gap: 10px; padding: 12px 14px; border: 1px solid var(--border); border-radius: 10px; background: var(--bg-page); }
  .fld.chk { flex-direction: row; align-items: center; gap: 8px; }
  .fld.chk input { width: auto; height: auto; }
</style>
