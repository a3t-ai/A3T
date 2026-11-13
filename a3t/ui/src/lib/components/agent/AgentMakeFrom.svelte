<script lang="ts">
  /// Make a new agent from this one: a child that inherits from it, a fork (keeps
  /// its link to the parent), or a clone (independent).
  import { agentName } from './agentData';
  let { kind, name, projects, current, onmake, oncancel }: {
    kind: 'clone' | 'fork' | 'inherit';
    /// The agent it is made from.
    name: string;
    projects: string[];
    current: string;
    /// Make it; an error, or null.
    onmake: (name: string, project: string) => Promise<string | null>;
    oncancel: () => void;
  } = $props();

  // svelte-ignore state_referenced_locally — a starting value, edited from here
  let newName = $state(`${name.replace(/^agent_[a-z0-9]+_/, '')}_${kind === 'fork' ? 'fork' : kind === 'inherit' ? 'child' : 'copy'}`);
  const TITLE = { clone: 'Clone', fork: 'Fork', inherit: 'Inherit from' } as const;
  // svelte-ignore state_referenced_locally
  let project = $state(current);
  let busy = $state(false);
  let err = $state('');
  const full = $derived(agentName(project, newName));
  const valid = $derived(!!full && full.length <= 96);

  async function make() {
    busy = true; err = '';
    err = (await onmake(newName, project)) ?? '';
    busy = false;
  }
</script>

<!-- Inline, where it was asked for (Lineage), like a template's copy form. -->
<div class="card mk" role="group" aria-label="{TITLE[kind]} {name}">
  <div class="card-head"><h3>{TITLE[kind]} <span class="mono">{name}</span></h3></div>
  {#if kind === 'inherit'}
    <p>A new agent made from <span class="mono">{name}</span>, <b>listed as its child</b> in Lineage: its template, model, instructions and tools to start with.</p>
  {:else if kind === 'clone'}
    <p>A new agent with the same template, model, instructions and tools — <b>on its own from then on</b>. Changes to either never reach the other.</p>
  {:else}
    <p>A new agent that <b>keeps its link</b> to <span class="mono">{name}</span>: it is listed as its fork in Lineage. Nothing it learns goes back.</p>
  {/if}
  <div class="fields">
    <label>Name<input type="text" bind:value={newName} spellcheck="false" /></label>
    <label>In project
      <select bind:value={project}>{#each projects as p (p)}<option value={p}>{p}</option>{/each}</select>
    </label>
  </div>
  {#if newName && !valid}<span class="err">Use letters or digits.</span>{:else if full}<span class="dim mono">It will be named {full}</span>{/if}
  <p class="dim">It starts with no memory, its own memory space, its own endpoint and its own key; it is not deployed. To carry what {name} remembers, branch it and promote the branch.</p>
  {#if err}<p class="err">{err}</p>{/if}
  <div class="acts">
    <button class="btn2 primary" disabled={!valid || !project || busy} onclick={make}>{busy ? 'Making…' : kind === 'fork' ? 'Fork it' : kind === 'inherit' ? 'Make it' : 'Clone it'}</button>
    <button class="btn2" disabled={busy} onclick={oncancel}>Cancel</button>
  </div>
</div>

<style>
  .mk { gap: 10px; border-color: var(--spec); }
  .mk h3 { margin: 0; }
  p { margin: 0; font-size: 13px; line-height: 1.5; color: var(--text-2); }
  .dim { font-size: 12.5px; color: var(--text-3); }
  .err { font-size: 12.5px; color: var(--err, #b42318); }
  .mono { font-family: var(--font-mono); font-size: 0.92em; }
  .fields { display: grid; grid-template-columns: 1fr 1fr; gap: 10px; }
  @container details (max-width: 560px) { .fields { grid-template-columns: 1fr; } }
  label { display: flex; flex-direction: column; gap: 5px; font-size: 12.5px; color: var(--text-2); text-transform: none; letter-spacing: 0; font-weight: 500; }
  .acts { display: flex; gap: 8px; }
</style>
