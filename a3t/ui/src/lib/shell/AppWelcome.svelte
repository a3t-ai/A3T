<script lang="ts">
  /// What Details shows for a list app with nothing picked: the app's banner
  /// (shell/appTheme.ts) and where to start.
  import PageBanner from '$lib/components/shared/PageBanner.svelte';
  import { bannerFor } from './appTheme';
  import { contextLabel } from '$lib/stores/apps.svelte.ts';

  let { appType, title, onitems }: { appType: string; title: string; onitems?: () => void } = $props();
  const banner = $derived(bannerFor(appType, title));
  const what = $derived(contextLabel(appType).toLowerCase());

</script>

<div class="aw">
  <PageBanner {...banner} />

  <div class="aw-start">
    <span>Pick from {what} in Items to open it here, or ask in Chat.</span>
    <span class="sp"></span>
    {#if onitems}<button class="btn" onclick={onitems}>Show Items</button>{/if}
  </div>

</div>

<style>
  .aw { height: 100%; overflow-y: auto; padding: 20px 24px 32px; box-sizing: border-box; display: flex; flex-direction: column; gap: 18px; }
  .aw-start { display: flex; align-items: center; gap: 8px; flex-wrap: wrap; padding: 10px 14px; border: 1px solid var(--border); border-radius: 12px; background: var(--bg-surface); font-size: 13px; color: var(--text-2); }
  .sp { flex: 1 1 auto; }
  .btn { height: 30px; padding: 0 12px; border-radius: 8px; border: 1px solid var(--border); background: var(--bg-surface); font: 500 13px var(--font-sans); color: var(--spec); cursor: pointer; white-space: nowrap; }
  .btn:hover { border-color: var(--spec); }
</style>
