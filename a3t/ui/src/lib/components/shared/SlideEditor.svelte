<script lang="ts">
  // The full-screen editor shell used by the Template editor — extracted so
  // Canvas (and others) render the IDENTICAL layout: a slide-in panel with a
  // left section rail, a top-right action bar (open/save/duplicate/close), and a
  // content area. Provide `sections` + `active`/`onselect`; render the active
  // section as children. Styling is copied verbatim from TemplatesList.
  import type { Snippet } from 'svelte';
  import WizardSteps, { type WizardStep } from './WizardSteps.svelte';
  import WizardNav from './WizardNav.svelte';
  import { detailsPane, fitToPane } from '$lib/shell/detailsPane';
  import { setDetailActions } from '$lib/stores/detailActions.svelte.ts';

  let {
    open = false,
    icon = 'NEW',
    color = 'var(--spec, #c47028)',
    name = '',
    meta = '',
    sections = [],
    active = '',
    onselect,
    onsave,
    onduplicate,
    onopen,
    onclose,
    saving = false,
    saveLabel = 'Save',
    steps,
    wizard = false,
    children,
  }: {
    open: boolean;
    icon?: string;
    color?: string;
    name?: string;
    meta?: string;
    sections: { id: string; label: string; icon: string }[];
    active: string;
    onselect: (id: string) => void;
    onsave?: () => void;
    onduplicate?: () => void;
    onopen?: () => void;
    onclose: () => void;
    saving?: boolean;
    saveLabel?: string;
    /// Numbered steps (instead of `sections`) with Back / Next / save at the foot;
    /// `wizard`: a new item, filled in order.
    steps?: WizardStep[];
    wizard?: boolean;
    children: Snippet;
  } = $props();

  let closing = $state(false);
  /// Inside the shell the editor fills the Details pane, and its actions go in
  /// the Details stack bar (one bar, not two).
  const pane = detailsPane();
  $effect(() => {
    if (!pane || !open) return;
    return setDetailActions({
      onopenineditor: onopen, onsave, onclose: close,
      extra: onduplicate ? [{ label: 'Duplicate', run: onduplicate }] : undefined,
    });
  });
  function close() {
    closing = true;
    setTimeout(() => { closing = false; onclose(); }, 200);
  }
</script>

{#if open}
  {#if !pane}<div class="slide-overlay" class:closing onclick={close} role="presentation"></div>{/if}
  <div class="slide-panel" class:closing class:in-pane={!!pane} use:fitToPane={pane}>
    <div class="sp-layout">
      <!-- LEFT: section rail -->
      <div class="sp-tree-panel">
        <div class="sp-tree-head">
          <div class="sp-tree-icon" style="background: {color}">{icon}</div>
          <div class="sp-tree-info">
            <div class="sp-tree-name">{name || 'Untitled'}</div>
            <div class="sp-tree-meta">{meta}</div>
          </div>
        </div>
        {#if steps}
          <WizardSteps {steps} {active} {wizard} {onselect} />
        {:else}
        <nav class="sp-tree">
          {#each sections as s}
            <button class="sp-tree-node" class:active={active === s.id} onclick={() => onselect(s.id)}>
              <svg width="14" height="14" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5">{@html s.icon}</svg>
              {s.label}
            </button>
          {/each}
        </nav>
        {/if}
      </div>

      <!-- MAIN: action bar + content -->
      <div class="sp-main">
        {#if !pane}
        <div class="sp-close-bar">
          {#if onopen}
            <div class="sp-btn-wrap">
              <button class="sp-action-btn" onclick={onopen}>
                <svg width="14" height="14" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5"><path d="M9 1h6v6M9 7L15 1M7 3H2a1 1 0 00-1 1v10a1 1 0 001 1h10a1 1 0 001-1V9"/></svg>
              </button>
              <span class="sp-tooltip">Open in editor</span>
            </div>
          {/if}
          {#if onsave}
            <div class="sp-btn-wrap">
              <button class="sp-action-btn save" onclick={onsave} disabled={saving}>
                <svg width="14" height="14" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5"><path d="M13 15H3a1 1 0 01-1-1V2a1 1 0 011-1h8l3 3v10a1 1 0 01-1 1z"/><path d="M11 15V9H5v6M5 1v4h5"/></svg>
              </button>
              <span class="sp-tooltip">{saving ? 'Saving…' : saveLabel}</span>
            </div>
          {/if}
          {#if onduplicate}
            <div class="sp-btn-wrap">
              <button class="sp-action-btn" onclick={onduplicate}>
                <svg width="14" height="14" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5"><rect x="4" y="4" width="10" height="10" rx="1"/><path d="M4 12H3a1 1 0 01-1-1V3a1 1 0 011-1h8a1 1 0 011 1v1"/></svg>
              </button>
              <span class="sp-tooltip">Duplicate</span>
            </div>
          {/if}
          <div class="sp-btn-wrap">
            <button class="sp-close" onclick={close}>
              <svg width="14" height="14" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="2"><path d="M12 4L4 12M4 4l8 8"/></svg>
            </button>
            <span class="sp-tooltip">Close</span>
          </div>
        </div>
        {/if}
        <div class="sp-content">{@render children()}</div>
        {#if steps}<WizardNav {steps} {active} {wizard} {onselect} final={onsave} finalLabel={saveLabel} finalBusy={saving} busyLabel={wizard ? 'Creating…' : 'Saving…'} />{/if}
      </div>
    </div>
  </div>
{/if}

<style>
  .slide-overlay { position: fixed; inset: 0; background: rgba(0,0,0,0.12); z-index: 100; animation: overlayIn 0.2s ease-out; }
  .slide-overlay.closing { animation: overlayOut 0.2s ease-in forwards; }
  @keyframes overlayIn { from { opacity: 0; } to { opacity: 1; } }
  @keyframes overlayOut { from { opacity: 1; } to { opacity: 0; } }

  .slide-panel {
    position: fixed; top: 42px; left: 110px; right: 6px; bottom: 6px;
    background: var(--bg-page); border: 1px solid var(--border); border-radius: 14px;
    z-index: 101; overflow: hidden;
    box-shadow: 0 8px 40px rgba(0,0,0,0.14), 0 2px 10px rgba(0,0,0,0.06), -4px 0 20px rgba(0,0,0,0.04);
    animation: panelIn 0.25s cubic-bezier(0.16, 1, 0.3, 1);
  }
  .slide-panel.closing { animation: panelOut 0.2s ease-in forwards; }
  @keyframes panelIn { from { transform: translateX(100%); } to { transform: translateX(0); } }
  @keyframes panelOut { from { transform: translateX(0); } to { transform: translateX(100%); } }

  .sp-layout { display: grid; grid-template-columns: 280px 1fr; height: 100%; }
  .sp-tree-panel { display: flex; flex-direction: column; background: var(--bg-surface); border-right: 1px solid var(--border); overflow-y: auto; }
  .sp-tree-head { display: flex; align-items: center; gap: 12px; min-height: 72px; padding: 12px 20px; box-sizing: border-box; border-bottom: 1px solid var(--border); }
  .sp-tree-icon { width: 32px; height: 32px; border-radius: 8px; display: grid; place-items: center; font-family: var(--font-mono); font-size: 11px; font-weight: 700; color: white; flex-shrink: 0; }
  .sp-tree-info { min-width: 0; }
  .sp-tree-name { font-family: var(--font-serif); font-size: 15px; font-weight: 500; line-height: 1.25; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .sp-tree-meta { font-size: 11px; color: var(--text-4); margin-top: 2px; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .sp-tree { display: flex; flex-direction: column; padding: 6px 0; flex: 1; }
  .sp-tree-node { display: flex; align-items: center; gap: 8px; padding: 7px 14px; border: none; background: none; cursor: pointer; font-size: 12px; color: var(--text-3); text-align: left; transition: all 0.1s; }
  .sp-tree-node:hover { color: var(--text-1); background: var(--bg-subtle); }
  .sp-tree-node.active { color: var(--text-1); font-weight: 550; background: var(--bg-subtle); box-shadow: inset 3px 0 0 var(--spec, #c47028); }
  .sp-tree-node svg { color: var(--text-4); flex-shrink: 0; }
  .sp-tree-node.active svg { color: var(--spec, #c47028); }

  .sp-main { overflow: hidden; position: relative; padding-top: 44px; display: flex; flex-direction: column; }
  .sp-close-bar { position: absolute; top: 0; left: 0; right: 0; height: 44px; display: flex; align-items: center; justify-content: flex-end; gap: 6px; padding: 0 12px; z-index: 10; background: var(--bg-page); border-bottom: 1px solid var(--border); }
  .sp-btn-wrap { position: relative; }
  .sp-tooltip { position: absolute; bottom: -30px; left: 50%; transform: translateX(-50%); padding: 4px 10px; background: var(--text-1); color: var(--bg-page); font-size: 11px; font-weight: 500; white-space: nowrap; border-radius: 5px; pointer-events: none; opacity: 0; transition: opacity 0.15s 0.3s; z-index: 20; }
  .sp-tooltip::before { content: ''; position: absolute; bottom: 100%; left: 50%; transform: translateX(-50%); border: 4px solid transparent; border-bottom-color: var(--text-1); }
  .sp-btn-wrap:hover .sp-tooltip { opacity: 1; }
  .sp-action-btn { width: 32px; height: 32px; border: 1px solid var(--border); border-radius: 50%; background: var(--bg-surface); color: var(--text-3); cursor: pointer; display: grid; place-items: center; transition: all 0.12s; }
  .sp-action-btn:hover { background: var(--bg-subtle); color: var(--text-1); border-color: var(--border-strong); }
  .sp-action-btn.save:hover { background: var(--ok-bg, #e3f4ef); color: var(--ok, #23987f); border-color: var(--ok); }
  .sp-action-btn:disabled { opacity: 0.5; cursor: default; }
  .sp-close { width: 32px; height: 32px; border: 1px solid var(--border); border-radius: 50%; background: var(--bg-surface); color: var(--text-3); cursor: pointer; display: grid; place-items: center; transition: all 0.12s; }
  .sp-close:hover { background: var(--text-1); color: var(--bg-page); border-color: var(--text-1); }
  .sp-content { flex: 1; min-height: 0; overflow: hidden; }
  /* In the shell's Details pane: flush with the pane, header and section tabs
     across the top, content below. */
  .slide-panel.in-pane { border: 0; border-radius: 0; box-shadow: none; z-index: 19; animation: none; }
  .slide-panel.in-pane.closing { animation: none; opacity: 0; }
  .slide-panel.in-pane .sp-layout { display: flex; flex-direction: column; }
  .slide-panel.in-pane .sp-tree-panel { flex: 0 0 auto; border-right: 0; overflow: visible; }
  .slide-panel.in-pane .sp-tree-head { min-height: 0; padding: 12px 18px; }
  .slide-panel.in-pane .sp-tree-name { font-size: 18px; font-weight: 400; }
  .slide-panel.in-pane .sp-tree { flex-direction: row; padding: 0 12px; overflow-x: auto; border-bottom: 1px solid var(--border); }
  .slide-panel.in-pane .sp-main { flex: 1 1 auto; min-height: 0; padding-top: 0; }
</style>
