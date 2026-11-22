<script lang="ts">
  /// Keys — the tokens apps call this agent with. Every call to its data plane
  /// needs one (or a user's sign-in). A key reaches this agent only: POST /chat
  /// with a session of this agent, its stream and its webhook; it runs as the
  /// person who made it — never more than the roles they had then, and only
  /// while they still reach its project. A new key is shown once — FlowR keeps
  /// only its hash. It expires (90 days unless chosen); Rotate makes its
  /// successor and lets the old one run out after a grace period. Revoking
  /// stops it on the next call.
  import { agentSpaceApi, type AgentKey } from '$lib/services/api';
  import { ago, type AgentRec } from './agentData';
  import { copy, chatSnippet } from './agentApi';
  import { keepKey, peekKey, dropKey } from './keyOnce';

  let { agent, canEdit }: { agent: AgentRec; canEdit: boolean } = $props();

  /// A key just made (with the agent, or here), until its owner has saved it.
  let fresh = $state<{ name: string; key: string } | null>(null);

  let keys = $state<AgentKey[] | null>(null);
  let err = $state('');
  let busy = $state('');
  let name = $state('');
  let days = $state(90);
  let confirm = $state('');
  let copied = $state('');

  async function load(id: string) {
    keys = null; err = '';
    // Its keys are for whoever may change it (the server refuses anyone else).
    if (!canEdit) return;
    const r = await agentSpaceApi.keys(id);
    if (agent.id !== id) return;
    if (r.ok) keys = r.data?.keys ?? []; else err = r.error ?? 'Its keys could not be read.';
  }
  $effect(() => { fresh = peekKey(agent.id); load(agent.id); });
  function saved() { dropKey(agent.id); fresh = null; }

  async function make() {
    busy = 'make'; err = '';
    const r = await agentSpaceApi.makeKey(agent.id, name.trim() || 'key', days);
    busy = '';
    if (!r.ok || !r.data?.key) { err = r.error ?? 'No key made.'; return; }
    keepKey(agent.id, { name: r.data.name, key: r.data.key });
    fresh = peekKey(agent.id);
    name = '';
    await load(agent.id);
  }
  async function revoke(k: AgentKey) {
    busy = k.id; err = '';
    const r = await agentSpaceApi.revokeKey(agent.id, k.id);
    busy = ''; confirm = '';
    if (!r.ok) { err = r.error ?? 'Not revoked.'; return; }
    await load(agent.id);
  }
  async function rotate(k: AgentKey) {
    busy = `rot:${k.id}`; err = '';
    const r = await agentSpaceApi.rotateKey(agent.id, k.id);
    busy = '';
    if (!r.ok || !r.data?.key) { err = r.error ?? 'Not rotated.'; return; }
    keepKey(agent.id, { name: r.data.name, key: r.data.key });
    fresh = peekKey(agent.id);
    await load(agent.id);
  }
  const expiry = (k: AgentKey) => {
    if (!k.expires_at) return '—';
    const ms = Date.parse(k.expires_at) - Date.now();
    if (ms <= 0) return 'expired';
    const d = Math.floor(ms / 86_400_000);
    return d >= 1 ? `in ${d} day${d === 1 ? '' : 's'}` : `in ${Math.max(1, Math.round(ms / 3_600_000))} h`;
  };
  async function cp(what: string, text: string) { if (await copy(text)) { copied = what; setTimeout(() => (copied = ''), 1500); } }

  const gone = (k: AgentKey) => !!k.revoked_at || (!!k.expires_at && Date.parse(k.expires_at) <= Date.now());
  const live = $derived((keys ?? []).filter((k) => !gone(k)));
  const revoked = $derived((keys ?? []).filter(gone));
  const curl = $derived(chatSnippet('curl', agent.id).replace('$A3T_AGENT_KEY', fresh?.key ?? '$A3T_AGENT_KEY'));
</script>

<section class="agt">
  {#if fresh}
    <div class="card fresh">
      <div class="card-head"><h3>Key "{fresh.name}"</h3><span>copy it now — it is not shown again</span></div>
      <div class="ep"><span class="verb">KEY</span><span class="path">{fresh.key}</span>
        <button class="btn2 primary" onclick={() => cp('key', fresh!.key)}>{copied === 'key' ? 'Copied' : 'Copy'}</button></div>
      <p class="dim">Call it with this key:</p>
      <pre class="code">{curl}<button class="btn2 copy" onclick={() => cp('curl', curl)}>{copied === 'curl' ? 'Copied' : 'Copy'}</button></pre>
      <div class="acts"><button class="link" onclick={saved}>I have saved it — hide it</button></div>
    </div>
  {/if}

  <div class="card">
    <div class="card-head"><h3>Keys</h3><span>for apps that call it — every call needs a token</span></div>
    <p class="dim">A key calls <b>{agent.name}</b> only: <span class="mono">POST /chat</span> with <span class="mono">session_id "{agent.id}:&lt;conversation&gt;"</span>, its stream and its webhook. It runs as whoever made it — never more than the roles they had then, and only while they still reach project <span class="mono">{agent.namespace}</span> — and can't change the agent or reach anything else. It expires; rotate it before it does.</p>
    {#if canEdit}
      <div class="row">
        <input type="text" class="grow" bind:value={name} placeholder="What uses it, e.g. web-app or billing-service" />
        <select bind:value={days} aria-label="Expires after"><option value={30}>30 days</option><option value={90}>90 days</option><option value={180}>180 days</option><option value={365}>a year</option></select>
        <button class="btn2 primary" disabled={!!busy} onclick={make}>{busy === 'make' ? 'Making…' : 'New key'}</button>
      </div>
    {:else}<p class="dim">Its keys are shown to, and made by, those who can change project <b class="mono">{agent.namespace}</b>.</p>{/if}
    {#if err}<p class="err">{err}</p>{/if}
    {#if !canEdit}
    {:else if !keys}<p class="dim">…</p>
    {:else if live.length}
      <table class="tbl">
        <thead><tr><th>Name</th><th>Key</th><th>Made</th><th>Last used</th><th>Expires</th><th></th></tr></thead>
        <tbody>
          {#each live as k (k.id)}
            <tr>
              <td>{k.name}</td><td class="mono">{k.prefix}…</td>
              <td><span class="mono">{k.created_by}</span>, {ago(k.created_at)}</td>
              <td>{k.last_used_at ? ago(k.last_used_at) : 'never'}</td>
              <td>{expiry(k)}</td>
              <td class="end">
                {#if confirm === k.id}
                  <button class="btn2 danger" disabled={!!busy} onclick={() => revoke(k)}>{busy === k.id ? 'Revoking…' : 'Revoke it'}</button>
                  <button class="link" onclick={() => (confirm = '')}>Cancel</button>
                {:else if canEdit}<button class="btn2" disabled={!!busy} onclick={() => rotate(k)} title="A new key now; this one keeps working for 24 hours">{busy === `rot:${k.id}` ? 'Rotating…' : 'Rotate'}</button>
                  <button class="btn2" onclick={() => (confirm = k.id)}>Revoke</button>{/if}
              </td>
            </tr>
          {/each}
        </tbody>
      </table>
    {:else}
      <p class="note warn">No key: apps can't call it. Make one above.</p>
    {/if}
  </div>

  {#if revoked.length}
    <div class="card">
      <div class="card-head"><h3>Revoked or expired</h3><span>calls with these are refused</span></div>
      <table class="tbl">
        <thead><tr><th>Name</th><th>Key</th><th>Made</th><th>Ended</th></tr></thead>
        <tbody>{#each revoked as k (k.id)}<tr><td>{k.name}</td><td class="mono">{k.prefix}…</td><td><span class="mono">{k.created_by}</span>, {ago(k.created_at)}</td><td>{k.revoked_at ? `revoked ${ago(k.revoked_at)}` : `expired ${ago(k.expires_at ?? '')}`}</td></tr>{/each}</tbody>
      </table>
    </div>
  {/if}
</section>

<style>
  .fresh { border-color: var(--spec); background: var(--spec-bg); }
  .grow { flex: 1 1 auto; }
  .end { text-align: right; white-space: nowrap; }
  .mono { font-family: var(--font-mono); font-size: 0.92em; }
  pre.code { padding-right: 80px; overflow-wrap: anywhere; white-space: pre-wrap; }
</style>
