<script lang="ts">
  import { navigateTo, getActivePanel } from '$lib/stores/panels.svelte.ts';
  import { menuGroups as filteredMenuGroups } from '$lib/stores/auth';
  import { appIcon } from './appIcons';
  import { closeOverlay, getOverlay } from '$lib/stores/layout.svelte.ts';

  /// Wide enough for labels (the menu's width is the user's, by dragging).
  /// `foot`: the shell's control for the menu itself (hide / keep it), above the version.
  let { wide = false, foot }: { wide?: boolean; foot?: import('svelte').Snippet } = $props();

  let menuGroups = $derived($filteredMenuGroups);
  let activeType = $derived(getActivePanel()?.type ?? 'dashboard');

  // Tooltip state
  let tooltipText = $state('');
  let tooltipY = $state(0);
  let tooltipX = $state(0);
  let showTooltip = $state(false);
  let tooltipTimeout: ReturnType<typeof setTimeout> | null = null;

  function handleClick(type: string, label: string) {
    activeType = type;
    navigateTo(type, label);
    // A popped-in menu steps aside once an app is picked.
    if (getOverlay() === 'menu') closeOverlay();
  }

  function onEnter(e: MouseEvent, label: string) {
    if (wide) return;  // the label is already on the button
    const rect = (e.currentTarget as HTMLElement).getBoundingClientRect();
    tooltipText = label;
    tooltipX = rect.right + 10;
    tooltipY = rect.top + rect.height / 2;
    tooltipTimeout = setTimeout(() => { showTooltip = true; }, 200);
  }

  function onLeave() {
    if (tooltipTimeout) clearTimeout(tooltipTimeout);
    showTooltip = false;
  }


  function getIcon(name: string): string {
    return appIcon(name);
  }
</script>

<nav class="sidebar" class:wide aria-label="Menu">
  {#snippet group(items: typeof menuGroups[number]['items'])}
    {#each items as item}
      <button
        class="nav-btn"
        class:active={activeType === item.type}
        onclick={() => handleClick(item.type, item.label)}
        aria-label={item.label}
        onmouseenter={(e) => onEnter(e, item.label)}
        onmouseleave={onLeave}
      >
        <svg class="nav-ico" width="20" height="20" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5">
          {@html getIcon(item.icon)}
        </svg>
        {#if wide}<span class="nav-label">{item.label}</span>{/if}
      </button>
    {/each}
  {/snippet}

  <div class="sidebar-top">
    {#each menuGroups.filter((g) => !g.pinned) as g}
      <div class="nav-group">
        <div class="group-divider"></div>
        {@render group(g.items)}
      </div>
    {/each}
  </div>

  <!-- Pinned groups (Studio, Settings) stay in view while the apps above scroll. -->
  {#each menuGroups.filter((g) => g.pinned) as g}
    <div class="nav-group pinned">{@render group(g.items)}</div>
  {/each}

  <div class="sidebar-bottom">
    {#if foot}{@render foot()}{/if}
    <div class="version-pill">v0.5</div>
  </div>
</nav>

<!-- Tooltip rendered at document level via fixed positioning — never clipped -->
{#if showTooltip}
  <div class="sidebar-tooltip" style="top: {tooltipY}px; left: {tooltipX}px;">
    {tooltipText}
  </div>
{/if}

<style>
  .sidebar {
    height: 100%;
    background: var(--bg-surface);
    border-right: 1px solid var(--border);
    display: flex;
    flex-direction: column;
    /* The apps scroll; the foot (menu control, version) stays in view. */
    overflow: hidden;
    width: 100%;
  }

  .sidebar-top {
    flex: 1 1 auto;
    min-height: 0;
    overflow-y: auto;
    scrollbar-width: none;
    display: flex;
    flex-direction: column;
    padding: 8px 0;
    gap: 2px;
  }

  .nav-group {
    display: flex;
    flex-direction: column;
    gap: 1px;
  }

  .group-divider {
    height: 1px;
    background: var(--divider);
    margin: 6px 14px;
  }
  .nav-group:first-child .group-divider { display: none; }
  .nav-group.pinned { flex: 0 0 auto; padding: 4px 0; border-top: 1px solid var(--divider); }

  .nav-btn {
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 12px 0;
    margin: 0 8px;
    border-radius: var(--r-sm);
    border: none;
    background: none;
    cursor: pointer;
    color: var(--text-3);
    position: relative;
    transition: all 0.12s;
  }
  .nav-btn:hover {
    background: var(--bg-subtle);
    color: var(--text-1);
  }
  .nav-btn.active {
    background: var(--bg-subtle);
    color: var(--text-1);
  }
  .nav-btn.active::before {
    content: '';
    position: absolute;
    left: -8px;
    top: 6px;
    bottom: 6px;
    width: 3px;
    background: var(--ok);
    border-radius: 0 2px 2px 0;
  }

  .nav-ico { flex-shrink: 0; }
  .nav-btn.active .nav-ico { color: var(--ephemeral); }

  /* One row — the menu control beside the version — so the foot is no taller
     than the version alone was and the apps above keep their room. */
  .sidebar-bottom {
    flex: 0 0 auto;
    padding: 4px 0 6px;
    display: flex;
    align-items: center;
    gap: 4px;
    justify-content: center;
    border-top: 1px solid var(--divider);
  }
  .version-pill {
    font-family: var(--font-mono);
    font-size: 8px;
    color: var(--text-5);
    padding: 2px 6px;
    border-radius: 3px;
    background: var(--bg-subtle);
  }

  /* Fixed-position tooltip — renders above everything */
  .sidebar-tooltip {
    position: fixed;
    transform: translateY(-50%);
    padding: 5px 12px;
    background: var(--text-1, #0f0f0f);
    color: var(--bg-page, #faf9f6);
    font-size: 12px;
    font-weight: 500;
    white-space: nowrap;
    border-radius: 5px;
    pointer-events: none;
    z-index: 9999;
    box-shadow: 0 2px 8px rgba(0,0,0,0.15);
  }
  .sidebar-tooltip::before {
    content: '';
    position: absolute;
    right: 100%;
    top: 50%;
    transform: translateY(-50%);
    border: 5px solid transparent;
    border-right-color: var(--text-1, #0f0f0f);
  }
  .sidebar.wide .nav-btn { justify-content: flex-start; gap: 10px; padding: 9px 12px; }
  .nav-label { font-size: 13px; font-weight: 500; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
</style>
