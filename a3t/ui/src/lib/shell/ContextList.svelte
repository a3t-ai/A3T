<script lang="ts">
  /// The per-app list: conversations in Chat, flows in the Studio, resources in
  /// Resources. It is the Items face of the shell's middle stack; the stack bar
  /// above it owns pinning, and selecting an item opens it in Details.
  import { navigateTo, navigateToDetail, getActivePanel } from '$lib/stores/panels.svelte.ts';
  import {
    APP_CONTEXT, contextLabel, settleContextPane, trackOpen, contextRev_, getPins, isPinned, setPins,
  } from '$lib/stores/apps.svelte.ts';
  import { deleteChatSession, setPin } from '$lib/services/api';
  import { getProject } from '$lib/stores/context.svelte.ts';
  import { setItemsRefresh } from '$lib/stores/detailActions.svelte.ts';
  import { loadAppItems, groupByNamespace, type AppItem } from './appItems';

  let { appType }: { appType: string } = $props();

  type Item = AppItem;

  let items = $state<Item[]>([]);
  let loading = $state(false);
  let error = $state('');
  let filter = $state('');

  // Sort, remembered per app in this browser.
  let sortBy = $state<'recent' | 'name'>('recent');
  let viewLoadedFor = '';
  $effect(() => {
    const key = `a3t.items.view.${appType}`;
    if (viewLoadedFor === key) return;
    viewLoadedFor = key;
    try {
      const v = JSON.parse(localStorage.getItem(key) ?? '{}');
      sortBy = v.sort === 'name' ? 'name' : 'recent';
    } catch { sortBy = 'recent'; }
  });
  function saveView() {
    try { localStorage.setItem(`a3t.items.view.${appType}`, JSON.stringify({ sort: sortBy })); } catch {}
  }
  const sorted = (list: AppItem[]) => sortBy === 'name' ? list.slice().sort((a, b) => a.label.localeCompare(b.label)) : list;

  /// Row icon colour per app, matching the accent each app's own cards use.
  const COLOR: Record<string, string> = {
    agents: '#1a7158', templates: '#5530a8', canvas: '#b45309', triggers: '#c97c2e',
    resources: '#3a5a85', projects: '#5530a8', chat: '#3a5a85', executions: '#55514a',
    runtime: '#326ce5', security: '#6b5a3a',
  };
  const color = $derived(COLOR[appType] ?? '#8a8478');
  const initials = (label: string) => label.replace(/[^a-z0-9]+/gi, ' ').trim().split(' ')
    .filter(Boolean).slice(0, 2).map((w) => w[0]).join('').toUpperCase() || '·';
  const crumb = $derived([getProject() || 'All projects', getActivePanel()?.title ?? contextLabel(appType)].join(' / '));

  async function togglePin(e: MouseEvent, item: AppItem) {
    e.stopPropagation();
    try { setPins(await setPin(appType, item.id, !isPinned(appType, item.id))); } catch {}
  }
  /// Plain `let`, not `$state`: the effect below both reads and writes it, and as
  /// reactive state that self-retriggers — the shape that trips Svelte's
  /// update-depth guard and froze reactivity elsewhere in this app.
  let loadedFor = '';

  const spec = $derived(APP_CONTEXT[appType]);
  const activeId = $derived(getActivePanel()?.props?.selectedId ?? '');

  /// A conversation is created server-side by its FIRST TURN, so a chat you have
  /// just opened is in no list until you send something — which reads as the new
  /// session having been lost. Show the open one regardless, marked as unsent.
  const listed = $derived.by<Item[]>(() => {
    if (spec?.source !== 'sessions' || !activeId) return items;
    if (items.some((i) => i.id === activeId)) return items;
    const unsent: Item = { id: activeId, label: 'New conversation', sub: 'not sent yet', pending: true };
    return [unsent, ...items];
  });

  /// A hierarchical list (items carry `depth`) renders as a tree instead of
  /// namespace groups.
  const isTree = $derived(listed.some((i) => i.depth != null));

  /// Each tree row's ancestors, from the depth-first order.
  const ancestors = $derived.by(() => {
    const m = new Map<string, string[]>();
    const stack: Item[] = [];
    for (const it of listed) {
      const d = it.depth ?? 0;
      while (stack.length && (stack[stack.length - 1].depth ?? 0) >= d) stack.pop();
      m.set(it.id, stack.map((s) => s.id));
      stack.push(it);
    }
    return m;
  });
  const hasChildren = (i: number) => (listed[i + 1]?.depth ?? -1) > (listed[i]?.depth ?? 0);

  const shown = $derived.by<Item[]>(() => {
    const q = filter.trim().toLowerCase();
    if (isTree) {
      if (!q) return listed.filter((i) => !(ancestors.get(i.id) ?? []).some((a) => collapsed.has(a)));
      // A match keeps its ancestors, so it is still shown where it lives.
      const keep = new Set<string>();
      for (const i of listed) {
        if (i.label.toLowerCase().includes(q)) { keep.add(i.id); for (const a of ancestors.get(i.id) ?? []) keep.add(a); }
      }
      return listed.filter((i) => keep.has(i.id));
    }
    if (!q) return listed;
    // Namespace is matched too, so typing "a3t" narrows to that group.
    return listed.filter((i) =>
      i.label.toLowerCase().includes(q) || (i.namespace ?? '').toLowerCase().includes(q));
  });

  /// Rows grouped under their namespace, or null for lists that have none
  /// (conversations, resource categories) — those stay flat.
  const groups = $derived(groupByNamespace(shown, spec?.source === 'settings-sections' || spec?.source === 'resources'));

  /// Pinned items first, as their own group; they stay in their namespace too.
  const pinnedShown = $derived.by<Item[]>(() => {
    const ids = getPins(appType);
    return ids.map((id) => shown.find((i) => i.id === id)).filter(Boolean) as Item[];
  });

  // ── Collapsed namespace groups, remembered per app in this browser ──
  const collapseKey = $derived(`a3t.ctx.collapsed.${appType}`);
  let collapsed = $state<Set<string>>(new Set());
  /// Plain `let` for the same reason as `loadedFor`: read and written below.
  let collapseLoadedFor = '';
  $effect(() => {
    const key = collapseKey;
    if (collapseLoadedFor === key) return;
    collapseLoadedFor = key;
    try { collapsed = new Set(JSON.parse(localStorage.getItem(key) ?? '[]')); }
    catch { collapsed = new Set(); }
  });
  function toggleGroup(ns: string) {
    const next = new Set(collapsed);
    next.has(ns) ? next.delete(ns) : next.add(ns);
    collapsed = next;
    try { localStorage.setItem(collapseKey, JSON.stringify([...next])); } catch {}
  }
  /// A filter shows every match: a collapsed group must not hide what was searched for.
  const isOpen = (ns: string) => !!filter.trim() || !collapsed.has(ns);

  /// Dot colour for a short state word. Unknown states get no dot rather than a
  /// guessed colour.
  function statusTone(s?: string): string | null {
    if (!s) return null;
    const v = s.toLowerCase();
    if (['completed', 'active', 'running', 'enabled', 'healthy', 'synced'].includes(v)) return v === 'running' ? 'run' : 'ok';
    if (['failed', 'error', 'cancelled', 'degraded'].includes(v)) return 'bad';
    if (['draft', 'disabled', 'queued', 'pending', 'hibernated'].includes(v)) return 'idle';
    // Paused is deliberate but not serving — it must stand out from "asleep".
    if (v === 'paused') return 'bad';
    return null;
  }


  /// Only the latest load may write: when the project list arrives mid-load a
  /// second load starts, and the first (made with the old project list) must
  /// not land after it and overwrite it.
  let loadSeq = 0;
  async function load(source: string) {
    const seq = ++loadSeq;
    loading = true;
    error = '';
    try {
      const got = await loadAppItems(source);
      if (seq === loadSeq) items = got;
    } catch (e: any) {
      if (seq === loadSeq) { error = e?.message ?? 'Failed to load'; items = []; }
    } finally {
      if (seq === loadSeq) loading = false;
    }
  }

  // Reload when the app changes, or when the app signals it changed the data —
  // a first chat turn creates the session row, and without the revision the new
  // conversation stayed missing from this list until a manual refresh.
  let loadedRev = -1;
  $effect(() => {
    const t = appType;
    const rev = contextRev_();
    const src = APP_CONTEXT[t]?.source;
    if (!src) { items = []; loadedFor = ''; return; }
    if (loadedFor === t && loadedRev === rev) return;
    loadedFor = t;
    loadedRev = rev;
    load(src);
  });

  // Refresh lives in the stack's control bar, not beside the main action.
  $effect(() => {
    const src = spec?.source;
    if (!src) return;
    return setItemsRefresh(() => load(src), loading);
  });

  function select(item: Item) {
    if (item.group) { toggleGroup(item.id); return; }
    const panel = getActivePanel();
    const title = panel?.title ?? contextLabel(appType);
    trackOpen(appType, title);
    if (spec?.focuses) navigateToDetail(appType, title, item.id);
    else navigateTo(appType, title);
    // The item opens in Details; a popped-in Details or list follows from that.
    settleContextPane();
  }

  let asideEl: HTMLElement | undefined = $state();

  /// Every app's Items header has one main action. Most open the app's create
  /// form in Details (a fresh `create` value each time, so a second click opens
  /// a fresh form); a few go where creating actually happens.
  const NEW_LABEL: Record<string, string> = {
    chat: 'New conversation', projects: 'New project', agents: 'New agent', templates: 'New template',
    triggers: 'New trigger', resources: 'New resource', canvas: 'New canvas', runtime: 'New runtime',
    security: 'New policy', executions: 'Run a flow',
  };
  const newLabel = $derived(NEW_LABEL[appType] ?? (spec?.source === 'flows' ? 'New flow' : ''));
  function newItem() {
    if (spec?.source === 'sessions') return newChat();
    if (spec?.source === 'flows') return newFlow();
    if (appType === 'executions') { trackOpen('flow-editor', 'Studio'); navigateTo('flow-editor', 'Studio'); settleContextPane(); return; }
    const title = getActivePanel()?.title ?? contextLabel(appType);
    trackOpen(appType, title);
    navigateTo(appType, title, appType === 'security' ? { selectedId: 'policies', create: Date.now() } : { create: Date.now() });
    settleContextPane();
  }

  function newChat() {
    trackOpen('chat', 'AI Chat');
    // A fresh client id; the transcript row appears once the first turn completes.
    const id = `session-${Date.now()}-${Math.random().toString(36).slice(2, 8)}`;
    navigateToDetail('chat', 'AI Chat', id);
    settleContextPane();
  }

  /// A blank editor: navigating with no selection is what FlowEditor treats as a
  /// new flow, so there is no separate "new" state to keep in sync.
  function newFlow() {
    trackOpen('flow-editor', 'Studio');
    navigateTo('flow-editor', 'Studio');
    settleContextPane();
  }

  async function removeItem(e: MouseEvent, item: Item) {
    e.stopPropagation();
    if (spec?.source !== 'sessions' || !item.serverId) return;
    // Hard delete, and the only thing that removes a transcript (sessions have no
    // TTL) — so drop it from the list only once the server has confirmed.
    const ok = await deleteChatSession(item.serverId);
    if (!ok) return;
    items = items.filter((i) => i.id !== item.id);
    // Deleting the conversation you are READING left it on screen: the chat still
    // pointed at it, and since it was no longer in the list it even reappeared
    // here as a phantom "New conversation". Move to a genuinely new one.
    if (item.id === activeId) newChat();
  }
</script>

{#snippet row(item: Item)}
  {@const idx = listed.indexOf(item)}
  {@const tone = statusTone(item.status)}
  <div class="ctx-row" class:active={item.id === activeId} class:pending={item.pending}
       style={item.depth != null ? `margin-left:${item.depth * 14}px` : ''}>
    {#if isTree}
      {#if hasChildren(idx)}
        <button class="ctx-twist" onclick={() => toggleGroup(item.id)} aria-expanded={isOpen(item.id)} aria-label="Expand">
          <svg class="ctx-chev" class:open={isOpen(item.id)} width="9" height="9" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="2.2"><path d="M6 3l5 5-5 5" /></svg>
        </button>
      {:else}
        <span class="ctx-twist"></span>
      {/if}
    {/if}
    <button class="ctx-pick" onclick={() => select(item)} title={item.namespace ? `${item.namespace}/${item.label}` : item.label}
      aria-current={item.id === activeId}>
      {#if item.badge}
        <span class="ctx-badge" style={item.badgeStyle}>{item.badge}</span>
      {:else}
        <span class="ctx-icon-sq" style:background={color}>{initials(item.label)}</span>
      {/if}
      <span class="ctx-text">
        <span class="ctx-label"><span class="ctx-name">{item.label}</span>{#if item.namespace && !groups}<span class="ctx-ns">{item.namespace}</span>{/if}</span>
        {#if item.sub}<span class="ctx-sub">{item.sub}</span>{/if}
      </span>
      {#if item.status}
        <span class="ctx-st {tone ?? ''}">{item.status}</span>
      {/if}
    </button>
    {#if !item.pending && !item.group && spec?.focuses}
      <span class="ctx-acts">
        {#if spec?.source === 'sessions'}
          <button class="ctx-rb" onclick={(e) => removeItem(e, item)} title="Delete — this cannot be undone" aria-label="Delete">
            <svg width="13" height="13" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.6"><path d="M12 4L4 12M4 4l8 8" /></svg>
          </button>
        {/if}
        <button class="ctx-rb" class:on={isPinned(appType, item.id)} onclick={(e) => togglePin(e, item)}
          title={isPinned(appType, item.id) ? 'Unpin' : 'Pin to top'} aria-label={isPinned(appType, item.id) ? 'Unpin' : 'Pin to top'} aria-pressed={isPinned(appType, item.id)}>
          <svg width="14" height="14" viewBox="0 0 24 24" fill={isPinned(appType, item.id) ? 'currentColor' : 'none'} stroke="currentColor" stroke-width="1.8" stroke-linejoin="round"><path d="M12 4l2.4 5 5.6.8-4 3.9 1 5.5-5-2.7-5 2.7 1-5.5-4-3.9 5.6-.8z" /></svg>
        </button>
      </span>
    {/if}
  </div>
{/snippet}

{#snippet list(rows: Item[])}
  <div class="ctx-gitems">
    {#each rows as item (item.id)}{@render row(item)}{/each}
  </div>
{/snippet}

<aside class="ctx" bind:this={asideEl}>
  <div class="ctx-head">
    <div class="ctx-ht">
      <div class="ctx-crumb">{crumb}</div>
      <h1 class="ctx-title">{getActivePanel()?.title ?? contextLabel(appType)}</h1>
    </div>
    <div class="ctx-actions">
      {#if newLabel}
        <button class="ctx-primary" onclick={newItem}>
          <svg width="13" height="13" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.8"><path d="M8 3v10M3 8h10" /></svg> {newLabel}
        </button>
      {/if}
    </div>
  </div>

  <div class="ctx-tools">
    <input class="ctx-search" type="text" placeholder={`Filter ${contextLabel(appType).toLowerCase()}`} bind:value={filter} aria-label="Filter" />
    {#if !isTree}
      <select class="ctx-sel" bind:value={sortBy} onchange={saveView} aria-label="Sort">
        <option value="recent">Recent</option>
        <option value="name">Name</option>
      </select>
    {/if}
  </div>

  <div class="ctx-body">
    {#if pinnedShown.length > 0 && !error}
      <section class="ctx-group ctx-pinned">
        <button class="ctx-ghead" onclick={() => toggleGroup('__pinned')} aria-expanded={isOpen('__pinned')}>
          <svg class="ctx-chev" class:open={isOpen('__pinned')} width="9" height="9" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="2.2"><path d="M6 3l5 5-5 5" /></svg>
          <span class="ctx-gname">Pinned</span>
          <span class="ctx-gcount">{pinnedShown.length}</span>
        </button>
        {#if isOpen('__pinned')}{@render list(sorted(pinnedShown))}{/if}
      </section>
    {/if}
    {#if loading && items.length === 0}
      <!-- the header bar shows loading -->
    {:else if error}
      <div class="ctx-note err">{error}</div>
    {:else if shown.length === 0}
      <div class="ctx-note">{filter ? 'Nothing matches.' : 'Nothing here yet.'}</div>
    {:else if isTree}
      {#each shown as item (item.id)}{@render row(item)}{/each}
    {:else if groups}
      {#each groups as g (g.ns)}
        <section class="ctx-group">
          <button class="ctx-ghead" onclick={() => toggleGroup(g.ns)} aria-expanded={isOpen(g.ns)}>
            <svg class="ctx-chev" class:open={isOpen(g.ns)} width="9" height="9" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="2.2"><path d="M6 3l5 5-5 5" /></svg>
            <span class="ctx-gname">{g.ns || 'No project'}</span>
            <span class="ctx-gcount">{g.items.length}</span>
          </button>
          {#if isOpen(g.ns)}{@render list(sorted(g.items))}{/if}
        </section>
      {/each}
    {:else}
      {@render list(sorted(shown))}
    {/if}
  </div>

  {#if spec && !spec.focuses && items.length > 0}
    <div class="ctx-foot">Selecting opens {contextLabel(appType).toLowerCase()} in this app; jumping straight to an item isn’t wired up for it yet.</div>
  {/if}
</aside>

<style>
  /* Items, as in the shell design: header strip, toolbar, groups of rows. */
  .ctx { flex: 1 1 auto; display: flex; flex-direction: column; min-height: 0; overflow: hidden; background: var(--bg-page); }

  .ctx-head { flex: 0 0 auto; display: flex; align-items: center; gap: 8px; padding: 10px 12px; background: var(--bg-surface); border-bottom: 1px solid var(--border); }
  .ctx-ht { flex: 1 1 auto; min-width: 0; }
  .ctx-crumb { font: 400 12px var(--font-mono); color: var(--text-3); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .ctx-title { margin: 2px 0 0; font: 400 22px var(--font-serif); color: var(--text-1); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .ctx-actions { display: flex; align-items: center; gap: 6px; }
  .ctx-primary { display: inline-flex; align-items: center; gap: 6px; height: 32px; padding: 0 12px; border-radius: 8px; border: 0; background: var(--green, #1a7158); color: #fff; font: 500 13px var(--font-sans); cursor: pointer; white-space: nowrap; }
  .ctx-icon { width: 30px; height: 30px; border-radius: 8px; border: 1px solid var(--border); background: var(--bg-surface); color: var(--text-3); display: grid; place-items: center; cursor: pointer; }
  .ctx-icon:hover:not(:disabled) { color: var(--spec); border-color: var(--spec); }
  .ctx-icon:disabled { opacity: 0.4; cursor: default; }

  .ctx-tools { flex: 0 0 auto; display: flex; flex-wrap: wrap; align-items: center; gap: 6px; padding: 10px 12px; background: var(--bg-surface); border-bottom: 1px solid var(--border); }
  .ctx-search { flex: 1 1 120px; width: auto; min-width: 100px; height: 30px; border: 1px solid var(--border); border-radius: 8px; padding: 0 10px; background: var(--bg-surface); font: 400 13px var(--font-sans); color: var(--text-1); outline: none; }
  .ctx-search:focus { border-color: var(--spec); }
  .ctx-sel { flex: 0 0 auto; width: auto; height: 30px; border: 1px solid var(--border); border-radius: 8px; background: var(--bg-surface); font: 400 12.5px var(--font-sans); padding: 0 6px; color: var(--text-2); }

  .ctx-body { flex: 1; min-height: 0; overflow-y: auto; padding: 14px; }
  .ctx-note { padding: 22px; border: 1px dashed var(--border-strong, #d9d4c6); border-radius: 12px; text-align: center; font-size: 13px; color: var(--text-3); }
  .ctx-note.err { color: var(--err, #d15858); }

  /* Groups: serif title and a count. */
  .ctx-group + .ctx-group { margin-top: 10px; }
  .ctx-ghead { display: flex; align-items: center; gap: 8px; width: 100%; margin: 0 0 6px; padding: 0; border: 0; background: none; cursor: pointer; text-align: left; }
  .ctx-chev { flex: 0 0 auto; color: var(--text-4); transition: transform 0.12s ease; }
  .ctx-chev.open { transform: rotate(90deg); }
  .ctx-gname { font: 400 15px var(--font-serif); color: var(--text-1); }
  .ctx-gcount { font: 500 10.5px var(--font-mono); color: var(--text-4); background: var(--bg-subtle); border-radius: 4px; padding: 1px 6px; }
  .ctx-gitems { display: flex; flex-direction: column; gap: 6px; }
  .ctx-body > .ctx-gitems + .ctx-group, .ctx-pinned + * { margin-top: 10px; }

  /* A row: icon · name and project · subline · status. */
  .ctx-row { position: relative; display: flex; align-items: center; border: 1px solid var(--border); border-radius: 10px; background: var(--bg-surface); transition: border-color 0.12s; }
  .ctx-row:hover { border-color: var(--border-strong, #d9d4c6); }
  .ctx-row.active { border-color: var(--spec); box-shadow: 0 0 0 1px var(--spec); }
  .ctx-pick { flex: 1; min-width: 0; display: flex; align-items: center; gap: 12px; padding: 10px 12px; border: 0; background: transparent; cursor: pointer; text-align: left; font: inherit; color: inherit; }
  .ctx-icon-sq { flex: 0 0 32px; width: 32px; height: 32px; border-radius: 8px; display: inline-flex; align-items: center; justify-content: center; font: 600 11.5px var(--font-sans); color: #fff; }
  .ctx-badge { flex: 0 0 auto; width: 32px; height: 32px; border-radius: 8px; display: grid; place-items: center; font-size: 10px; font-weight: 600; }
  .ctx-text { flex: 1 1 auto; min-width: 0; display: flex; flex-direction: column; gap: 3px; }
  .ctx-label { display: flex; gap: 6px; align-items: center; flex-wrap: wrap; min-width: 0; }
  .ctx-name { font-weight: 600; font-size: 13.5px; color: var(--text-1); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; max-width: 100%; }
  .ctx-ns { font: 400 11px var(--font-mono); color: var(--spec); border: 1px solid #b8a6ea; border-radius: 999px; padding: 0 7px; }
  .ctx-sub { font-size: 12.5px; color: var(--text-3); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .ctx-row.pending .ctx-name { font-style: italic; color: var(--text-3); font-weight: 500; }
  .ctx-st { flex: 0 0 auto; font: 500 10.5px var(--font-mono); letter-spacing: 0.06em; text-transform: uppercase; color: var(--text-3); white-space: nowrap; }
  .ctx-st.ok { color: var(--green, #1a7158); }
  .ctx-st.run { color: #1f4fb8; }
  .ctx-st.bad { color: var(--err, #d15858); }
  .ctx-st.idle { color: #8a5112; }

  /* Hover actions, on the right over the status. */
  .ctx-acts { display: none; position: absolute; right: 8px; top: 50%; transform: translateY(-50%); gap: 2px; background: var(--bg-surface); padding-left: 8px; }
  .ctx-row:hover .ctx-acts, .ctx-row:focus-within .ctx-acts { display: flex; }
  .ctx-row:hover .ctx-st, .ctx-row:focus-within .ctx-st { visibility: hidden; }
  .ctx-rb { width: 28px; height: 28px; border-radius: 7px; border: 1px solid transparent; background: transparent; display: inline-flex; align-items: center; justify-content: center; color: var(--text-3); padding: 0; cursor: pointer; }
  .ctx-rb:hover { background: var(--bg-subtle); border-color: var(--border); }
  .ctx-rb.on { color: #c97c2e; }
  .ctx-row.active .ctx-acts { background: var(--bg-surface); }

  /* Trees (Runtimes): compact rows with a twisty. */
  .ctx-twist { flex: none; width: 16px; height: 22px; display: grid; place-items: center; background: none; border: 0; padding: 0 0 0 6px; color: var(--text-4); cursor: pointer; }

  .ctx-foot { flex: 0 0 auto; padding: 8px 12px; border-top: 1px solid var(--border); font-size: 11px; line-height: 1.35; color: var(--text-4); }
</style>
