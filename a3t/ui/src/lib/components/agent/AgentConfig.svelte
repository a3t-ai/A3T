<script lang="ts">
  /// Config — the agent's values as YAML (what it fills into its template), and
  /// the template's code they apply to, read-only. Values: edit here, or ask the
  /// Chat. Logic: change the code in the Studio.
  import { untrack } from 'svelte';
  import YamlEditor from '$lib/components/editor/YamlEditor.svelte';
  import { toYaml, fromYaml, completer, type Bindings, type Issue } from './agentValues';
  import type { AgentRec, TemplateRec } from './agentData';

  type Opt = { value: string; label: string };
  let {
    agent, templates, bindings = $bindable(), dirty, saving, saveError, onsave, ondiscard, oncode,
    llmResources, mcpResources, flowOptions,
  }: {
    agent: AgentRec;
    templates: TemplateRec[];
    bindings: Bindings;
    dirty: boolean;
    saving: boolean;
    saveError: string;
    onsave: () => void;
    ondiscard: () => void;
    /// Open the template's code in the Studio.
    oncode: () => void;
    llmResources: Opt[];
    mcpResources: Opt[];
    flowOptions: Opt[];
  } = $props();

  const tpl = $derived(templates.find((t) => t.id === agent.template_id));
  const header = $derived(`${agent.name} — values for ${tpl ? `${tpl.name}@${tpl.version}` : 'its template'}`);

  // The text follows the bindings when they change from outside (a save, a
  // discard, the Chat's edit), never while it is being typed.
  let text = $state('');
  let issues = $state<Issue[]>([]);
  let fromText = '';
  $effect(() => {
    const now = JSON.stringify($state.snapshot(bindings));
    const h = header;
    untrack(() => {
      if (now === fromText) return;
      text = toYaml(bindings, h);
      fromText = now;
      issues = [];
    });
  });
  function edited(t: string) {
    text = t;
    const r = fromYaml(t, $state.snapshot(bindings) as Bindings);
    issues = r.issues;
    if (r.values) { fromText = JSON.stringify(r.values); bindings = r.values; }
  }
  const complete = $derived(completer({ model: llmResources.map((o) => o.value), tools: mcpResources.map((o) => o.value), flows: flowOptions.map((o) => o.value) }));
  const errors = $derived(issues.filter((i) => i.level === 'error'));
  let showCode = $state(true);
</script>

<section class="agt">
  {#if dirty || saveError || errors.length}
    <div class="dirty" class:bad={!!saveError || errors.length > 0}>
      <span>{errors.length ? `Line ${errors[0].line}: ${errors[0].message}` : saveError ? `Not saved: ${saveError}` : 'Unsaved changes'}</span>
      <span class="acts"><button class="btn2 quiet" disabled={saving} onclick={ondiscard}>Discard</button>
        <button class="btn2 primary" disabled={saving || errors.length > 0 || !dirty} onclick={onsave}>{saving ? 'Saving…' : 'Save'}</button></span>
    </div>
  {/if}

  <div class="card flush">
    <div class="card-head pad"><h3>Values</h3><span>this agent's — <kbd>Ctrl</kbd>+<kbd>Space</kbd> lists keys and choices</span></div>
    <div class="ed"><YamlEditor value={text} onchange={edited} {issues} {complete} /></div>
  </div>

  <div class="card flush">
    <div class="card-head pad">
      <h3>Code</h3>
      <span class="mono">{tpl ? `${tpl.name}@${tpl.version}` : 'no template'}</span><span>read-only — the values above fill its <span class="mono">a3t:agent</span> step</span>
      <span class="grow"></span>
      <button class="link" onclick={() => (showCode = !showCode)}>{showCode ? 'Hide' : 'Show'}</button>
      <button class="btn2" onclick={oncode}>Change the code in Studio</button>
    </div>
    {#if showCode}
      {#if tpl?.spec_yaml}<div class="ed code-ro"><YamlEditor value={tpl.spec_yaml} readOnly /></div>
      {:else}<p class="dim pad">Its template's code is not available.</p>{/if}
    {/if}
  </div>
  <p class="dim">Changing what it connects to, its steps or its hooks is a code change: in the Studio, on your copy of the template — then move this agent to it in Settings.</p>
</section>

<style>
  :global(.agt) .card.flush { padding: 0; gap: 0; overflow: hidden; }
  .pad { padding: 12px 16px; border-bottom: 1px solid var(--border); }
  p.pad { padding: 12px 16px; }
  .ed { height: 330px; }
  .code-ro { height: 380px; }
  kbd { font: 11px var(--font-mono); padding: 0 4px; border: 1px solid var(--border); border-radius: 3px; background: var(--bg-subtle); }
</style>
