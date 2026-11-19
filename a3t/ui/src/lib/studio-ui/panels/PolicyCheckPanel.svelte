<script lang="ts">
  /// Validate (right of a policy file): does the org's set of enforced files
  /// work with this text in place? Checked on the server (the same check a save
  /// passes), again a moment after each edit, and on demand. Nothing is saved.
  import { untrack } from 'svelte';
  import { orgPolicy, type Doc, type Workspace } from '$lib/studio-core';

  let { doc }: { doc: Doc | null; workspace: Workspace } = $props();

  const org = $derived(doc ? orgPolicy.orgOf(doc.ref.project) : '');
  let res = $state<{ ok: boolean; checked: string[]; errors?: string[] } | null>(null);
  let err = $state('');
  let busy = $state(false);
  let checkedText = $state<string | null>(null);
  const stale = $derived(!!doc && checkedText !== null && checkedText !== (doc.text ?? ''));
  const enforced = $derived(doc?.data?.file ? doc.data.file.enforced !== false : true);

  async function run() {
    if (!doc || !org) return;
    const text = doc.text ?? '';
    busy = true; err = '';
    const r = await orgPolicy.validate(org, [{ path: doc.ref.name, content: text }]);
    busy = false;
    if (!r.ok) { err = r.error; res = null; return; }
    res = r.data; checkedText = text;
  }
  // On opening, and shortly after each edit.
  let timer: ReturnType<typeof setTimeout> | undefined;
  $effect(() => {
    void doc?.address; void doc?.text;
    untrack(() => { clearTimeout(timer); timer = setTimeout(run, 700); });
    return () => clearTimeout(timer);
  });
</script>

<div class="pc">
  {#if !doc || !org}<p class="dim">Open an org's policy file to check it.</p>
  {:else}
    <div class="head">
      {#if busy && !res}<span class="dim">Checking…</span>
      {:else if err}<span class="bad">Couldn't check: {err}</span>
      {:else if res?.ok}<span class="good">✓ Works</span><span class="dim">with {org}'s {res.checked.length - 1} other enforced file{res.checked.length === 2 ? '' : 's'}</span>
      {:else if res}<span class="bad">✕ Won't be saved</span>
      {/if}
      <button class="btn" disabled={busy} onclick={run}>{busy ? 'Checking…' : 'Validate again'}</button>
    </div>
    {#if stale}<p class="dim">Edited since the last check…</p>{/if}
    {#if res && !res.ok}
      <ul class="errs">{#each res.errors ?? [] as e}<li>{e}</li>{/each}</ul>
      <p class="dim">A save is refused until these are fixed; the files enforced now stay as they are.</p>
    {/if}
    {#if res}
      <p class="dim">Checked together: {res.checked.join(', ')}.</p>
    {/if}
    <p class="dim">{enforced ? 'This file is enforced: a save applies at once, for every project it names.' : 'This file is switched off: a save keeps it, checked as if on, but it applies only once switched on (Security › the org › Policy files).'}</p>
  {/if}
</div>

<style>
  .pc { display: flex; flex-direction: column; gap: 10px; padding: 12px 14px; font-size: 12.5px; }
  .head { display: flex; align-items: center; gap: 8px; flex-wrap: wrap; }
  .head .btn { margin-left: auto; }
  .good { color: var(--ok, #1a7158); font-weight: 600; }
  .bad { color: var(--err, #b42318); font-weight: 600; }
  .dim { color: var(--text-4); margin: 0; line-height: 1.45; }
  .errs { margin: 0; padding: 8px 10px 8px 26px; border-radius: 8px; background: color-mix(in srgb, var(--err, #b42318) 8%, transparent); color: var(--err, #b42318); display: flex; flex-direction: column; gap: 4px; }
  .btn { padding: 4px 10px; border-radius: 6px; border: 1px solid var(--border); background: var(--bg-surface); font: inherit; color: var(--text-1); cursor: pointer; }
  .btn:disabled { opacity: 0.5; cursor: default; }
</style>
