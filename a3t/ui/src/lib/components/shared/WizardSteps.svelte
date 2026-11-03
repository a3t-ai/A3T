<script lang="ts" module>
  /// One step of a pop-in: its id, label, and — for a new item — whether it is
  /// filled in well enough to go on (`ok`) and what is missing when not (`need`).
  export type WizardStep = { id: string; label: string; icon?: string; ok?: boolean; need?: string; optional?: boolean };

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
  /// The steps of a pop-in, numbered: done ones ticked, the current one marked,
  /// later ones locked until the ones before them are filled in (new items only).
  /// Vertical in a pop-in's left rail, horizontal over a single-column one.
  let { steps, active, onselect, wizard = false, orientation = 'vertical' }: {
    steps: WizardStep[]; active: string; onselect: (id: string) => void;
    /// A new item: go in order. An existing one: any step, any time.
    wizard?: boolean;
    orientation?: 'vertical' | 'horizontal';
  } = $props();

  const idx = $derived(steps.findIndex((s) => s.id === active));
</script>

<nav class="ws {orientation}" aria-label="Steps">
  {#each steps as s, i (s.id)}
    {@const open = reachable(steps, s.id, wizard)}
    {@const done = i < idx && s.ok !== false}
    <button class="st" class:on={s.id === active} class:done class:locked={!open} disabled={!open}
            onclick={() => onselect(s.id)} aria-current={s.id === active ? 'step' : undefined}
            title={open ? s.label : `Finish the steps before ${s.label}`}>
      <span class="n">
        {#if done}<svg viewBox="0 0 16 16" aria-hidden="true"><path d="M3.5 8.5l3 3 6-7" /></svg>{:else}{i + 1}{/if}
      </span>
      <span class="l">{s.label}{#if s.optional}<em> · optional</em>{/if}</span>
    </button>
    {#if orientation === 'horizontal' && i < steps.length - 1}<span class="bar" class:done={i < idx}></span>{/if}
  {/each}
</nav>

<style>
  .ws { display: flex; }
  .ws.vertical { flex-direction: column; padding: 8px 0; gap: 2px; position: relative; }
  .ws.horizontal { align-items: center; gap: 6px; flex-wrap: wrap; }

  .st { display: flex; align-items: center; gap: 9px; border: 0; background: none; cursor: pointer; text-align: left; color: var(--text-3); font-size: 12.5px; }
  .vertical .st { padding: 7px 14px; }
  .horizontal .st { padding: 6px 4px; }
  .st:hover:not(:disabled) { color: var(--text-1); }
  .vertical .st:hover:not(:disabled) { background: var(--bg-subtle); }
  .st.on { color: var(--text-1); font-weight: 600; }
  .vertical .st.on { background: var(--bg-subtle); box-shadow: inset 3px 0 0 var(--spec, #5530a8); }
  .st.locked { opacity: 0.45; cursor: not-allowed; }

  .n { width: 20px; height: 20px; border-radius: 50%; flex-shrink: 0; display: grid; place-items: center; font: 600 10.5px var(--font-mono);
       border: 1.5px solid var(--border); color: var(--text-3); background: var(--bg-surface); }
  .st.on .n { border-color: var(--spec, #5530a8); background: var(--spec, #5530a8); color: #fff; }
  .st.done .n { border-color: var(--ok); background: var(--ok-bg); color: var(--ok); }
  .n svg { width: 11px; height: 11px; fill: none; stroke: currentColor; stroke-width: 2.2; stroke-linecap: round; stroke-linejoin: round; }
  .l { white-space: nowrap; }
  .l em { font-style: normal; font-weight: 400; color: var(--text-4); font-size: 11px; }

  .bar { flex: 0 0 22px; height: 1.5px; background: var(--border); }
  .bar.done { background: var(--ok); }
</style>
