<script lang="ts">
  /// Build › Code — what the agent runs: its template's workflow, as the design
  /// (graph) or as code, read here. The template is code shared by every agent
  /// made from it, so it is changed in the Studio (Edit in Studio), where it is
  /// saved, versioned and checked. The agent's own values are in Model & tools.
  import FlowEditorPane from '$lib/components/editor/FlowEditorPane.svelte';
  import type { AgentRec } from './agentData';

  let { agent, templates, oncode, ongo }: {
    agent: AgentRec;
    templates: { id: string; name: string; version: string; spec_yaml: string }[];
    oncode: () => void;
    ongo: (tab: string) => void;
  } = $props();

  const tpl = $derived(templates.find((t) => t.id === agent.template_id) ?? null);
  // A new key per template (and version) so the pane loads it afresh.
  const key = $derived(tpl ? `${tpl.id}@${tpl.version}` : '');
</script>

<section class="agt code">
  <div class="card-head head"><h3>Code</h3>
    {#if tpl}<span>its template <span class="mono">{tpl.name}</span> v{tpl.version}: the same for every agent made from it</span>{/if}
    <span class="grow"></span>
    <button class="link" onclick={() => ongo('tools')}>Its own values →</button>
    {#if tpl}<button class="btn2" onclick={oncode}>Edit in Studio</button>{/if}
  </div>
  {#if !templates.length}
    <p class="dim">Reading its template…</p>
  {:else if !tpl}
    <p class="dim">Its template <span class="mono">{agent.template_id || '—'}</span> is not in reach of this project.</p>
  {:else if !tpl.spec_yaml?.trim()}
    <p class="dim">Its template has no workflow yet.</p>
  {:else}
    {#key key}
      <div class="pane">
        <FlowEditorPane part="main" initialYaml={tpl.spec_yaml} readOnly />
      </div>
    {/key}
  {/if}
</section>

<style>
  .code { display: flex; flex-direction: column; gap: 10px; }
  .head { display: flex; flex-wrap: wrap; align-items: baseline; gap: 8px; }
  .grow { flex: 1 1 auto; }
  .pane { height: max(480px, calc(100vh - 300px)); border: 1px solid var(--border); border-radius: 10px; overflow: hidden; display: flex; flex-direction: column; }
  .pane > :global(*) { flex: 1; min-height: 0; }
  .mono { font-family: var(--font-mono); font-size: 0.92em; }
</style>
