<script lang="ts">
  /// Sharing: who can see the agent, and what each namespace may do with it: call
  /// it, inherit from it, fork it, merge back into it. "Public" is the target "*".
  /// Sharing never hands over memory. FlowR does not check these rights on fork or
  /// merge yet (requested); the dash and the control plane do.
  import { control, rightsOf, type Rights, type Share } from './controlPlane';
  import type { AgentRec } from './agentData';

  let { agent, canEdit }: { agent: AgentRec; canEdit: boolean } = $props();

  let shares = $state<Share[]>([]);
  let loaded = $state(false);
  let error = $state('');
  let busy = $state('');
  let addNs = $state('');

  const isPublic = $derived(shares.some((s) => s.target === '*'));
  const named = $derived(shares.filter((s) => s.target !== '*'));
  const reach = $derived(isPublic ? 'public' : named.length ? 'namespaces' : 'private');
  const RIGHTS: (keyof Rights)[] = ['call', 'inherit', 'fork', 'merge_back'];
  const LABEL: Record<keyof Rights, string> = { call: 'Call', inherit: 'Inherit', fork: 'Fork', merge_back: 'Merge back' };

  async function load(id: string) {
    loaded = false; error = '';
    const r = await control.shares(id);
    if (r.ok) shares = r.data?.shares ?? []; else error = r.error ?? 'Sharing could not be read.';
    loaded = true;
  }
  $effect(() => { load(agent.id); });

  async function set(target: string, rights: Rights) {
    busy = target; error = '';
    const r = await control.share(agent.id, target, rights);
    busy = '';
    if (!r.ok) { error = r.error ?? 'Not saved.'; return; }
    await load(agent.id);
  }
  async function remove(target: string) {
    busy = target; error = '';
    const r = await control.unshare(agent.id, target);
    busy = '';
    if (!r.ok) { error = r.error ?? 'Not removed.'; return; }
    await load(agent.id);
  }
  function toggle(s: Share, k: keyof Rights) { const r = rightsOf(s); r[k] = !r[k]; set(s.target, r); }
  async function add() {
    const ns = addNs.trim();
    if (!ns || shares.some((s) => s.target === ns)) return;
    await set(ns, { call: true, inherit: false, fork: false, merge_back: false });
    addNs = '';
  }
  const setPublic = (on: boolean) => (on ? set('*', { call: true, inherit: true, fork: true, merge_back: false }) : remove('*'));
</script>

<section class="agt">
  <p class="dim">Sharing never hands over its memory.</p>

  <div class="reach">
    <div class="opt" class:sel={reach === 'private'}><b>This project</b><span>Only {agent.namespace ?? 'default'} uses it.</span></div>
    <div class="opt" class:sel={reach === 'namespaces'}><b>Chosen namespaces</b><span>Those below, with the rights you give each.</span></div>
    <div class="opt" class:sel={reach === 'public'}>
      <b>Public</b><span>Listed for every org on A3T.</span>
      {#if canEdit}<button class="link" disabled={busy === '*'} onclick={() => setPublic(!isPublic)}>{isPublic ? 'Make it not public' : 'Make it public'}</button>{/if}
    </div>
  </div>

  <div class="card">
    <div class="card-head"><h3>Namespaces</h3><span>rights per namespace</span></div>
    {#if !loaded}
      <p class="dim">Reading…</p>
    {:else}
      <table>
        <thead><tr><th scope="col">Namespace</th>{#each RIGHTS as k}<th scope="col">{LABEL[k]}</th>{/each}<th scope="col">Since</th><th></th></tr></thead>
        <tbody>
          {#each named as s (s.target)}
            {@const r = rightsOf(s)}
            <tr>
              <th scope="row" class="mono">{s.target}</th>
              {#each RIGHTS as k}
                <td><input type="checkbox" checked={r[k]} disabled={!canEdit || busy === s.target} aria-label={`${LABEL[k]} for ${s.target}`} onchange={() => toggle(s, k)} /></td>
              {/each}
              <td class="dim">{s.granted_at?.slice(0, 10)}</td>
              <td>{#if canEdit}<button class="link" onclick={() => remove(s.target)}>Remove</button>{/if}</td>
            </tr>
          {:else}
            <tr><td colspan="7" class="dim">Not shared with any namespace.</td></tr>
          {/each}
        </tbody>
      </table>
      {#if canEdit}
        <div class="row">
          <label class="sr" for="share-ns">Add a namespace</label>
          <input id="share-ns" class="grow" placeholder="Add a namespace" bind:value={addNs} onkeydown={(e) => e.key === 'Enter' && add()} />
          <button class="btn2" disabled={!addNs.trim()} onclick={add}>Add</button>
        </div>
      {/if}
    {/if}
    {#if error}<p class="err">{error}</p>{/if}
  </div>

  <div class="note"><b>Not yet:</b> <span>FlowR doesn't check these rights itself yet when another namespace forks or merges; the dash and the control plane do (requested).</span></div>
</section>

<style>
  .reach { display: grid; grid-template-columns: repeat(3, minmax(0, 1fr)); gap: 8px; }
  .opt { display: flex; flex-direction: column; gap: 4px; padding: 11px 12px; border: 1px solid var(--border); border-radius: 8px; background: var(--bg-surface); font-size: 12px; }
  .opt b { font-size: 13px; color: var(--text-1); }
  .opt.sel { border: 2px solid var(--spec); background: var(--signal-bg); }
  .opt .link { align-self: flex-start; margin-top: 2px; }
  table { width: 100%; border-collapse: collapse; font-size: 12.5px; }
  th, td { padding: 8px 6px; border-top: 1px solid var(--divider); text-align: center; font-weight: 400; }
  thead th { border-top: 0; color: var(--text-3); font-weight: 500; }
  th[scope='row'], thead th:first-child { text-align: left; }
  input:not([type]) { height: 30px; padding: 0 10px; border: 1px solid var(--border-strong); border-radius: 6px; font: inherit; font-size: 12.5px; }
  input[type='checkbox'] { width: 16px; height: 16px; accent-color: var(--spec); }
  .sr { position: absolute; left: -9999px; }
</style>
