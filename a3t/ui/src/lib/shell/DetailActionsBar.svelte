<script lang="ts">
  /// The open item's actions, on the right of its header row: the Details
  /// counterpart of the "New …" button in the Items header. Every item gets
  /// Open in Studio and Save, after what is particular to it.
  /// Renders nothing outside the shell's Details pane.
  import { untrack } from 'svelte';
  import { detailsPane } from './detailsPane';
  import { getDetailActions, hostActions } from '$lib/stores/detailActions.svelte.ts';

  const inPane = !!detailsPane();
  // Untracked: hostActions reads and writes the host count.
  $effect(() => (inPane ? untrack(hostActions) : undefined));
  const a = $derived(inPane ? getDetailActions() : null);
</script>

{#if a}
  <div class="dab" role="toolbar" aria-label="Item actions">
    {#each a.extra ?? [] as x (x.label)}
      <button class="btn" onclick={x.run}>{x.label}</button>
    {/each}
    {#if a.onopenineditor}
      <button class="btn" onclick={a.onopenineditor} title="Open in Studio">
        <svg width="13" height="13" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5"><path d="M9 1h6v6M9 7L15 1M7 3H2a1 1 0 00-1 1v10a1 1 0 001 1h10a1 1 0 001-1V9" /></svg>
        <span>Open in Studio</span>
      </button>
    {/if}
    {#if a.onsave}
      <button class="btn g" onclick={a.onsave}>
        <svg width="13" height="13" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5"><path d="M13 15H3a1 1 0 01-1-1V2a1 1 0 011-1h8l3 3v10a1 1 0 01-1 1z" /><path d="M11 15V9H5v6M5 1v4h5" /></svg>
        <span>Save</span>
      </button>
    {/if}
  </div>
{/if}

<style>
  .dab { margin-left: auto; flex: 0 0 auto; display: flex; align-items: center; gap: 6px; padding-left: 12px; }
  .btn { display: inline-flex; align-items: center; gap: 6px; height: 32px; padding: 0 12px; border-radius: 8px; border: 1px solid var(--border); background: var(--bg-surface); font: 500 13px var(--font-sans); color: var(--text-1); white-space: nowrap; cursor: pointer; }
  .btn:hover { background: var(--bg-subtle); }
  .btn.g { background: var(--green, #1a7158); border-color: var(--green, #1a7158); color: #fff; }
  .btn.g:hover { filter: brightness(1.06); }
  .btn svg { flex: none; }
  /* A narrow pane keeps the icons and drops the words, except on Save. */
  @container (max-width: 560px) { .btn:not(.g) span { display: none; } .btn:not(.g) { padding: 0 9px; } }
</style>
