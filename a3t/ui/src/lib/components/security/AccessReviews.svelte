<script lang="ts">
  /// Access reviews (GOV-11): a snapshot of who holds what in the org — its
  /// members' org roles, people's project roles, its agents' live keys — that
  /// reviewers go through, keeping or revoking each. A revoke applies at once;
  /// every decision goes to the decision log. A review closes when every item
  /// is decided.
  import { untrack } from 'svelte';
  import { accessReviews, type AccessReview, type AccessReviewItem } from '$lib/services/api';

  let { org, canReview }: { org: string; canReview: boolean } = $props();
  let list = $state<AccessReview[] | null>(null);
  let open = $state<{ review: AccessReview; items: AccessReviewItem[] } | null>(null);
  let err = $state('');
  let busy = $state('');
  let name = $state('');
  let kind = $state('');
  async function load() {
    err = '';
    const r = await accessReviews.list(org);
    if (!r.ok) { err = r.error ?? 'Reviews could not be read.'; return; }
    list = r.data?.reviews ?? [];
  }
  $effect(() => { org; untrack(() => void load()); });
  async function start() {
    busy = 'new';
    const r = await accessReviews.create(org, name.trim() || `Review ${new Date().toLocaleDateString()}`);
    busy = '';
    if (!r.ok) { err = r.error ?? 'Not started.'; return; }
    name = '';
    await load();
    await show(r.data!.id);
  }
  async function show(id: string) {
    const r = await accessReviews.get(org, id);
    if (r.ok) open = r.data!; else err = r.error ?? 'It could not be read.';
  }
  async function decide(it: AccessReviewItem, decision: 'keep' | 'revoke') {
    if (!open) return;
    busy = it.id;
    const r = await accessReviews.decide(org, open.review.id, it.id, decision);
    busy = '';
    if (!r.ok) { err = r.error ?? 'Not decided.'; return; }
    await show(open.review.id);
  }
  async function close() {
    if (!open) return;
    busy = 'close';
    const r = await accessReviews.close(org, open.review.id);
    busy = '';
    if (!r.ok) { err = r.error ?? 'Not closed.'; return; }
    await load(); await show(open.review.id);
  }
  const KIND: Record<string, string> = { org_role: 'Org role', project_role: 'Project role', agent_key: 'Agent key' };
  const shown = $derived((open?.items ?? []).filter((i) => !kind || i.kind === kind));
  const pending = $derived((open?.items ?? []).filter((i) => i.decision === 'pending').length);
</script>

{#if err}<div class="note bad">{err}</div>{/if}
<div class="card">
  <div class="card-head"><h3>1 · Reviews</h3><span>who holds what, confirmed or revoked by a person</span></div>
  {#if canReview}
    <div class="acts"><input class="tin grow" bind:value={name} placeholder="Q4 access review" aria-label="Review name" /><button class="btn2 primary" disabled={busy === 'new'} onclick={start}>{busy === 'new' ? 'Taking the snapshot…' : 'Start a review'}</button></div>
  {/if}
  {#if !list}<p class="dim">Reading…</p>
  {:else if !list.length}<p class="dim">No reviews yet.</p>
  {:else}
    <ul class="rows wide">
      {#each list as r (r.id)}
        <li><span class="lbl">{r.status}</span><span class="val">{r.name} · {r.items} items, {r.pending} to decide, {r.revoked} revoked · started {new Date(r.created_at).toLocaleDateString()} by {r.created_by}</span>
          <button class="btn2" onclick={() => show(r.id)}>Open</button></li>
      {/each}
    </ul>
  {/if}
</div>
{#if open}
  <div class="card">
    <div class="card-head"><h3>2 · {open.review.name}</h3><span>{pending} to decide of {open.items.length}</span><span class="grow"></span>
      {#if canReview && open.review.status === 'open'}<button class="btn2" disabled={pending > 0 || busy === 'close'} title={pending ? 'Decide every item first' : ''} onclick={close}>Close the review</button>{/if}</div>
    <div class="acts"><select class="tin" bind:value={kind} aria-label="Kind"><option value="">Everything</option><option value="org_role">Org roles</option><option value="project_role">Project roles</option><option value="agent_key">Agent keys</option></select></div>
    <table class="tbl">
      <thead><tr><th>What</th><th>Who</th><th>Holds</th><th>Decision</th><th></th></tr></thead>
      <tbody>
        {#each shown as it (it.id)}
          <tr>
            <td>{KIND[it.kind] ?? it.kind}</td>
            <td class="mono">{it.subject}</td>
            <td>{it.detail}</td>
            <td>{it.decision === 'pending' ? '—' : `${it.decision} · ${it.decided_by}`}</td>
            <td class="end">
              {#if canReview && open.review.status === 'open' && it.decision === 'pending'}
                <button class="btn2" disabled={busy === it.id} onclick={() => decide(it, 'keep')}>Keep</button>
                <button class="btn2 danger" disabled={busy === it.id} onclick={() => decide(it, 'revoke')}>Revoke</button>
              {/if}
            </td>
          </tr>
        {/each}
      </tbody>
    </table>
    <p class="dim">Revoke applies at once: the membership, the project role or the key is removed, and the decision goes to the decision log.</p>
  </div>
{/if}

<style>
  .grow { flex: 1 1 auto; }
  .tin { height: 34px; padding: 0 10px; border: 1px solid var(--border); border-radius: 8px; background: var(--bg-surface); color: var(--text-1); font: 13px var(--font-sans); min-width: 0; }
  .tin::placeholder { color: var(--text-4); }
  .tin:focus { outline: none; border-color: var(--accent, #5530a8); }
  .end { text-align: right; white-space: nowrap; }
</style>
