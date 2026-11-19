<script lang="ts">
  /// The Console's Studio page (menu: Studio; /studio, and /flow-editor for old
  /// links), hosted by the web dash. The Studio is used only through its entry point.
  import SubmenuToggle from '$lib/components/shared/SubmenuToggle.svelte';
  import { Studio, STUDIOS } from '$lib/studio-ui';
  import { webHost } from '$lib/host/web.svelte.ts';
  import { getActivePanel } from '$lib/stores/panels.svelte.ts';
  import { parseStudioItem } from '$lib/stores/panels.svelte.ts';
  import ChatPageAI from '$lib/components/pages/ChatPageAI.svelte';
  import { trackActiveItem } from '$lib/stores/activeItem.svelte.ts';
  import { getChatSession } from '$lib/stores/layout.svelte.ts';
  import type { Workspace } from '$lib/studio-core';

  // An item named in the address (/studio/<kind>:<project>~<name>, or the older
  // /studio/<project>~<name> for a flow) opens on arrival, and again whenever an
  // "Open in Studio" elsewhere names one (each navigation is a new panel id).
  const request = $derived.by(() => {
    const p = getActivePanel();
    if (p?.type !== 'flow-editor') return null;
    const item = parseStudioItem(p.props?.selectedId);
    return item ? { item, at: p.id } : null;
  });

  // The chat beside the Explorer knows the open document: what it is, the view
  // on screen, and its text when it differs from the registry copy. The agent's
  // proposed edits (propose_edit) go into the document, unsaved, like typing.
  let ws = $state<Workspace | null>(null);
  trackActiveItem(() => {
    const d = ws?.active;
    if (!d || !ws) return null;
    const isText = typeof d.text === 'string';
    const w = ws;
    return {
      kind: d.ref.kind, namespace: d.ref.project, name: d.ref.name, id: d.ref.id ?? `${d.ref.project}/${d.ref.name}`,
      // The flow designer reports Graph / Code; other views are the editor or code.
      section: d.ui.section ?? (d.ui.view === 'code' ? 'code' : 'editor'),
      text: isText ? d.text : undefined,
      dirty: isText && (!!d.isNew || d.text !== (d.saved ?? '')),
      applyEdit: isText && w.kindOf(d).push ? (t: string) => { d.text = t; } : undefined,
    };
  });
</script>

<div class="page">
  <Studio host={webHost} config={STUDIOS.flows} {request} onworkspace={(w) => (ws = w)}>
    {#snippet lead()}<SubmenuToggle appType="flow-editor" />{/snippet}
    {#snippet chat()}{#key getChatSession()}<ChatPageAI docked />{/key}{/snippet}
  </Studio>
</div>

<style>
  .page { height: 100%; min-height: 0; }
</style>
