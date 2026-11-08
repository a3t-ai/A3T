<script lang="ts">
  /// The A3T shell: Top bar over three stacks — Menu · Items|Chat · Details.
  ///
  /// Every stack is always mounted; pins, Expand and pop-ins only change how it
  /// is laid out (docked, full width, popped in from an edge, or hidden). So
  /// pinning or expanding never remounts a page and never loses what is on it.
  import { untrack } from 'svelte';
  import StatusBar from './StatusBar.svelte';
  import Topbar from './Topbar.svelte';
  import Sidebar from './Sidebar.svelte';
  import ContextList from './ContextList.svelte';
  import PanelStack from './PanelStack.svelte';
  import StackBar from './StackBar.svelte';
  import Resizer from './Resizer.svelte';
  import CommandPalette from './CommandPalette.svelte';
  import DetailsHost from './DetailsHost.svelte';
  import AppWelcome from './AppWelcome.svelte';
  import ChatArtifactHost from './ChatArtifactHost.svelte';
  import { getChatArtifact, setArtifactPinned } from '$lib/stores/chatArtifact.svelte.ts';
  import { setActiveApp } from '$lib/stores/activeItem.svelte.ts';
  import { getDetailActions, getItemsRefresh, actionsHosted, paneFitted } from '$lib/stores/detailActions.svelte.ts';
  import ChatPageAI from '$lib/components/pages/ChatPageAI.svelte';
  import { getActivePanel, navigateTo, navigateToDetail } from '$lib/stores/panels.svelte.ts';
  import { hasContext, APP_CONTEXT, trackOpen } from '$lib/stores/apps.svelte.ts';
  import {
    menuDocked, docked, getOverlay, openOverlay, closeOverlay, getFace, setFace, width,
    togglePin, toggleFace, togglePalette, escape, revealDetails, getToast, getChatSession,
    setChatSession, MENU_LABELS_AT, isVisible, showMid, isNarrow,
  } from '$lib/stores/layout.svelte.ts';

  const panel = $derived(getActivePanel());
  const activeType = $derived(panel?.type ?? '');
  const selectedId = $derived(panel?.props?.selectedId ?? '');
  const d = $derived(docked());
  const overlay = $derived(getOverlay());
  const face = $derived(getFace());

  /// How each stack is laid out right now.
  const menuMode = $derived(menuDocked() ? 'dock' : overlay === 'menu' ? 'pop' : 'off');
  /// Four kinds of app, one layout rule each:
  ///  - list: things you pick (Agents, Projects, Executions, Settings…): Items
  ///    lists them, Details shows the one picked.
  ///  - own: an explorer and AI of their own (Studio, Memory, Knowledge, the
  ///    consoles): the page takes the width, no Items, no Chat.
  ///  - page: one table or log with nothing to open (Decision log, Errors, Early
  ///    access): the page takes the width; Chat waits on the edge.
  ///  - chat: AI Chat itself, laid out like a list app: conversations in Items,
  ///    the open conversation in Details. No Items | Chat toggle — the app is
  ///    the chat, so the list and the conversation sit side by side.
  const OWN_EXPLORER = new Set(['flow-editor', 'memory', 'knowledge', 'db-console', 'aql-console', 'console-editor']);
  const kind = $derived<'list' | 'own' | 'page' | 'chat'>(
    activeType === 'chat' ? 'chat' : OWN_EXPLORER.has(activeType) ? 'own' : hasContext(activeType) ? 'list' : 'page');
  const ownExplorer = $derived(kind === 'own');
  const midMode = $derived(
    kind === 'own' ? 'off'
    : kind === 'page' ? (overlay === 'mid' ? 'pop' : 'off')
    : d.mid ? (d.details ? 'col' : 'wide') : overlay === 'mid' ? 'pop' : 'off');
  const detMode = $derived(
    kind === 'own' || kind === 'page' ? 'pane'
    : d.details ? 'pane' : overlay === 'details' ? 'pop' : 'off');
  /// A page app's pop-in is Chat only.
  // What the user is working on, for the chat's context: the app and its
  // selected item. Item views add what the item is and its config.
  $effect(() => { setActiveApp(activeType, panel?.title ?? '', String(selectedId)); });

  // ── The chat's editing pane (a card's item, co-edited with the agent) ──
  // It shows in Details beside the conversation that opened it: AI Chat's open
  // conversation, or the Chat face's in any other app. Popped in over Details,
  // or pinned to take Details — in AI Chat the conversation then moves to the
  // middle stack's Chat face, so the item and the conversation sit side by side.
  const art = $derived(getChatArtifact());
  const artInView = $derived(!!art && art.session === (kind === 'chat' ? selectedId : getChatSession()));
  const artPinned = $derived(artInView && !!art?.pinned);
  /// AI Chat with its item pinned: the conversation is in the middle stack.
  const chatBeside = $derived(kind === 'chat' && artPinned);
  let artHost = $state<ReturnType<typeof ChatArtifactHost>>();

  /// AI Chat's middle stack is its conversation list — and, while an item is
  /// pinned in Details, the conversation itself (the Chat face).
  const shownFace = $derived(kind === 'page' ? 'chat' : kind === 'chat' && !chatBeside ? 'items' : face);
  $effect(() => {
    if (!chatBeside) return;
    const sel = selectedId;
    untrack(() => { setChatSession(sel); setFace('chat'); });
  });
  /// Opening an item where Details is out of sight brings Details in.
  let lastArt: unknown = null;
  $effect(() => {
    const a = art?.artifact;
    untrack(() => {
      if (a && a !== lastArt && artInView && detMode === 'off') openOverlay('details');
      lastArt = a ?? null;
    });
  });
  /// Over Details when Details is docked; otherwise (pinned, or Details itself
  /// popped in) it fills Details.
  const artMode = $derived<'pop' | 'fill' | 'hidden'>(!artInView ? 'hidden' : artPinned || detMode !== 'pane' ? 'fill' : 'pop');
  /// The item in its own app: the conversation goes along in that app's Chat face.
  function openArtInApp(link: { type: string; title: string; id: string }) {
    const sess = art?.session;
    if (!artHost?.close()) return;
    if (sess) setChatSession(sess);
    trackOpen(link.type, link.title);
    navigateToDetail(link.type, link.title, link.id);
    setFace('chat');
  }

  /// Items is the list and Details is one item. For an app with a list,
  /// Details shows the app's welcome until something is picked or created;
  /// the app's old list page is not shown. Apps without a list keep their page.
  const listApp = $derived(hasContext(activeType) && !!APP_CONTEXT[activeType]?.focuses);
  const creating = $derived(!!panel?.props?.create);
  // AI Chat with nothing picked is a new conversation, not a welcome page.
  const nothingSelected = $derived(listApp && kind !== 'chat' && !selectedId && !creating);
  const hasDetails = $derived(!nothingSelected);

  /// Apps whose item opens as a view fitted over the app's own page. While an
  /// item is shown, the page under it (its old list, banner and New button) is
  /// never visible — not while the item opens, and not during any transition.
  /// If no view arrives (the item is gone, or failed to load) the page shows
  /// after a few seconds rather than leaving Details blank.
  const FITS_ITEM = new Set(['agents', 'templates', 'triggers', 'projects', 'resources', 'executions', 'canvas']);
  let gaveUp = $state(false);
  $effect(() => {
    void selectedId; void activeType;
    gaveUp = false;
    const t = setTimeout(() => (gaveUp = true), 4000);
    return () => clearTimeout(t);
  });
  const covering = $derived(FITS_ITEM.has(activeType) && (!!selectedId || creating) && (paneFitted() || !gaveUp));
  const opening = $derived(covering && !paneFitted());

  // A create form closed (its actions went away): drop the request from the
  // URL so Details goes back to "Nothing selected" and New works again.
  let hadActions = false;
  $effect(() => {
    const has = !!getDetailActions();
    untrack(() => {
      if (hadActions && !has && panel?.props?.create && !panel?.props?.selectedId) navigateTo(activeType, panel.title);
      hadActions = has;
    });
  });

  // Chat mounts the first time it is shown, then stays mounted so a reply keeps
  // streaming while you look at Items.
  let chatMounted = $state(false);
  $effect(() => { if (shownFace === 'chat' && midMode !== 'off') chatMounted = true; });

  // Selecting an item puts Details where it can be seen. An app with no list
  // has nothing to pick in Items, so its page is shown straight away.
  let lastSel = '';
  let lastType = '';
  let lastCreate: unknown = undefined;
  $effect(() => {
    const sel = selectedId, type = activeType, create = panel?.props?.create;
    untrack(() => {
      const selChanged = (sel !== lastSel && !!sel) || (!!create && create !== lastCreate);
      lastCreate = create;
      const typeChanged = type !== lastType;
      lastSel = sel; lastType = type;
      if (selChanged || (typeChanged && type && !(hasContext(type) && APP_CONTEXT[type]?.focuses))) revealDetails();
    });
  });

  // A pop-in closes on a click outside it. Edge tabs and the top bar's own
  // controls are left alone so they can open or swap what floats.
  let menuEl = $state<HTMLElement>();
  let midEl = $state<HTMLElement>();
  let detEl = $state<HTMLElement>();
  let pageEl = $state<HTMLElement>();
  $effect(() => {
    const o = overlay;
    if (!o) return;
    const el = o === 'menu' ? menuEl : o === 'mid' ? midEl : detEl;
    const onDown = (e: PointerEvent) => {
      const t = e.target as Element | null;
      if (!t || el?.contains(t) || t.closest('.edge, .scrim, [role="dialog"], .smt')) return;
      closeOverlay();
    };
    const id = setTimeout(() => window.addEventListener('pointerdown', onDown, true));
    return () => { clearTimeout(id); window.removeEventListener('pointerdown', onDown, true); };
  });

  function onKey(e: KeyboardEvent) {
    const mod = e.ctrlKey || e.metaKey;
    const k = e.key.toLowerCase();
    if (mod && k === 'k') { e.preventDefault(); togglePalette(); }
    else if (mod && k === 'j') {
      e.preventDefault();
      if (kind === 'page') { overlay === 'mid' ? closeOverlay() : openOverlay('mid'); }
      else if (kind === 'list') toggleFace();
      else if (kind === 'chat' && !d.mid) { overlay === 'mid' ? closeOverlay() : openOverlay('mid'); }
    }
    else if (mod && e.key === '.') { e.preventDefault(); togglePin('details'); }
    else if (mod && e.key === '\\') { e.preventDefault(); togglePin('menu'); }
    else if (e.key === 'Escape' && !e.defaultPrevented) {
      // A popped-in item closes first; pinned, it stays until unpinned.
      if (artMode === 'pop') { e.preventDefault(); artHost?.close(); }
      else if (escape()) e.preventDefault();
    }
  }

  function newConversation() { setChatSession(''); setFace('chat'); }
</script>

<svelte:window onkeydown={onKey} />

<div class="frame">
  <StatusBar />
  <Topbar />
  <div class="body">
    <div class="menu {menuMode}" bind:this={menuEl} style:width="{width('menu')}px">
      <Sidebar wide={width('menu') >= MENU_LABELS_AT} />
      <!-- The menu's shoulder: hide it (docked) or keep it (popped in). -->
      <button class="shoulder" onclick={() => togglePin('menu')}
        title={menuMode === 'dock' ? 'Hide the menu (Ctrl \\)' : 'Keep the menu open (Ctrl \\)'}
        aria-label={menuMode === 'dock' ? 'Hide the menu' : 'Keep the menu open'}>
        <svg width="12" height="12" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
          {#if menuMode === 'dock'}<path d="M10 3L5 8l5 5" />{:else}<path d="M6 3l5 5-5 5" />{/if}
        </svg>
      </button>
      {#if menuMode === 'pop'}<Resizer k="menu" label="menu" overlay />{/if}
    </div>
    {#if menuMode === 'dock'}<Resizer k="menu" label="menu" />{/if}
    {#if menuMode === 'off' && !isNarrow()}
      <button class="shoulder out" onclick={() => togglePin('menu')} title="Show the menu (Ctrl \)" aria-label="Show the menu">
        <svg width="12" height="12" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M6 3l5 5-5 5" /></svg>
      </button>
    {/if}

    <section class="mid {midMode}" bind:this={midEl} aria-label="Items and Chat"
      style:flex-basis={midMode === 'col' ? `${width('mid')}px` : undefined}
      style:width={midMode === 'pop' ? `${width('chatPop')}px` : undefined}>
      {#if midMode === 'pop'}<Resizer k="chatPop" edge="left" label="Items and Chat" overlay />{/if}
      <StackBar stack="mid">
        {#snippet right()}
          {#if shownFace === 'items' && getItemsRefresh()}
            {@const r = getItemsRefresh()!}
            <button class="da" onclick={r.run} disabled={r.busy} title="Refresh" aria-label="Refresh">
              <svg width="13" height="13" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5"><path d="M14 8A6 6 0 112.6 5.5" /><path d="M2 2v4h4" /></svg>
            </button>
          {/if}
        {/snippet}
        {#snippet left()}
          {#if kind === 'page'}
            <span class="sbname">Chat</span>
          {:else if kind === 'chat' && !chatBeside}
            <span class="sbname">Items</span>
          {:else}
            <div class="seg" role="group" aria-label="Show">
              <button aria-pressed={face === 'items'} onclick={() => setFace('items')}>Items</button>
              <button aria-pressed={face === 'chat'} onclick={() => setFace('chat')}>Chat</button>
            </div>
          {/if}
          {#if shownFace === 'chat' && getChatSession() && kind !== 'chat'}
            <button class="sb-btn" onclick={newConversation} title="New conversation" aria-label="New conversation">
              <svg width="13" height="13" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.6"><path d="M8 3v10M3 8h10" /></svg>
            </button>
          {/if}
        {/snippet}
      </StackBar>
      <div class="face" class:hidden={shownFace !== 'items'}>
        {#if hasContext(activeType)}
          {#key activeType}<ContextList appType={activeType} />{/key}
        {:else}
          <div class="note">
            <b>{panel?.title ?? 'This app'} has no list</b>
            <span>Its page is in Details. Switch to Chat to ask about it.</span>
            {#if !d.details}<button class="link" onclick={() => openOverlay('details')}>Open Details</button>{/if}
          </div>
        {/if}
      </div>
      <div class="face" class:hidden={shownFace !== 'chat'}>
        <!-- One copy of a conversation at a time, or two would each answer the
             pane and the agent: in AI Chat the conversation is in Details (it
             comes here only while an item is pinned beside it), and the Studio
             has the chat beside its Explorer. -->
        {#if chatMounted && kind !== 'own' && (kind !== 'chat' || chatBeside)}
          {#key getChatSession()}<ChatPageAI docked />{/key}
        {/if}
      </div>
    </section>
    {#if midMode === 'col'}<Resizer k="mid" label="Items and Chat" />{/if}

    <section class="details {detMode}" bind:this={detEl} aria-label="Details"
      style:width={detMode === 'pop' ? `${width('detailsPop')}px` : undefined}>
      {#if detMode === 'pop'}<Resizer k="detailsPop" edge="left" label="Details" overlay />{/if}
      <StackBar stack="details"
        pin={artMode === 'fill' ? { on: artPinned, toggle: () => setArtifactPinned(!artPinned), what: artPinned ? 'it from beside the conversation' : 'this beside the conversation' } : undefined}
        onclose={artMode === 'fill' && !artPinned ? () => artHost?.close() : undefined}>
        {#snippet left()}<span class="sbname">Details</span>{/snippet}
        {#snippet right()}
          {#if getDetailActions() && !actionsHosted()}
            {@const a = getDetailActions()!}
            <span class="dacts">
              {#if a.onopenineditor}
                <button class="da" onclick={a.onopenineditor} title="Open in Studio" aria-label="Open in Studio">
                  <svg width="14" height="14" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5"><path d="M9 1h6v6M9 7L15 1M7 3H2a1 1 0 00-1 1v10a1 1 0 001 1h10a1 1 0 001-1V9" /></svg>
                </button>
              {/if}
              {#each a.extra ?? [] as x (x.label)}
                <button class="da" onclick={x.run}>{x.label}</button>
              {/each}
              {#if a.onsave}
                <button class="da save" onclick={a.onsave} title="Save" aria-label="Save">
                  <svg width="14" height="14" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5"><path d="M13 15H3a1 1 0 01-1-1V2a1 1 0 011-1h8l3 3v10a1 1 0 01-1 1z" /><path d="M11 15V9H5v6M5 1v4h5" /></svg>
                  <span>Save</span>
                </button>
              {/if}
              <span class="dsep" aria-hidden="true"></span>
            </span>
          {/if}
        {/snippet}
      </StackBar>
      <div class="page" class:covering bind:this={pageEl}>
        {#if opening}<div class="opening-note" role="status">Opening…</div>{/if}
        {#if nothingSelected}
          <AppWelcome appType={activeType} title={panel?.title ?? 'App'}
            onitems={!isVisible('mid') || getFace() !== 'items' ? () => showMid('items') : undefined} />
        {:else if !chatBeside}
          <DetailsHost host={() => pageEl}><PanelStack /></DetailsHost>
        {/if}
      </div>
      <ChatArtifactHost bind:this={artHost} mode={artMode} onopenapp={openArtInApp} />
    </section>

    {#if midMode === 'off' && kind !== 'own'}
      <button class="edge" onclick={() => openOverlay('mid')} title={kind === 'page' ? 'Open Chat (Ctrl J)' : kind === 'chat' ? 'Open Items (Ctrl J)' : 'Open Items | Chat (Ctrl J)'}>{kind === 'page' ? 'Chat' : kind === 'chat' ? 'Items' : 'Items · Chat'}</button>
    {/if}
    {#if detMode === 'off' && hasDetails}
      <button class="edge" style:top={midMode === 'off' ? '210px' : '90px'} onclick={() => openOverlay('details')} title="Open Details">Details</button>
    {/if}
  </div>
</div>

<CommandPalette />
{#if getToast()}<div class="toast" role="status">{getToast()}</div>{/if}

<style>
  .frame { height: calc(100vh - 12px); margin: 6px; background: var(--bg-page); border: 1px solid var(--border); border-radius: 16px 16px 0 0; overflow: hidden; display: flex; flex-direction: column; }
  .body { flex: 1 1 auto; display: flex; min-height: 0; position: relative; }

  /* Menu: docked at the left edge, or popped in over the page from it. */
  .menu { flex: 0 0 auto; min-width: 0; position: relative; display: flex; flex-direction: column; }
  .menu.off { display: none; }
  .menu.pop { position: absolute; left: 0; top: 0; bottom: 0; z-index: 30; box-shadow: var(--shadow-lg, 0 12px 30px rgba(30, 25, 15, 0.12)); }

  /* Items | Chat */
  .mid { min-width: 0; min-height: 0; display: flex; flex-direction: column; background: var(--bg-page); position: relative; }
  .mid.col { flex: 0 0 auto; border-right: 1px solid var(--border); }
  .mid.wide { flex: 1 1 auto; }
  .mid.pop, .details.pop { position: absolute; top: 0; right: 0; bottom: 0; z-index: 25; border-left: 1px solid var(--border); box-shadow: var(--shadow-lg, 0 12px 30px rgba(30, 25, 15, 0.12)); max-width: calc(100% - 24px); }
  .mid.off, .details.off { display: none; }
  .face { flex: 1 1 auto; min-height: 0; display: flex; flex-direction: column; overflow: hidden; }
  .face.hidden { display: none; }

  /* Details */
  .details { flex: 1 1 auto; min-width: 0; min-height: 0; display: flex; flex-direction: column; background: var(--bg-page); position: relative; }
  .page { flex: 1 1 auto; min-height: 0; position: relative; overflow: hidden; }
  /* While an item opens, only the item's own view may show. */
  .page.covering > :global(*:not(.opening-note)) { visibility: hidden; }
  .page.covering :global([data-fitted]) { visibility: visible; }
  .opening-note { position: absolute; inset: 0; display: flex; align-items: flex-start; justify-content: center; padding-top: 64px; font-size: 13px; color: var(--text-3); }
  .sbname { font: 500 11px var(--font-mono); letter-spacing: 0.05em; text-transform: uppercase; color: var(--text-3); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }

  /* The open item's actions, in the Details bar before Expand and Pin. */
  .dacts { display: inline-flex; align-items: center; gap: 4px; }
  .da { height: 28px; min-width: 28px; padding: 0 7px; border-radius: 7px; border: 1px solid transparent; background: transparent; display: inline-flex; align-items: center; justify-content: center; gap: 5px; color: var(--text-3); cursor: pointer; font: 500 12.5px var(--font-sans); }
  .da:disabled { opacity: 0.4; cursor: default; }
  .da:hover { background: var(--bg-subtle); border-color: var(--border); color: var(--text-1); }
  .da.save { border-color: var(--border); color: var(--text-1); background: var(--bg-surface); }
  .da.save:hover { border-color: var(--green, #1a7158); color: var(--green, #1a7158); }
  .dsep { width: 1px; height: 18px; background: var(--border); margin: 0 4px; }

  .seg { display: inline-flex; gap: 2px; padding: 3px; background: var(--bg-subtle); border: 1px solid var(--border); border-radius: 9px; }
  .seg button { border: 0; border-radius: 7px; background: transparent; padding: 3px 10px; font: 500 12px var(--font-sans); color: var(--text-3); cursor: pointer; }
  .seg button[aria-pressed='true'] { background: var(--bg-surface); color: var(--text-1); box-shadow: 0 1px 2px rgba(0, 0, 0, 0.05); }
  .sb-btn { width: 28px; height: 28px; margin-left: 4px; border-radius: 7px; border: 1px solid var(--border); background: var(--bg-surface); display: inline-flex; align-items: center; justify-content: center; color: var(--text-2); cursor: pointer; }
  .sb-btn:hover { border-color: var(--spec); color: var(--spec); }

  .note { margin: 40px 20px; display: flex; flex-direction: column; gap: 6px; align-items: center; text-align: center; color: var(--text-3); font-size: 13px; }
  .note b { color: var(--text-1); font-weight: 600; }
  .link { margin-top: 6px; border: 1px solid var(--border); border-radius: 7px; background: var(--bg-surface); padding: 5px 12px; font: 500 12px var(--font-sans); color: var(--spec); cursor: pointer; }

  /* The menu's shoulder arrow: on its right edge near the top, half outside. */
  .shoulder { position: absolute; top: 14px; right: -11px; z-index: 31; width: 22px; height: 22px; border-radius: 50%; border: 1px solid var(--border); background: var(--bg-surface); color: var(--text-3); display: grid; place-items: center; padding: 0; cursor: pointer; box-shadow: 0 1px 3px rgba(30, 25, 15, 0.08); transition: color 0.12s, border-color 0.12s; }
  .shoulder:hover { color: var(--spec); border-color: var(--spec); }
  .shoulder.out { right: auto; left: -1px; border-radius: 0 11px 11px 0; border-left: 0; width: 18px; }

  /* Edge tab: where an unpinned stack waits. */
  .edge { position: absolute; right: 0; top: 90px; z-index: 20; writing-mode: vertical-rl; transform: rotate(180deg); background: var(--spec); color: #fff; border: 0; border-radius: 0 8px 8px 0; padding: 12px 6px; font: 600 11px var(--font-mono); letter-spacing: 0.05em; cursor: pointer; }
  .edge:focus-visible { outline: 2px solid var(--spec); outline-offset: 2px; }

  .toast { position: fixed; left: 50%; top: 70px; transform: translateX(-50%); background: var(--text-1); color: var(--bg-surface); border-radius: 10px; padding: 8px 14px; font-size: 13px; z-index: 210; pointer-events: none; }

  :global(body.resizing) { cursor: col-resize; user-select: none; }

  @media (max-width: 720px) {
    .mid.pop, .details.pop, .menu.pop { width: calc(100% - 16px) !important; max-width: none; }
  }
</style>
