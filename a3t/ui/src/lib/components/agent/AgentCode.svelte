<script lang="ts">
  /// Build › Code — what the agent runs: its template's workflow, as the design
  /// (graph) or as code, read here. The template is code shared by every agent
  /// made from it, so it is changed in the Studio where it is
  /// saved, versioned and checked (Edit in Studio).
  import FlowEditorPane from '$lib/components/editor/FlowEditorPane.svelte';
  import { getYaml } from '$lib/studio-core/adapters/flows';
  import { readTemplate } from '$lib/components/templates/templateSpec';
  import type { AgentRec } from './agentData';

  let { agent, templates, oncode }: {
    agent: AgentRec;
    templates: { id: string; name: string; version: string; spec_yaml: string }[];
    oncode: () => void;
  } = $props();

  const tpl = $derived(templates.find((t) => t.id === agent.template_id) ?? null);
  /// The template's flow: its spec's document, else its name (tmpl_<project>_<function>).
  const flow = $derived.by(() => {
    if (!tpl) return null;
    const f = readTemplate(tpl.spec_yaml);
    const name = tpl.name || tpl.id;
    const ns = f.namespace || (/^tmpl_([a-z0-9]+)_/.exec(name)?.[1] ?? '');
    return ns ? { ns, name } : null;
  });
  // What runs is the flow's live version in FlowR; the catalogue's copy can lag.
  let live = $state<{ yaml: string; from: 'flowr' | 'catalogue' } | null>(null);
  let loadErr = $state('');
  $effect(() => {
    const t = tpl, fl = flow;
    live = null; loadErr = '';
    if (!t) return;
    if (!fl) { live = t.spec_yaml?.trim() ? { yaml: t.spec_yaml, from: 'catalogue' } : null; return; }
    getYaml(fl.ns, fl.name).then((r) => {
      if (tpl !== t) return;
      if (r.ok && r.data.trim()) live = { yaml: r.data, from: 'flowr' };
      else if (t.spec_yaml?.trim()) live = { yaml: t.spec_yaml, from: 'catalogue' };
      else loadErr = r.ok ? '' : r.error ?? '';
    });
  });
  const liveVersion = $derived(live ? readTemplate(live.yaml).version ?? '' : '');
  // A new key per template (and version) so the pane loads it afresh.
  const key = $derived(tpl && live ? `${tpl.id}@${liveVersion}@${live.from}` : '');
</script>

<section class="agt code">
  {#if !templates.length}
    <p class="dim">Reading its template…</p>
  {:else if !tpl}
    <p class="dim">Its template <span class="mono">{agent.template_id || '—'}</span> is not in reach of this project.</p>
  {:else if !live}
    <p class="dim">{loadErr ? `Its template's flow could not be read: ${loadErr}` : flow ? 'Reading its template\'s flow…' : 'Its template has no workflow yet.'}</p>
  {:else}
    {#key key}
      <div class="pane">
        <FlowEditorPane part="main" initialYaml={live.yaml} readOnly>
          {#snippet toolbar()}<span class="grow"></span><span class="ver mono">{flow ? `${flow.ns}/${flow.name}` : tpl.name}{liveVersion ? ` v${liveVersion}` : ''}{live.from === 'flowr' ? ' · live' : ' · catalogue copy'}</span><button class="link edit" onclick={oncode}>Edit in Studio →</button>{/snippet}
        </FlowEditorPane>
      </div>
    {/key}
  {/if}
</section>

<style>
  .code { display: flex; flex-direction: column; gap: 10px; }
  .pane { height: max(480px, calc(100vh - 300px)); border: 1px solid var(--border); border-radius: 10px; overflow: hidden; display: flex; flex-direction: column; }
  .pane > :global(*) { flex: 1; min-height: 0; }
  .grow { flex: 1 1 auto; }
  .edit { margin-right: 10px; white-space: nowrap; }
  .ver { margin-right: 12px; font-size: 11.5px; color: var(--text-3); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .mono { font-family: var(--font-mono); font-size: 0.92em; }
</style>
