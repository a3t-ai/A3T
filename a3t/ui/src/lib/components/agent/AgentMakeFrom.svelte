<script lang="ts">
  /// Make a new agent from this one — the only two ways an agent is made from
  /// another: a clone (independent) or a fork (keeps its link to the parent).
  let { kind, name, projects, current, onmake, oncancel }: {
    kind: 'clone' | 'fork';
    /// The agent it is made from.
    name: string;
    projects: string[];
    current: string;
    /// Make it; an error, or null.
    onmake: (name: string, project: string) => Promise<string | null>;
    oncancel: () => void;
  } = $props();

  // svelte-ignore state_referenced_locally — a starting value, edited from here
  let newName = $state(`${name}-${kind === 'fork' ? 'fork' : 'copy'}`);
  // svelte-ignore state_referenced_locally
  let project = $state(current);
  let busy = $state(false);
  let err = $state('');
  const valid = $derived(/^[a-z0-9][a-z0-9-]{1,62}$/.test(newName));

  async function make() {
    busy = true; err = '';
    err = (await onmake(newName, project)) ?? '';
    busy = false;
  }
</script>

<div class="mk-scrim" role="presentation" onclick={(e) => { if (e.target === e.currentTarget && !busy) oncancel(); }}>
  <div class="mk" role="dialog" aria-modal="true" aria-label="{kind === 'fork' ? 'Fork' : 'Clone'} {name}">
    <h3>{kind === 'fork' ? 'Fork' : 'Clone'} <span class="mono">{name}</span></h3>
    {#if kind === 'clone'}
      <p>A new agent with the same template, model, instructions and tools — <b>on its own from then on</b>. Changes to either never reach the other.</p>
    {:else}
      <p>A new agent that <b>keeps its link</b> to <span class="mono">{name}</span>: it is listed as its fork in Lineage. Nothing it learns goes back.</p>
    {/if}
    <label>Name<input type="text" bind:value={newName} spellcheck="false" /></label>
    {#if newName && !valid}<span class="err">Lower-case letters, digits and dashes.</span>{/if}
    <label>Project
      <select bind:value={project}>{#each projects as p (p)}<option value={p}>{p}</option>{/each}</select>
    </label>
    <p class="dim">It starts with no memory, its own memory space and its own endpoint; it is not deployed. To carry what {name} remembers, branch it in Releases and promote the branch.</p>
    {#if err}<p class="err">{err}</p>{/if}
    <div class="acts">
      <button class="btn-p" disabled={!valid || !project || busy} onclick={make}>{busy ? 'Making…' : kind === 'fork' ? 'Fork it' : 'Clone it'}</button>
      <button class="btn-s" disabled={busy} onclick={oncancel}>Cancel</button>
    </div>
  </div>
</div>

<style>
  .mk-scrim { position: absolute; inset: 0; z-index: 30; display: grid; place-items: start center; padding-top: 70px; background: color-mix(in srgb, var(--bg-page) 55%, transparent); }
  .mk { width: min(460px, calc(100% - 32px)); display: flex; flex-direction: column; gap: 10px; padding: 20px 22px; border: 1px solid var(--border); border-radius: 14px; background: var(--bg-surface); box-shadow: 0 12px 40px rgba(0,0,0,0.16); }
  h3 { margin: 0 0 2px; font: 400 19px var(--font-serif); color: var(--text-1); }
  p { margin: 0; font-size: 13px; line-height: 1.5; color: var(--text-2); }
  .dim { font-size: 12.5px; color: var(--text-3); }
  .err { font-size: 12.5px; color: var(--err, #b42318); }
  .mono { font-family: var(--font-mono); font-size: 0.92em; }
  label { display: flex; flex-direction: column; gap: 5px; font-size: 12.5px; color: var(--text-2); text-transform: none; letter-spacing: 0; font-weight: 500; }
  .acts { display: flex; gap: 8px; margin-top: 4px; }
  .btn-p, .btn-s { height: 32px; padding: 0 14px; border-radius: 8px; border: 1px solid var(--border); background: var(--bg-surface); font: 500 13px var(--font-sans); color: var(--text-1); cursor: pointer; }
  .btn-p { background: var(--green, #1a7158); border-color: var(--green, #1a7158); color: #fff; }
  .btn-p:disabled, .btn-s:disabled { opacity: 0.5; cursor: default; }
</style>
