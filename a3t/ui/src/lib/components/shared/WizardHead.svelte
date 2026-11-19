<script lang="ts">
  /// The title of the step being shown, with the line on what it is for — the
  /// same heading at the top of every stepped pop-in.
  import type { WizardStep } from './WizardSteps.svelte';
  let { step, prefix = '' }: { step: WizardStep | undefined; prefix?: string } = $props();
</script>

{#if step}
  <header class="wh">
    <h3>{prefix}{step.label}{#if step.optional}<span class="opt">optional</span>{/if}</h3>
    {#if step.hint}<p>{step.hint}</p>{/if}
  </header>
{/if}

<style>
  .wh { margin: 0 0 22px; }
  h3 { margin: 0; font-family: var(--font-serif); font-size: 19px; font-weight: 500; color: var(--text-1); display: flex; align-items: baseline; gap: 10px; }
  .opt { font: 500 10.5px var(--font-sans); text-transform: uppercase; letter-spacing: 0.05em; color: var(--text-4); }
  p { margin: 5px 0 0; font-size: 12.5px; color: var(--text-3); line-height: 1.45; }
</style>
