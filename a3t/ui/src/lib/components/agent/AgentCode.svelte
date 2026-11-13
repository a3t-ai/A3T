<script lang="ts">
  /// Build › Code — what the agent runs: its template's workflow, as the design
  /// (graph) or as code, read here. The template is code shared by every agent
  /// made from it, so it is changed in the Studio where it is
  /// saved, versioned and checked (Edit in Studio).
  import FlowEditorPane from '$lib/components/editor/FlowEditorPane.svelte';
  import type { AgentRec } from './agentData';

  let { agent, templates, oncode }: {
    agent: AgentRec;
    templates: { id: string; name: string; version: string; spec_yaml: string }[];
    oncode: () => void;
  } = $props();

  const tpl = $derived(templates.find((t) => t.id === agent.template_id) ?? null);
  // A new key per template (and version) so the pane loads it afresh.
  const key = $derived(tpl ? `${tpl.id}@${tpl.version}` : '');
</script>

<section class="agt code">
  {#if !templates.length}
    <p class="dim">Reading its template…</p>
  {:else if !tpl}
    <p class="dim">Its template <span class="mono">{agent.template_id || '—'}</span> is not in reach of this project.</p>
  {:else if !tpl.spec_yaml?.trim()}
    <p class="dim">Its template has no workflow yet.</p>
  {:else}
    {#key key}
      <div class="pane">
        <FlowEditorPane part="main" initialYaml={tpl.spec_yaml} readOnly>
          {#snippet toolbar()}<span class="grow"></span><button class="link edit" onclick={oncode}>Edit in Studio →</button>{/snippet}
        </FlowEditorPane>
      </div>
    {/key}
  {/if}
</section>

<style>
  .code { display: flex; flex-direction: column; gap: 10px; max-width: none; }
  .pane { height: max(480px, calc(100vh - 300px)); border: 1px solid var(--border); border-radius: 10px; overflow: hidden; display: flex; flex-direction: column; }
  .pane > :global(*) { flex: 1; min-height: 0; }
  .grow { flex: 1 1 auto; }
  .edit { margin-right: 10px; white-space: nowrap; }
  .mono { font-family: var(--font-mono); font-size: 0.92em; }
</style>
