<script lang="ts">
  /// A row of tabs that never gets cut off: when it is wider than its place it
  /// scrolls sideways — swipe on touch, the wheel or trackpad with a mouse — and
  /// snaps to a tab. The side with more shows a fade and a ‹ › button; the
  /// selected tab (.on, aria-current, aria-selected) is always scrolled into view.
  import type { Snippet } from 'svelte';
  import { onMount } from 'svelte';

  let { children, class: cls = '' }: { children: Snippet; class?: string } = $props();

  let track: HTMLDivElement;
  let left = $state(false);
  let right = $state(false);

  function edges() {
    if (!track) return;
    left = track.scrollLeft > 2;
    right = track.scrollLeft + track.clientWidth < track.scrollWidth - 2;
  }
  function page(dir: -1 | 1) {
    track.scrollBy({ left: dir * Math.max(120, track.clientWidth * 0.7), behavior: 'smooth' });
  }
  /// A vertical wheel scrolls the row when it overflows (a mouse has no sideways wheel).
  function wheel(e: WheelEvent) {
    if (track.scrollWidth <= track.clientWidth || Math.abs(e.deltaX) > Math.abs(e.deltaY)) return;
    e.preventDefault();
    track.scrollLeft += e.deltaY;
  }
  function showActive(smooth = true) {
    const on = track?.querySelector<HTMLElement>('.on, [aria-current="page"], [aria-current="step"], [aria-selected="true"]');
    if (!on) return;
    const t = track.getBoundingClientRect(), r = on.getBoundingClientRect();
    // Clear of the ‹ › buttons (26px), which sit over the ends.
    const pad = 34;
    if (r.left < t.left + pad) track.scrollBy({ left: r.left - t.left - pad, behavior: smooth ? 'smooth' : 'auto' });
    else if (r.right > t.right - pad) track.scrollBy({ left: r.right - t.right + pad, behavior: smooth ? 'smooth' : 'auto' });
  }

  onMount(() => {
    edges();
    showActive(false);
    const ro = new ResizeObserver(() => { edges(); showActive(false); });
    ro.observe(track);
    // A new tab picked (or the tabs changed): keep it in view.
    // Other tabs altogether (another section's): start from the beginning.
    const mo = new MutationObserver((rs) => {
      if (rs.some((r) => r.type === 'childList')) track.scrollLeft = 0;
      edges(); showActive(!rs.some((r) => r.type === 'childList'));
    });
    mo.observe(track, { subtree: true, childList: true, attributes: true, attributeFilter: ['class', 'aria-current', 'aria-selected'] });
    return () => { ro.disconnect(); mo.disconnect(); };
  });
</script>

<div class="swipe {cls}" class:l={left} class:r={right}>
  <div class="track" bind:this={track} onscroll={edges} onwheel={wheel}>
    {@render children()}
  </div>
  {#if left}<button class="arr al" type="button" tabindex="-1" aria-label="Earlier tabs" onclick={() => page(-1)}>‹</button>{/if}
  {#if right}<button class="arr ar" type="button" tabindex="-1" aria-label="More tabs" onclick={() => page(1)}>›</button>{/if}
</div>

<style>
  .swipe { position: relative; min-width: 0; max-width: 100%; }
  .track {
    display: flex; overflow-x: auto; overflow-y: hidden; scrollbar-width: none;
    scroll-snap-type: x proximity; -webkit-overflow-scrolling: touch; overscroll-behavior-x: contain;
  }
  .track::-webkit-scrollbar { display: none; }
  .track > :global(*) { flex: 0 0 auto; }
  .track :global(button) { scroll-snap-align: start; }
  .track { scroll-padding-inline: 34px; }
  /* A fade where there is more. */
  .swipe.r .track { -webkit-mask-image: linear-gradient(to right, #000 calc(100% - 44px), transparent); mask-image: linear-gradient(to right, #000 calc(100% - 44px), transparent); }
  .swipe.l .track { -webkit-mask-image: linear-gradient(to right, transparent, #000 44px); mask-image: linear-gradient(to right, transparent, #000 44px); }
  .swipe.l.r .track { -webkit-mask-image: linear-gradient(to right, transparent, #000 44px, #000 calc(100% - 44px), transparent); mask-image: linear-gradient(to right, transparent, #000 44px, #000 calc(100% - 44px), transparent); }
  .arr {
    position: absolute; top: 50%; transform: translateY(-50%); width: 26px; height: 26px; border-radius: 50%;
    border: 1px solid var(--border); background: var(--bg-surface); color: var(--text-2); font: 600 15px/1 var(--font-sans);
    display: grid; place-items: center; cursor: pointer; box-shadow: 0 1px 3px rgba(0,0,0,0.12); z-index: 2; padding: 0 0 2px;
  }
  .arr:hover { color: var(--text-1); border-color: var(--spec); }
  .al { left: 2px; } .ar { right: 2px; }
  /* Touch screens swipe; the buttons are for a mouse. */
  @media (hover: none) { .arr { display: none; } }
</style>
