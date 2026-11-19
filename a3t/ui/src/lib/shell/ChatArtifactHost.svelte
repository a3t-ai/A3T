<script lang="ts">
  /// The chat's editing pane, in Details. A card in a conversation opens an item
  /// here for the user and the agent to edit together. It pops in over Details;
  /// pinned, it takes Details (and the conversation moves beside it — the shell
  /// does that part). It stays mounted while the item is open, and only its
  /// placement changes: the pane holds the unsaved edits and the agent's pending
  /// revision, which must survive pinning, unpinning and the conversation moving.
  import ArtifactPane from '$lib/components/chat/ArtifactPane.svelte';
  import StackBar from './StackBar.svelte';
  import Resizer from './Resizer.svelte';
  import { width } from '$lib/stores/layout.svelte.ts';
  import {
    getChatArtifact, closeChatArtifact, setArtifactPinned, setArtifactRevision,
    setArtifactSection, setArtifactEdit, sendFromPane,
  } from '$lib/stores/chatArtifact.svelte.ts';

  /// `pop`: over Details, from the right. `fill`: all of Details, under its bar.
  /// `hidden`: its conversation is not on screen — kept, not shown.
  let { mode, onopenapp }: {
    mode: 'pop' | 'fill' | 'hidden';
    onopenapp: (link: { type: string; title: string; id: string }) => void;
  } = $props();

  const st = $derived(getChatArtifact());

  /// Closing drops the pane's unsaved edits, so ask first when there are any.
  export function close(): boolean {
    if (st?.edit?.dirty && !confirm(`Close ${st.artifact.name}? Your unsaved changes will be lost.`)) return false;
    closeChatArtifact();
    return true;
  }
</script>

{#if st}
  <section class="cah {mode}" aria-label="Editing {st.artifact.name}"
    style:width={mode === 'pop' ? `${width('detailsPop')}px` : undefined}>
    {#if mode === 'pop'}
      <Resizer k="detailsPop" edge="left" label="Details" overlay />
      <StackBar stack="details" pin={{ on: false, toggle: () => setArtifactPinned(true), what: 'this beside the conversation' }} onclose={close}>
        {#snippet left()}<span class="sbname">Details</span>{/snippet}
      </StackBar>
    {/if}
    <div class="cah-body">
      <!-- One pane per item: another item starts clean (its own text, section,
           Undo), never with the previous one's. -->
      {#key `${st.artifact.kind}:${st.artifact.namespace}/${st.artifact.name}`}
      <ArtifactPane
        artifact={st.artifact}
        revision={st.revision}
        onclose={close}
        onapplied={(t) => sendFromPane('applied', t)}
        onask={(t) => sendFromPane('ask', t)}
        onsection={setArtifactSection}
        onedit={setArtifactEdit}
        {onopenapp}
      />
      {/key}
    </div>
  </section>
{/if}

<style>
  .cah { position: absolute; z-index: 26; display: flex; flex-direction: column; background: var(--bg-surface); min-width: 0; }
  .cah.pop { top: 0; right: 0; bottom: 0; max-width: calc(100% - 24px); border-left: 1px solid var(--border); box-shadow: var(--shadow-lg, 0 12px 30px rgba(30, 25, 15, 0.12)); }
  /* Under the Details bar, which carries the pane's pin while it fills Details. */
  .cah.fill { top: 40px; left: 0; right: 0; bottom: 0; }
  .cah.hidden { display: none; }
  .cah-body { flex: 1 1 auto; min-height: 0; display: flex; flex-direction: column; }
  .sbname { font: 500 11px var(--font-mono); letter-spacing: 0.05em; text-transform: uppercase; color: var(--text-3); }
</style>
