<script lang="ts">
  import WizardSteps, { type WizardStep } from './WizardSteps.svelte';
  import { detailsPane } from '$lib/shell/detailsPane';
  /// In the shell's Details pane: the item's header, then its sections as
  /// horizontal tabs (icon only when the pane is narrow). Elsewhere, a tree.
  const tabs = !!detailsPane();
  let {
    icon = '?',
    color = '#999',
    name = '',
    meta = '',
    nodes = [] as { id: string; label: string; icon: string }[],
    activeNode = '',
    onselect,
    steps,
    wizard = false,
  }: {
    icon: string;
    color: string;
    name: string;
    meta: string;
    nodes: { id: string; label: string; icon: string }[];
    activeNode: string;
    onselect: (id: string) => void;
    /// Numbered steps instead of plain nodes (see WizardSteps); `wizard`: in order.
    steps?: WizardStep[];
    wizard?: boolean;
  } = $props();
</script>

<div class="pt-panel" class:tabs>
  <div class="pt-header">
    <div class="pt-icon" style="background: {color}">{icon}</div>
    <div class="pt-info">
      <div class="pt-name">{name}</div>
      <div class="pt-meta">{meta}</div>
    </div>
  </div>
  {#if steps}
    <WizardSteps {steps} active={activeNode} {wizard} {onselect} />
  {:else}
  <nav class="pt-tree" role={tabs ? 'tablist' : undefined}>
    {#each nodes as node}
      <button class="pt-node" class:active={activeNode === node.id} onclick={() => onselect(node.id)}
        role={tabs ? 'tab' : undefined} aria-selected={tabs ? activeNode === node.id : undefined} title={node.label}>
        <svg width="14" height="14" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5">{@html node.icon}</svg>
        <span>{node.label}</span>
      </button>
    {/each}
  </nav>
  {/if}
</div>

<style>
  .pt-panel {
    display: flex; flex-direction: column; background: var(--bg-surface);
    border-right: 1px solid var(--border); overflow-y: auto; width: 100%; height: 100%;
  }

  .pt-header { display: flex; align-items: center; gap: 12px; min-height: 72px; padding: 12px 20px; box-sizing: border-box; border-bottom: 1px solid var(--border); }
  .pt-icon { width: 32px; height: 32px; border-radius: 8px; display: grid; place-items: center; font-family: var(--font-mono); font-size: 11px; font-weight: 700; color: white; flex-shrink: 0; }
  .pt-info { min-width: 0; }
  .pt-name { font-family: var(--font-serif); font-size: 15px; font-weight: 500; line-height: 1.25; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .pt-meta { font-size: 11px; color: var(--text-4); margin-top: 2px; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }

  .pt-tree { display: flex; flex-direction: column; padding: 6px 0; flex: 1; }
  .pt-node {
    display: flex; align-items: center; gap: 8px;
    padding: 7px 14px; border: none; background: none;
    cursor: pointer; font-size: 12px; color: var(--text-3);
    text-align: left; transition: all 0.1s;
  }
  .pt-node:hover { color: var(--text-1); background: var(--bg-subtle); }
  .pt-node.active {
    color: var(--text-1); font-weight: 550; background: var(--bg-subtle);
    box-shadow: inset 3px 0 0 var(--spec, #c47028);
  }
  .pt-node svg { color: var(--text-4); flex-shrink: 0; }
  .pt-node.active svg { color: var(--spec, #c47028); }

  /* Tabs: header row, then one horizontal row of sections. */
  .pt-panel.tabs { height: auto; flex: 0 0 auto; border-right: 0; overflow: visible; container-type: inline-size; }
  .pt-panel.tabs .pt-header { min-height: 0; padding: 12px 18px; border-bottom: 1px solid var(--border-light, #f0ece3); }
  .pt-panel.tabs .pt-name { font-size: 18px; font-weight: 400; }
  .pt-panel.tabs .pt-tree { flex-direction: row; gap: 2px; padding: 0 12px; overflow-x: auto; scrollbar-width: none; border-bottom: 1px solid var(--border); }
  .pt-panel.tabs .pt-node { padding: 10px 10px 9px; font-size: 13px; white-space: nowrap; border-bottom: 2px solid transparent; background: none; }
  .pt-panel.tabs .pt-node:hover { background: none; }
  .pt-panel.tabs .pt-node.active { box-shadow: none; background: none; font-weight: 600; border-bottom-color: var(--spec, #5530a8); }
  @container (max-width: 560px) {
    .pt-panel.tabs .pt-node span { display: none; }
    .pt-panel.tabs .pt-node.active span { display: inline; }
  }
</style>
