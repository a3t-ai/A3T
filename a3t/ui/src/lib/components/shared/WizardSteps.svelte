<script lang="ts" module>
  /// One step of a pop-in: its id, label, a line on what it is for (`hint`), and —
  /// for a new item — whether it is filled in well enough to go on (`ok`) and
  /// what is missing when not (`need`).
  export type WizardStep = { id: string; label: string; hint?: string; icon?: string; ok?: boolean; need?: string; optional?: boolean };

  /// A step can be opened when every step before it is ok (a new item), or always
  /// (an existing one).
  export function reachable(steps: WizardStep[], id: string, wizard: boolean): boolean {
    if (!wizard) return true;
    for (const s of steps) {
      if (s.id === id) return true;
      if (s.ok === false) return false;
    }
    return false;
  }
</script>

<script lang="ts">
  /// The steps of a pop-in as a wizard: a vertical stack down the full height of
  /// the rail, each step a numbered circle with its title and what it is for,
  /// joined by a line that turns from grey to colour as the steps are done.
  /// Later steps of a new item stay locked until the ones before are filled in.
  /// `horizontal` is a compact strip for narrow places.
  import { detailsPane } from '$lib/shell/detailsPane';
  import SwipeStrip from './SwipeStrip.svelte';
  let { steps, active, onselect, wizard = false, orientation: given }: {
    steps: WizardStep[]; active: string; onselect: (id: string) => void;
    /// A new item: go in order. An existing one: any step, any time.
    wizard?: boolean;
    orientation?: 'vertical' | 'horizontal';
  } = $props();

  /// In the shell's Details pane sections run across the top; elsewhere down the side.
  const inPane = !!detailsPane();  // context: read once, at init
  const orientation = $derived(given ?? (inPane ? 'horizontal' : 'vertical'));
  const idx = $derived(steps.findIndex((s) => s.id === active));
</script>

{#if !wizard}
  <!-- An existing item: plain tabs, any one at a time. Across the top they swipe. -->
  {#snippet tabs(row: boolean)}
    <nav class="tabs" class:row aria-label="Sections">
      {#each steps as s (s.id)}
        <button class="tab" class:on={s.id === active} onclick={() => onselect(s.id)} aria-current={s.id === active ? 'page' : undefined} title={s.hint ?? s.label}>
          {#if s.icon}<svg width="15" height="15" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" aria-hidden="true">{@html s.icon}</svg>{/if}
          <span>{s.label}</span>
        </button>
      {/each}
    </nav>
  {/snippet}
  {#if orientation === 'horizontal'}<SwipeStrip class="tabstrip">{@render tabs(true)}</SwipeStrip>{:else}{@render tabs(false)}{/if}
{:else}
<nav class="ws {orientation}" aria-label="Steps">
  {#each steps as s, i (s.id)}
    {@const open = reachable(steps, s.id, wizard)}
    {@const done = i < idx && s.ok !== false}
    <div class="item" class:on={s.id === active} class:done class:locked={!open} class:last={i === steps.length - 1}>
      <button class="st" disabled={!open} onclick={() => onselect(s.id)} aria-current={s.id === active ? 'step' : undefined}
              title={open ? s.label : `Finish the steps before ${s.label}`}>
        <span class="n">
          {#if done}<svg viewBox="0 0 16 16" aria-hidden="true"><path d="M3.5 8.5l3 3 6-7" /></svg>{:else}{i + 1}{/if}
        </span>
        <span class="txt">
          <span class="l">{s.label}{#if s.optional}<em> · optional</em>{/if}</span>
          {#if s.hint && orientation === 'vertical'}<span class="h">{s.hint}</span>{/if}
        </span>
      </button>
      {#if i < steps.length - 1}<span class="line" class:filled={i < idx} aria-hidden="true"></span>{/if}
    </div>
  {/each}
</nav>
{/if}

<style>
  /* ── Tabs (an existing item) ── */
  .tabs { display: flex; flex-direction: column; padding: 10px 0; gap: 1px; }
  .tab { display: flex; align-items: center; gap: 10px; height: 38px; padding: 0 20px; border: 0; background: none; cursor: pointer; text-align: left;
         font-size: 13px; color: var(--text-3); transition: background 0.1s, color 0.1s; }
  .tab svg { color: var(--text-4); flex-shrink: 0; }
  .tab:hover { color: var(--text-1); background: var(--bg-subtle); }
  .tab.on { color: var(--text-1); font-weight: 600; background: var(--bg-subtle); box-shadow: inset 3px 0 0 var(--spec, #5530a8); }
  .tab.on svg { color: var(--spec, #5530a8); }
  /* Across the top, as in the shell's Details pane. */
  .tabs.row { flex-direction: row; padding: 0 12px; gap: 2px; }
  :global(.swipe.tabstrip) { border-bottom: 1px solid var(--border); }
  .tabs.row .tab { height: auto; padding: 10px 10px 9px; white-space: nowrap; border-bottom: 2px solid transparent; gap: 7px; }
  .tabs.row .tab:hover { background: none; }
  .tabs.row .tab.on { background: none; box-shadow: none; border-bottom-color: var(--spec, #5530a8); }

  .ws { display: flex; --c-done: var(--ok, #2fa88d); --c-on: var(--spec, #5530a8); --c-off: var(--border, #e4e0d8); }

  /* ── Vertical: the full height of the rail ── */
  .ws.vertical { flex-direction: column; padding: 24px 20px 20px; flex: 1; min-height: 0; }
  .vertical .item { position: relative; display: flex; flex-direction: column; flex: 1 1 0; max-height: 128px; min-height: 64px; }
  .vertical .item.last { flex: 0 0 auto; min-height: 0; }
  .vertical .st { display: flex; align-items: flex-start; gap: 12px; width: 100%; padding: 0; border: 0; background: none; text-align: left; cursor: pointer; position: relative; z-index: 1; }
  .vertical .n { width: 32px; height: 32px; font-size: 12px; }
  .vertical .txt { display: flex; flex-direction: column; gap: 3px; padding-top: 6px; min-width: 0; }
  .vertical .l { font-size: 13px; }
  .h { font-size: 11.5px; line-height: 1.4; color: var(--text-4); }
  /* The connector: from under this circle to the next one. */
  .vertical .line { position: absolute; left: 15px; top: 38px; bottom: 6px; width: 2px; border-radius: 1px; background: var(--c-off); overflow: hidden; }
  .vertical .line::after { content: ''; position: absolute; inset: 0; background: var(--c-done); transform: scaleY(0); transform-origin: top; transition: transform 0.35s ease; }
  .vertical .line.filled::after { transform: scaleY(1); }

  /* ── Horizontal: a compact strip ── */
  .ws.horizontal { align-items: center; gap: 8px; flex-wrap: wrap; }
  .horizontal .item { display: flex; align-items: center; gap: 8px; }
  .horizontal .st { display: flex; align-items: center; gap: 8px; padding: 6px 2px; border: 0; background: none; cursor: pointer; }
  .horizontal .n { width: 22px; height: 22px; font-size: 10.5px; }
  .horizontal .line { width: 26px; height: 2px; background: var(--c-off); }
  .horizontal .line.filled { background: var(--c-done); }

  /* ── Both ── */
  .n { border-radius: 50%; flex-shrink: 0; display: grid; place-items: center; font-family: var(--font-mono); font-weight: 600;
       border: 2px solid var(--c-off); color: var(--text-4); background: var(--bg-surface); transition: all 0.2s; }
  .item.on .n { border-color: var(--c-on); background: var(--c-on); color: #fff; box-shadow: 0 0 0 4px color-mix(in srgb, var(--c-on) 15%, transparent); }
  .item.done .n { border-color: var(--c-done); background: var(--c-done); color: #fff; }
  .n svg { width: 13px; height: 13px; fill: none; stroke: currentColor; stroke-width: 2.4; stroke-linecap: round; stroke-linejoin: round; }
  .l { color: var(--text-3); font-weight: 500; white-space: nowrap; }
  .l em { font-style: normal; font-weight: 400; color: var(--text-4); font-size: 11px; }
  .item.on .l { color: var(--text-1); font-weight: 650; }
  .item.done .l { color: var(--text-2); }
  .st:hover:not(:disabled) .l { color: var(--text-1); }
  .item.locked .st { cursor: not-allowed; }
  .item.locked .l, .item.locked .h { opacity: 0.55; }
</style>
