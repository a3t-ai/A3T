<script lang="ts">
  /// Settings → General → Organisations. A superadmin creates and deletes
  /// organisations and manages who is in each (member, admin, security admin);
  /// an org's own admins can manage its members too. Users are independent: one
  /// person can be in several orgs. All of it is the flow_a3t_orgs flow, which checks
  /// the caller on the server.
  import { onMount } from 'svelte';
  import { userRole, currentUser } from '$lib/stores/auth';
  import { loadProjects } from '$lib/stores/context.svelte.ts';
  import { orgsApi, loadDirectory, type OrgInfo, type OrgRole, type OrgSso, type DirectoryUser } from '$lib/services/api';

  const ROLES: OrgRole[] = ['member', 'admin', 'securityadmin'];
  const HANDLE = /^[a-z0-9]([a-z0-9-]{0,22}[a-z0-9])?$/;

  const isSuper = $derived($userRole === 'superadmin');
  let orgs = $state<OrgInfo[]>([]);
  let loading = $state(true);
  let message = $state('');
  let messageOk = $state(false);
  let busy = $state(false);
  let selected = $state('');
  let newHandle = $state('');
  let newName = $state('');
  let directory = $state<DirectoryUser[]>([]);
  let addWho = $state('');
  let addRole = $state<OrgRole>('member');

  const org = $derived(orgs.find((o) => o.handle === selected));

  // Sign-in through the org's own identity provider: the form for the org shown.
  let ssoFor = '';
  let sso = $state({ alias: '', domains: '', map: '', dflt: 'member' as 'member' | '' });
  $effect(() => {
    if (!org || ssoFor === org.handle) return;
    ssoFor = org.handle;
    const s = org.sso;
    sso = {
      alias: s?.idp_alias ?? '',
      domains: (s?.domains ?? []).join(', '),
      map: Object.entries(s?.role_map ?? {}).map(([g, r]) => `${g} = ${r}`).join('\n'),
      dflt: s ? s.default_role : 'member',
    };
  });
  async function saveSso(o: OrgInfo) {
    const role_map: Record<string, OrgRole> = {};
    for (const line of sso.map.split('\n').map((l) => l.trim()).filter(Boolean)) {
      const [g, r] = line.split('=').map((x) => x.trim());
      if (!g || !ROLES.includes(r as OrgRole)) { say(`"${line}": write each line as group = member, admin or securityadmin`); return; }
      role_map[g] = r as OrgRole;
    }
    const body: OrgSso = { idp_alias: sso.alias.trim(), domains: sso.domains.split(',').map((d) => d.trim().toLowerCase()).filter(Boolean), role_map, default_role: sso.dflt };
    busy = true;
    const res = await orgsApi.setSso(o.handle, body);
    busy = false;
    if (!res.ok) { say(res.error ?? 'Could not save sign-in'); return; }
    say(body.idp_alias ? `${o.handle} now signs in through "${body.idp_alias}".` : `Sign-in through a provider is off for ${o.handle}.`, true);
    ssoFor = '';
    await refresh();
  }

  function say(text: string, ok = false) { message = text; messageOk = ok; }

  async function refresh() {
    loading = true;
    const res = await orgsApi.list();
    loading = false;
    if (!res.ok) { say(res.error ?? 'Could not load organisations'); return; }
    orgs = res.data?.orgs ?? [];
    if (selected && !orgs.some((o) => o.handle === selected)) selected = '';
  }

  async function create() {
    const h = newHandle.trim();
    if (!HANDLE.test(h)) { say('An org handle is 2–24 lowercase letters, digits and dashes.'); return; }
    busy = true;
    const res = await orgsApi.create(h, newName.trim() || h, { email: $currentUser?.email, name: $currentUser?.name });
    busy = false;
    if (!res.ok) { say(res.error ?? 'Create failed'); return; }
    newHandle = ''; newName = '';
    say(`Created ${h}. Its projects are named ${h}-<project>.`, true);
    await refresh();
    selected = h;
    void loadProjects(true);
  }

  async function remove(o: OrgInfo) {
    if (!confirm(`Delete organisation “${o.handle}”? Its members are removed; it must own no projects.`)) return;
    busy = true;
    const res = await orgsApi.remove(o.handle);
    busy = false;
    if (!res.ok) { say(res.error ?? 'Delete failed'); return; }
    say(`Deleted ${o.handle}.`, true);
    await refresh();
    void loadProjects(true);
  }

  async function setRuntime(o: OrgInfo, runtime: 'isolated' | 'shared') {
    busy = true;
    const res = await orgsApi.setRuntime(o.handle, runtime);
    busy = false;
    if (!res.ok) { say(res.error ?? 'Could not change the runtime policy'); await refresh(); return; }
    say(runtime === 'isolated'
      ? `${o.handle} now runs only on its own runtimes (label a3t.io/org=${o.handle}). Deployments elsewhere fail until it has one.`
      : `${o.handle} runs on the shared runtimes.`, true);
    await refresh();
  }

  async function addMember() {
    if (!org) return;
    const q = addWho.trim().toLowerCase();
    const u = directory.find((d) => d.email.toLowerCase() === q || d.id === addWho.trim());
    if (!u) { say('Pick someone from the directory — members are matched by account, not by typed email.'); return; }
    busy = true;
    const res = await orgsApi.setMember(org.handle, { user_id: u.id, email: u.email, name: u.name }, addRole);
    busy = false;
    if (!res.ok) { say(res.error ?? 'Could not add the member'); return; }
    addWho = '';
    say('');
    await refresh();
  }

  async function setRole(user_id: string, email: string | undefined, name: string | undefined, role: OrgRole) {
    if (!org) return;
    const res = await orgsApi.setMember(org.handle, { user_id, email, name }, role);
    if (!res.ok) say(res.error ?? 'Could not change the role');
    await refresh();
  }

  async function removeMember(user_id: string) {
    if (!org) return;
    const res = await orgsApi.removeMember(org.handle, user_id);
    if (!res.ok) say(res.error ?? 'Could not remove the member');
    await refresh();
  }

  onMount(async () => {
    await refresh();
    directory = await loadDirectory();
  });
</script>

<div class="card">
  <div class="card-head"><h2>Organisations</h2></div>
  <p class="card-sub">An organisation owns projects named <span class="mono">&lt;org&gt;-&lt;project&gt;</span>. People are independent of it: add and remove them here, as member, admin or security admin. Admins and security admins see the org's private projects; members see the rest. {isSuper ? '' : 'Creating and deleting organisations needs the SuperAdmin role.'}</p>

  {#if message}<div class="msg" class:ok={messageOk}>{message}</div>{/if}

  {#if isSuper}
    <div class="row">
      <input class="mono" bind:value={newHandle} placeholder="handle, e.g. acme" />
      <input bind:value={newName} placeholder="Display name" />
      <button class="btn primary" onclick={create} disabled={busy || !newHandle.trim()}>Create organisation</button>
    </div>
  {/if}

  {#if loading}
    <!-- the header bar shows loading -->
  {:else if orgs.length === 0}
    <p class="dim">{isSuper ? 'No organisations yet.' : "You aren't in any organisation."}</p>
  {:else}
    <table class="tbl">
      <thead><tr><th>Organisation</th><th>Members</th><th>Your role</th><th title="Isolated: its flows run only on its own runtimes (label a3t.io/org). Shared: on the platform's shared runtimes.">Runtime</th><th></th></tr></thead>
      <tbody>
        {#each orgs as o (o.id)}
          <tr class:sel={o.handle === selected}>
            <td><button class="link" onclick={() => (selected = o.handle === selected ? '' : o.handle)}>{o.display_name}</button><div class="dim mono">{o.handle}</div></td>
            <td>{o.members.length}</td>
            <td>{o.my_role ?? (isSuper ? 'superadmin' : '—')}</td>
            <td>
              <select value={o.runtime ?? 'shared'} disabled={!o.can_admin || busy} onchange={(e) => setRuntime(o, e.currentTarget.value as 'isolated' | 'shared')}>
                <option value="shared">Shared</option>
                <option value="isolated">Isolated</option>
              </select>
            </td>
            <td class="right">
              {#if o.can_admin}<button class="btn" onclick={() => (selected = o.handle)}>Members</button>{/if}
              {#if isSuper}<button class="btn danger" onclick={() => remove(o)} disabled={busy || o.id === 'org-default'} title={o.id === 'org-default' ? "The default organisation can't be deleted" : ''}>Delete</button>{/if}
            </td>
          </tr>
        {/each}
      </tbody>
    </table>
  {/if}
</div>

{#if org}
  <div class="card">
    <div class="card-head"><h2>{org.display_name} — members</h2></div>
    <table class="tbl">
      <thead><tr><th>Person</th><th>Role</th><th></th></tr></thead>
      <tbody>
        {#each org.members as m (m.user_id)}
          <tr>
            <td>{m.name || m.email || m.user_id}{#if m.source === 'sso'} <span class="pill" title="From the organisation's sign-in: set by their groups at every sign-in">SSO</span>{/if}<div class="dim mono">{m.email || m.user_id}</div></td>
            <td>
              <select value={m.role} disabled={!org.can_admin} onchange={(e) => setRole(m.user_id, m.email, m.name, e.currentTarget.value as OrgRole)}>
                {#each ROLES as r}<option value={r}>{r}</option>{/each}
              </select>
            </td>
            <td class="right">{#if org.can_admin}<button class="btn danger" onclick={() => removeMember(m.user_id)}>Remove</button>{/if}</td>
          </tr>
        {:else}
          <tr><td colspan="3" class="dim">No members yet.</td></tr>
        {/each}
      </tbody>
    </table>
    {#if org.can_admin}
      <div class="row">
        <input bind:value={addWho} list="org-users" placeholder="name or email" onkeydown={(e) => { if (e.key === 'Enter') addMember(); }} />
        <datalist id="org-users">{#each directory as u}<option value={u.email || u.id}>{u.name}{u.source === 'sso' ? ' (SSO)' : ''}</option>{/each}</datalist>
        <select bind:value={addRole}>{#each ROLES as r}<option value={r}>{r}</option>{/each}</select>
        <button class="btn primary" onclick={addMember} disabled={busy || !addWho.trim()}>Add</button>
      </div>
    {/if}
  </div>

  {#if org.can_admin}
    <div class="card">
      <div class="card-head"><h2>{org.display_name} — sign-in</h2></div>
      <p class="card-sub">People sign in through the organisation's own identity provider (Entra ID, Okta, Google, its own Keycloak…), connected to A3T's Keycloak under the name below. They land in this organisation only, with the role their groups give them, checked at every sign-in. People added by hand above are not changed by sign-in.</p>
      <div class="sso-grid">
        <label for="sso-alias">Identity provider</label>
        <input id="sso-alias" class="mono" bind:value={sso.alias} placeholder="its name in Keycloak, e.g. acme (empty: off)" />
        <label for="sso-domains">Email domains</label>
        <input id="sso-domains" bind:value={sso.domains} placeholder="acme.com, acme.co.uk" />
        <label for="sso-map">Groups → role</label>
        <textarea id="sso-map" class="mono" rows="3" bind:value={sso.map} placeholder={'a3t-admins = admin\nsecurity = securityadmin'}></textarea>
        <label for="sso-default">Anyone else</label>
        <select id="sso-default" bind:value={sso.dflt}><option value="member">member</option><option value="">no access</option></select>
      </div>
      <div class="row"><button class="btn primary" onclick={() => saveSso(org)} disabled={busy}>Save sign-in</button></div>
    </div>
  {/if}
{/if}

<style>
  .card { background: var(--bg-surface); border: 1px solid var(--border); border-radius: var(--r); padding: 18px 20px; margin-bottom: 16px; display: flex; flex-direction: column; gap: 12px; }
  .card-head h2 { margin: 0; font-family: var(--font-serif); font-size: 18px; font-weight: 500; }
  .card-sub { margin: 0; font-size: 12.5px; color: var(--text-3); line-height: 1.5; }
  .mono { font-family: var(--font-mono); }
  .dim { color: var(--text-4); font-size: 11.5px; margin: 0; }
  .msg { padding: 8px 12px; border-radius: var(--r-sm); background: var(--err-bg, #fde8e8); color: var(--err, #b42318); font-size: 12.5px; }
  .msg.ok { background: var(--spec-bg); color: var(--spec); }
  .row { display: flex; gap: 8px; flex-wrap: wrap; }
  .row input, .row select, .tbl select { padding: 7px 9px; border: 1px solid var(--border); border-radius: var(--r-sm); background: var(--bg-page); color: var(--text-1); font-size: 13px; }
  .row input { flex: 1; min-width: 160px; }
  .btn { padding: 6px 12px; border-radius: var(--r-sm); border: 1px solid var(--border); background: var(--bg-surface); font-size: 12px; color: var(--text-1); cursor: pointer; }
  .btn:hover:not(:disabled) { border-color: var(--spec); color: var(--spec); }
  .btn.primary { background: var(--spec); border-color: var(--spec); color: white; }
  .btn.danger:hover:not(:disabled) { border-color: var(--err, #d44); color: var(--err, #d44); }
  .btn:disabled { opacity: 0.5; cursor: default; }
  .tbl { width: 100%; border-collapse: collapse; font-size: 12.5px; }
  .tbl th { text-align: left; font-weight: 550; color: var(--text-4); font-size: 11px; padding: 6px 8px; border-bottom: 1px solid var(--border); }
  .tbl td { padding: 8px; border-bottom: 1px solid var(--divider, var(--border)); vertical-align: middle; }
  .tbl tr.sel td { background: var(--spec-bg); }
  .right { text-align: right; white-space: nowrap; }
  .right .btn + .btn { margin-left: 6px; }
  .link { border: none; background: none; padding: 0; color: var(--text-1); font-size: 13px; cursor: pointer; text-align: left; }
  .link:hover { color: var(--spec); }
  .sso-grid { display: grid; grid-template-columns: 140px 1fr; gap: 8px 12px; align-items: center; font-size: 13px; }
  .sso-grid label { color: var(--text-2); }
  .sso-grid textarea { font: inherit; font-size: 12.5px; padding: 6px 8px; border: 1px solid var(--border); border-radius: 6px; background: var(--bg-surface); color: var(--text-1); }
  .pill { font-size: 10.5px; padding: 1px 6px; border-radius: 99px; background: var(--bg-muted, #eeeae2); color: var(--text-2); }
  @container (max-width: 560px) { .sso-grid { grid-template-columns: 1fr; } }
</style>
