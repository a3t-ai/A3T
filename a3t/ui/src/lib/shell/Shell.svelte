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
  import { getDetailActions, getItemsRefresh, actionsHosted } from '$lib/stores/detailActions.svelte.ts';
  import ChatPageAI from '$lib/components/pages/ChatPageAI.svelte';
  import { getActivePanel, navigateTo } from '$lib/stores/panels.svelte.ts';
  import { hasContext, APP_CONTEXT } from '$lib/stores/apps.svelte.ts';
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
  ///  - chat: AI Chat itself. Conversations in Items, the conversation in the
  ///    Chat face across the width; nothing in Details.
  const OWN_EXPLORER = new Set(['flow-editor', 'memory', 'knowledge', 'db-console', 'aql-console', 'console-editor']);
  const kind = $derived<'list' | 'own' | 'page' | 'chat'>(
    activeType === 'chat' ? 'chat' : OWN_EXPLORER.has(activeType) ? 'own' : hasContext(activeType) ? 'list' : 'page');
  const ownExplorer = $derived(kind === 'own');
  const midMode = $derived(
    kind === 'own' ? 'off'
    : kind === 'page' ? (overlay === 'mid' ? 'pop' : 'off')
    : kind === 'chat' ? 'wide'
    : d.mid ? (d.details ? 'col' : 'wide') : overlay === 'mid' ? 'pop' : 'off');
  const detMode = $derived(
    kind === 'own' || kind === 'page' ? 'pane'
    : kind === 'chat' ? 'off'
    : d.details ? 'pane' : overlay === 'details' ? 'pop' : 'off');
  /// A page app's pop-in is Chat only.
  const shownFace = $derived(kind === 'page' ? 'chat' : face);

  // AI Chat: a conversation picked in Items (or a /chat/<id> link) opens in the
  // Chat face; arriving in the app shows Chat.
  let lastChatSel = '';
  $effect(() => {
    const k = kind, sel = selectedId;
    untrack(() => {
      if (k !== 'chat') { lastChatSel = ''; return; }
      if (sel && sel !== lastChatSel) { setChatSession(sel); setFace('chat'); }
      else if (!lastChatSel && !sel) setFace('chat');
      lastChatSel = sel || '·';
    });
  });

  /// Items is the list and Details is one item. For an app with a list that
  /// can open an item, Details stays empty until something is picked; its main
  /// page (with create and the other app-wide actions) is one click away as the
  /// overview. Apps without a list keep their page in Details.
  let overviewFor = $state<Record<string, boolean>>({});
  const listApp = $derived(hasContext(activeType) && !!APP_CONTEXT[activeType]?.focuses);
  const creating = $derived(!!panel?.props?.create);
  const nothingSelected = $derived(listApp && !selectedId && !creating && !overviewFor[activeType]);
  const hasDetails = $derived(!nothingSelected);
  function showOverview() { overviewFor = { ...overviewFor, [activeType]: true }; }
  // Picking an item and coming back to the app starts from the empty state again.
  $effect(() => { if (selectedId) untrack(() => { if (overviewFor[activeType]) overviewFor = { ...overviewFor, [activeType]: false }; }); });

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
      // Coming into an app starts from its list, not a previously opened overview.
      if (typeChanged && Object.keys(overviewFor).length) overviewFor = {};
      if (type === 'chat') return;
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
      else if (kind !== 'own') toggleFace();
    }
    else if (mod && e.key === '.') { e.preventDefault(); togglePin('details'); }
    else if (mod && e.key === '\\') { e.preventDefault(); togglePin('menu'); }
    else if (e.key === 'Escape' && !e.defaultPrevented) { if (escape()) e.preventDefault(); }
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
          {:else}
            <div class="seg" role="group" aria-label="Show">
              <button aria-pressed={face === 'items'} onclick={() => setFace('items')}>Items</button>
              <button aria-pressed={face === 'chat'} onclick={() => setFace('chat')}>Chat</button>
            </div>
          {/if}
          {#if shownFace === 'chat' && getChatSession()}
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
        {#if chatMounted}
          {#key getChatSession()}<ChatPageAI docked />{/key}
        {/if}
      </div>
    </section>
    {#if midMode === 'col'}<Resizer k="mid" label="Items and Chat" />{/if}

    <section class="details {detMode}" bind:this={detEl} aria-label="Details"
      style:width={detMode === 'pop' ? `${width('detailsPop')}px` : undefined}>
      {#if detMode === 'pop'}<Resizer k="detailsPop" edge="left" label="Details" overlay />{/if}
      <StackBar stack="details">
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
      <div class="page" bind:this={pageEl}>
        {#if nothingSelected}
          <AppWelcome appType={activeType} title={panel?.title ?? 'App'} onoverview={showOverview}
            onitems={!isVisible('mid') || getFace() !== 'items' ? () => showMid('items') : undefined} />
        {:else}
          {#if kind !== 'chat'}<DetailsHost host={() => pageEl}><PanelStack /></DetailsHost>{/if}
        {/if}
      </div>
    </section>

    {#if midMode === 'off' && kind !== 'own'}
      <button class="edge" onclick={() => openOverlay('mid')} title={kind === 'page' ? 'Open Chat (Ctrl J)' : 'Open Items | Chat (Ctrl J)'}>{kind === 'page' ? 'Chat' : 'Items · Chat'}</button>
    {/if}
    {#if detMode === 'off' && hasDetails && kind !== 'chat'}
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
