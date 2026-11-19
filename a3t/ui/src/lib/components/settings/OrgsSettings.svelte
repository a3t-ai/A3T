<script lang="ts">
  /// Settings → General → Organisations: which orgs exist (a SuperAdmin creates
  /// and deletes them) and where each one's flows run. Who is in an org, its
  /// sign-in and its policy files are its security: they live in Security ›
  /// the org (OrgSecurity), for its admins and security admins and for InfoSec.
  /// All of it is the flow_a3t_orgs flow, which checks the caller on the server.
  import { onMount } from 'svelte';
  import { userRole, currentUser } from '$lib/stores/auth';
  import { loadProjects } from '$lib/stores/context.svelte.ts';
  import { orgsApi, type OrgInfo } from '$lib/services/api';
  import { navigateToDetail } from '$lib/stores/panels.svelte.ts';

  const HANDLE = /^[a-z0-9]([a-z0-9-]{0,22}[a-z0-9])?$/;

  const isSuper = $derived($userRole === 'superadmin');
  let orgs = $state<OrgInfo[]>([]);
  let loading = $state(true);
  let message = $state('');
  let messageOk = $state(false);
  let busy = $state(false);
  let newHandle = $state('');
  let newName = $state('');
  function say(text: string, ok = false) { message = text; messageOk = ok; }

  async function refresh() {
    loading = true;
    const res = await orgsApi.list();
    loading = false;
    if (!res.ok) { say(res.error ?? 'Could not load organisations'); return; }
    orgs = res.data?.orgs ?? [];
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

  onMount(refresh);
  const openSecurity = (o: OrgInfo) => navigateToDetail('security', 'Security', `org:${o.handle}:members`);
</script>

<div class="card">
  <div class="card-head"><h2>Organisations</h2></div>
  <p class="card-sub">An organisation owns projects named <span class="mono">&lt;org&gt;-&lt;project&gt;</span>. Who is in it, how they sign in and its policy files are in <b>Security</b>, under the org, for its admins and security admins. {isSuper ? '' : 'Creating and deleting organisations needs the SuperAdmin role.'}</p>

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
          <tr>
            <td>{o.display_name}<div class="dim mono">{o.handle}</div></td>
            <td>{o.members.length}</td>
            <td>{o.my_role ?? (isSuper ? 'superadmin' : '—')}</td>
            <td>
              <select value={o.runtime ?? 'shared'} disabled={!o.can_admin || busy} onchange={(e) => setRuntime(o, e.currentTarget.value as 'isolated' | 'shared')}>
                <option value="shared">Shared</option>
                <option value="isolated">Isolated</option>
              </select>
            </td>
            <td class="right">
              {#if o.can_audit}<button class="btn" onclick={() => openSecurity(o)} title="Members, sign-in and policy files, in Security">Members & sign-in →</button>{/if}
              {#if isSuper}<button class="btn danger" onclick={() => remove(o)} disabled={busy || o.id === 'org-default'} title={o.id === 'org-default' ? "The default organisation can't be deleted" : ''}>Delete</button>{/if}
            </td>
          </tr>
        {/each}
      </tbody>
    </table>
  {/if}
</div>

<style>
  .card { background: var(--bg-surface); border: 1px solid var(--border); border-radius: var(--r); padding: 18px 20px; margin-bottom: 16px; display: flex; flex-direction: column; gap: 12px; }
  .card-head h2 { margin: 0; font-family: var(--font-serif); font-size: 18px; font-weight: 500; }
  .card-sub { margin: 0; font-size: 12.5px; color: var(--text-3); line-height: 1.5; }
  .mono { font-family: var(--font-mono); }
  .dim { color: var(--text-4); font-size: 11.5px; margin: 0; }
  .msg { padding: 8px 12px; border-radius: var(--r-sm); background: var(--err-bg, #fde8e8); color: var(--err, #b42318); font-size: 12.5px; }
  .msg.ok { background: var(--spec-bg); color: var(--spec); }
  .row { display: flex; gap: 8px; flex-wrap: wrap; }
  .row input { flex: 1; min-width: 160px; }
  .btn { padding: 6px 12px; border-radius: var(--r-sm); border: 1px solid var(--border); background: var(--bg-surface); font-size: 12px; color: var(--text-1); cursor: pointer; }
  .btn:hover:not(:disabled) { border-color: var(--spec); color: var(--spec); }
  .btn.primary { background: var(--spec); border-color: var(--spec); color: white; }
  .btn.danger:hover:not(:disabled) { border-color: var(--err, #d44); color: var(--err, #d44); }
  .btn:disabled { opacity: 0.5; cursor: default; }
  .tbl { width: 100%; border-collapse: collapse; font-size: 12.5px; }
  .tbl th { text-align: left; font-weight: 550; color: var(--text-4); font-size: 11px; padding: 6px 8px; border-bottom: 1px solid var(--border); }
  .tbl td { padding: 8px; border-bottom: 1px solid var(--divider, var(--border)); vertical-align: middle; }
  .right { text-align: right; white-space: nowrap; }
  .right .btn + .btn { margin-left: 6px; }
</style>
