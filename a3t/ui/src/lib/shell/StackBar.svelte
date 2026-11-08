<script lang="ts">
  /// The bar on top of every stack: whatever the stack puts on the left, then
  /// Expand and Pin in the same place on each. The menu has Pin only; its Expand
  /// slot stays empty so the Pin lines up with the other stacks.
  import type { Snippet } from 'svelte';
  import { type Stack, STACK_NAME, isPinned, getFocus, getOverlay, togglePin, toggleExpand } from '$lib/stores/layout.svelte.ts';

  /// `right`: the stack's own actions, just before Expand and Pin.
  /// `pin`: what Pin acts on when it is not the stack itself — the chat's
  /// editing pane in Details pins the pane, not the stack. Expand goes with it.
  /// `onclose`: a pop-in's close, after Pin.
  let { stack, left, right, compact = false, pin, onclose }: {
    stack: Stack; left?: Snippet; right?: Snippet; compact?: boolean;
    pin?: { on: boolean; toggle: () => void; what: string }; onclose?: () => void;
  } = $props();

  const expanded = $derived(stack !== 'menu' && getFocus() === stack);
  // A popped-in stack shows the unpinned icon: pinning it docks it where it is.
  const pinned = $derived(pin ? pin.on : isPinned(stack) && getOverlay() !== stack && !(stack !== 'menu' && getFocus() && getFocus() !== stack));
  const name = $derived(pin?.what ?? STACK_NAME[stack]);
</script>

<div class="sbar" class:compact>
  {#if left}{@render left()}{/if}
  <span class="sp"></span>
  {#if right}{@render right()}{/if}
  {#if stack !== 'menu' && !pin}
    <button class="sb" class:on={expanded} onclick={() => toggleExpand(stack as 'mid' | 'details')}
      title={(expanded ? 'Restore ' : 'Expand ') + name} aria-label={(expanded ? 'Restore ' : 'Expand ') + name} aria-pressed={expanded}>
      <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.9" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
        {#if expanded}<path d="M9 4v5H4M15 4v5h5M9 20v-5H4M15 20v-5h5" />{:else}<path d="M4 9V4h5M20 9V4h-5M4 15v5h5M20 15v5h-5" />{/if}
      </svg>
    </button>
  {:else if !compact && !pin}
    <span class="sb ghost" aria-hidden="true"></span>
  {/if}
  <button class="sb" class:on={pinned} onclick={() => (pin ? pin.toggle() : togglePin(stack))}
    title={(pinned ? 'Unpin ' : 'Pin ') + name} aria-label={(pinned ? 'Unpin ' : 'Pin ') + name} aria-pressed={pinned}>
    <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.9" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
      <path d="M9 3h6l-1 6 3 3H7l3-3z" fill={pinned ? 'currentColor' : 'none'} /><path d="M12 12v9" />
    </svg>
  </button>
  {#if onclose}
    <button class="sb" onclick={onclose} title="Close {name}" aria-label="Close {name}">
      <svg width="14" height="14" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.7" aria-hidden="true"><path d="M12 4L4 12M4 4l8 8" /></svg>
    </button>
  {/if}
</div>

<style>
  .sbar { display: flex; align-items: center; gap: 4px; height: 40px; padding: 0 8px 0 12px; background: var(--bg-surface); border-bottom: 1px solid var(--border); flex: 0 0 auto; min-width: 0; }
  .sbar.compact { padding: 0 6px; background: transparent; }
  .sp { flex: 1 1 auto; }
  .sb { width: 28px; height: 28px; border-radius: 7px; border: 1px solid transparent; background: transparent; display: inline-flex; align-items: center; justify-content: center; color: var(--text-3); flex: 0 0 auto; padding: 0; cursor: pointer; }
  .sbar.compact .sb { width: 24px; height: 24px; }
  .sb:hover { background: var(--bg-subtle); border-color: var(--border); color: var(--text-1); }
  .sb.on { color: var(--spec); }
  .sb.ghost { visibility: hidden; }
  .sb:focus-visible { outline: 2px solid var(--spec); outline-offset: 1px; }
</style>
