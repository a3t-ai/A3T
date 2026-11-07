<script lang="ts">
  /// A drag handle on a stack's edge. Drag it, or focus it and use ← →;
  /// double-click resets the width. `edge` says which side of the stack it sits
  /// on: a handle on the left edge (pop-ins from the right) grows leftwards.
  import { width, setWidth, resetWidth } from '$lib/stores/layout.svelte.ts';

  type Key = 'menu' | 'mid' | 'chatPop' | 'detailsPop';
  let { k, edge = 'right', label, overlay = false }: { k: Key; edge?: 'left' | 'right'; label: string; overlay?: boolean } = $props();

  let drag: { x: number; w: number } | null = null;
  let active = $state(false);
  const sign = $derived(edge === 'left' ? -1 : 1);

  function down(e: PointerEvent) {
    drag = { x: e.clientX, w: width(k) };
    active = true;
    (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
    document.body.classList.add('resizing');
    e.preventDefault();
  }
  function move(e: PointerEvent) {
    if (!drag) return;
    setWidth(k, drag.w + sign * (e.clientX - drag.x));
  }
  function up(e: PointerEvent) {
    drag = null;
    active = false;
    document.body.classList.remove('resizing');
    try { (e.currentTarget as HTMLElement).releasePointerCapture(e.pointerId); } catch {}
  }
  function key(e: KeyboardEvent) {
    if (e.key !== 'ArrowLeft' && e.key !== 'ArrowRight') return;
    e.preventDefault();
    setWidth(k, width(k) + (e.key === 'ArrowRight' ? 16 : -16) * sign);
  }
</script>

<!-- svelte-ignore a11y_no_noninteractive_tabindex -->
<!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
<div class="rz" class:on={active} class:overlay class:left={edge === 'left'}
  role="separator" aria-orientation="vertical" aria-label={`Resize ${label}`} aria-valuenow={width(k)} tabindex="0"
  title="Drag, or ← →. Double-click resets."
  onpointerdown={down} onpointermove={move} onpointerup={up} onpointercancel={up}
  ondblclick={() => resetWidth(k)} onkeydown={key}></div>

<style>
  .rz { flex: 0 0 8px; margin: 0 -4px; cursor: col-resize; position: relative; z-index: 6; touch-action: none; }
  .rz::after { content: ''; position: absolute; top: 0; bottom: 0; left: 3px; width: 2px; background: transparent; transition: background 0.1s; }
  .rz:hover::after, .rz.on::after, .rz:focus-visible::after { background: var(--spec); }
  .rz:focus-visible { outline: none; }
  .rz.overlay { position: absolute; top: 0; bottom: 0; width: 8px; margin: 0; }
  .rz.overlay.left { left: -4px; }
  .rz.overlay:not(.left) { right: -4px; }
</style>
