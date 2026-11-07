<script lang="ts">
  /// Ctrl K: one box for apps and commands, centred over everything. With no
  /// match, Enter asks the question in Chat.
  import { navigateTo } from '$lib/stores/panels.svelte.ts';
  import { menuGroups as filteredMenuGroups } from '$lib/stores/auth';
  import { trackOpen, setPendingMessage } from '$lib/stores/apps.svelte.ts';
  import {
    isPaletteOpen, closePalette, togglePin, toggleFace, showMid, setChatSession, isPinned,
  } from '$lib/stores/layout.svelte.ts';
  import { appIcon } from './appIcons';

  interface Entry { id: string; label: string; hint: string; kind: 'App' | 'Command'; icon?: string; run: () => void; }

  let q = $state('');
  let at = $state(0);
  let inputEl = $state<HTMLInputElement>();

  const apps = $derived<Entry[]>($filteredMenuGroups.flatMap((g: any) => g.items.map((it: any) => ({
    id: `app:${it.type}`, label: it.label, hint: g.label ?? '', kind: 'App' as const, icon: it.icon,
    run: () => { trackOpen(it.type, it.label); navigateTo(it.type, it.label); },
  }))));

  const commands = $derived<Entry[]>([
    { id: 'cmd:menu', label: `${isPinned('menu') ? 'Unpin' : 'Pin'} the menu`, hint: 'Ctrl \\', kind: 'Command', run: () => togglePin('menu') },
    { id: 'cmd:face', label: 'Switch Items ↔ Chat', hint: 'Ctrl J', kind: 'Command', run: () => toggleFace() },
    { id: 'cmd:details', label: `${isPinned('details') ? 'Unpin' : 'Pin'} Details`, hint: 'Ctrl .', kind: 'Command', run: () => togglePin('details') },
    { id: 'cmd:mid', label: `${isPinned('mid') ? 'Unpin' : 'Pin'} Items | Chat`, hint: '', kind: 'Command', run: () => togglePin('mid') },
    { id: 'cmd:newchat', label: 'New conversation', hint: 'Chat', kind: 'Command', run: () => { setChatSession(''); showMid('chat'); } },
  ]);

  const results = $derived.by<Entry[]>(() => {
    const s = q.trim().toLowerCase();
    const all = [...apps, ...commands];
    if (!s) return all;
    return all.filter((e) => e.label.toLowerCase().includes(s) || e.hint.toLowerCase().includes(s));
  });

  $effect(() => { void q; at = 0; });
  $effect(() => {
    if (!isPaletteOpen()) return;
    q = '';
    queueMicrotask(() => inputEl?.focus());
  });

  function run(e: Entry) { closePalette(); e.run(); }

  /// No match: hand the words to Chat as its next message.
  function ask() {
    const text = q.trim();
    if (!text) return;
    closePalette();
    setPendingMessage(text);
    setChatSession(`session-${Date.now()}-${Math.random().toString(36).slice(2, 8)}`);
    showMid('chat');
  }

  function key(e: KeyboardEvent) {
    if (e.key === 'ArrowDown') { e.preventDefault(); at = Math.min(results.length - 1, at + 1); }
    else if (e.key === 'ArrowUp') { e.preventDefault(); at = Math.max(0, at - 1); }
    else if (e.key === 'Enter') { e.preventDefault(); results[at] ? run(results[at]) : ask(); }
  }
</script>

{#if isPaletteOpen()}
  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="scrim" onclick={(e) => { if (e.target === e.currentTarget) closePalette(); }}>
    <div class="palette" role="dialog" aria-label="Search and commands">
      <input bind:this={inputEl} bind:value={q} onkeydown={key} placeholder="Type an app or a command — or ask anything" aria-label="Search" />
      <div class="results" role="listbox">
        {#each results as e, i (e.id)}
          <button class="row" class:on={i === at} role="option" aria-selected={i === at}
            onmouseenter={() => (at = i)} onclick={() => run(e)}>
            <span class="ico">
              {#if e.icon}
                <svg width="16" height="16" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5">{@html appIcon(e.icon)}</svg>
              {:else}
                <svg width="14" height="14" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round"><path d="M5 4l-3 4 3 4M11 4l3 4-3 4" /></svg>
              {/if}
            </span>
            <span class="label">{e.label}</span>
            <span class="kind">{e.kind}</span>
            {#if e.hint}<span class="hint">{e.hint}</span>{/if}
          </button>
        {:else}
          <button class="row on" onclick={ask}>
            <span class="ico">
              <svg width="14" height="14" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5"><path d="M14 2L7 9M14 2l-5 12-2-5-5-2z" /></svg>
            </span>
            <span class="label">Ask in Chat: “{q.trim()}”</span>
            <span class="hint">Enter</span>
          </button>
        {/each}
      </div>
      <div class="foot"><span class="kbd">↑ ↓</span> move · <span class="kbd">Enter</span> open · <span class="kbd">Esc</span> close</div>
    </div>
  </div>
{/if}

<style>
  .scrim { position: fixed; inset: 0; background: rgba(26, 26, 26, 0.18); z-index: 200; display: flex; justify-content: center; align-items: flex-start; padding-top: 12vh; }
  .palette { width: min(600px, 92vw); background: var(--bg-surface); border: 1px solid var(--border); border-radius: 14px; box-shadow: var(--shadow-lg, 0 12px 30px rgba(30, 25, 15, 0.12)); padding: 10px; display: flex; flex-direction: column; gap: 6px; }
  input { width: 100%; box-sizing: border-box; height: 44px; padding: 0 12px; border: 1px solid var(--border); border-radius: 10px; background: var(--bg-page); color: var(--text-1); font: 400 15px var(--font-sans); outline: none; }
  input:focus { border-color: var(--spec); }
  .results { max-height: 50vh; overflow-y: auto; display: flex; flex-direction: column; gap: 1px; }
  .row { display: flex; align-items: center; gap: 10px; width: 100%; padding: 8px 10px; border: 0; border-radius: 8px; background: none; color: var(--text-1); font: 400 13.5px var(--font-sans); text-align: left; cursor: pointer; }
  .row.on { background: var(--spec-bg); }
  .ico { width: 22px; display: inline-flex; justify-content: center; color: var(--text-3); flex: none; }
  .label { flex: 1 1 auto; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .kind { font: 500 10.5px var(--font-mono); letter-spacing: 0.05em; text-transform: uppercase; color: var(--text-4); }
  .hint { font: 500 11px var(--font-mono); color: var(--text-3); border: 1px solid var(--border); border-radius: 4px; padding: 0 5px; background: var(--bg-subtle); }
  .foot { font: 500 11px var(--font-mono); color: var(--text-4); padding: 2px 4px; }
  .kbd { border: 1px solid var(--border); border-radius: 4px; padding: 0 5px; background: var(--bg-subtle); }
</style>
