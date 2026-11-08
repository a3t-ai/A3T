<script lang="ts">
  /// Scopes the Details-pane context to what the shell shows in Details, so a
  /// slide-over opened there fills the pane. Chat and Items are outside it.
  import type { Snippet } from 'svelte';
  import { provideDetailsPane, type PaneHost } from './detailsPane';

  let { host, children }: { host: PaneHost; children: Snippet } = $props();
  provideDetailsPane(host);
</script>

{@render children()}
