<script lang="ts">
  /// The foot of a stepped pop-in: Back, Next, and on the last step the final
  /// action (Create for a new item, Save for an existing one). For a new item
  /// Next waits until the step is filled in, and says what is missing.
  import { reachable, type WizardStep } from './WizardSteps.svelte';

  let { steps, active, onselect, wizard = false, final, finalLabel = 'Save', finalBusy = false, finalDisabled = false, busyLabel = 'Saving…' }: {
    steps: WizardStep[]; active: string; onselect: (id: string) => void;
    wizard?: boolean;
    /// The final action; none (e.g. read-only) shows Back / Next only.
    final?: () => void;
    finalLabel?: string; finalBusy?: boolean; finalDisabled?: boolean; busyLabel?: string;
  } = $props();

  const i = $derived(steps.findIndex((s) => s.id === active));
  const cur = $derived(steps[i]);
  const prev = $derived(i > 0 ? steps[i - 1] : null);
  const next = $derived(i >= 0 && i < steps.length - 1 ? steps[i + 1] : null);
  const blocked = $derived(wizard && cur?.ok === false);
  /// A new item is made from the last step; an existing one saves from any.
  const showFinal = $derived(!!final && (!wizard || !next));
  const allOk = $derived(steps.every((s) => s.ok !== false));
</script>

<div class="wn">
  <span class="pos">Step {i + 1} of {steps.length}</span>
  {#if blocked && cur?.need}<span class="need">{cur.need}</span>{/if}
  {#if wizard && !next && !allOk}<span class="need">{steps.find((s) => s.ok === false)?.need ?? 'Some steps are not complete.'}</span>{/if}
  <span class="grow"></span>
  {#if prev}<button class="b" onclick={() => onselect(prev.id)}>← Back</button>{/if}
  {#if next}
    <button class="b" class:primary={wizard} disabled={blocked || !reachable(steps, next.id, wizard) && wizard} onclick={() => onselect(next.id)}>Next: {next.label} →</button>
  {/if}
  {#if showFinal}
    <button class="b primary" disabled={finalBusy || finalDisabled || (wizard && !allOk)} onclick={final}>{finalBusy ? busyLabel : finalLabel}</button>
  {/if}
</div>

<style>
  .wn { display: flex; align-items: center; gap: 10px; padding: 10px 16px; border-top: 1px solid var(--border); background: var(--bg-surface); }
  .pos { font: 11px var(--font-mono); color: var(--text-4); white-space: nowrap; }
  .need { font-size: 12px; color: var(--warn); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; min-width: 0; }
  .grow { flex: 1; }
  .b { padding: 7px 14px; border: 1px solid var(--border); border-radius: var(--r-sm, 6px); background: var(--bg-surface); color: var(--text-2); font-size: 12.5px; font-weight: 550; cursor: pointer; white-space: nowrap; }
  .b:hover:not(:disabled) { background: var(--bg-subtle); color: var(--text-1); }
  .b.primary { background: var(--spec, #5530a8); border-color: var(--spec, #5530a8); color: #fff; }
  .b.primary:hover:not(:disabled) { opacity: 0.9; background: var(--spec, #5530a8); color: #fff; }
  .b:disabled { opacity: 0.45; cursor: not-allowed; }
</style>
