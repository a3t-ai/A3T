<script lang="ts">
  /// Access — everything the agent can reach: who it acts as, which model sees
  /// its data, each tool it may call (destructive ones marked), what its code
  /// touches, what it remembers. Then its values as code; owners can edit them.
  import type { Snippet } from 'svelte';
  import YamlEditor from '$lib/components/editor/YamlEditor.svelte';
  import { isDestructive } from '$lib/services/mcpTools';
  import { factsOf, type AgentRec, type TemplateRec } from './agentData';
  import { toYaml, usesOf, type Bindings } from './agentValues';
  import type { AgentLive } from './agentLive.svelte';

  let { agent, templates, live, bindings, llmLabel, canEdit, editor, ongo }: {
    agent: AgentRec;
    templates: TemplateRec[];
    live: AgentLive;
    bindings: Bindings;
    llmLabel?: string;
    canEdit: boolean;
    /// The values editor (the page's), shown in Edit mode.
    editor: Snippet;
    ongo: (tab: string) => void;
  } = $props();

  const f = $derived(factsOf(agent, templates));
  const uses = $derived(usesOf(f.spec).filter((u) => u.kind !== 'model' && u.kind !== 'tool server'));
  const allow = $derived(f.tools);
  const tpl = $derived(templates.find((t) => t.id === agent.template_id));
  let editing = $state(false);
</script>

<section class="agt">
  <div class="card">
    <div class="card-head"><h3>Identity</h3><span>who its calls run as</span></div>
    <dl class="kv">
      <dt>Acts as</dt><dd class="mono">agent:{agent.id}</dd>
      <dt>On behalf of</dt><dd>the caller, when the call carries a user's token — then every tool call is decided for both.
        <span class="warnline">A call without a token runs with no user (SRV-16).</span></dd>
      <dt>Policies</dt><dd><button class="link" onclick={() => ongo('controls')}>What applies to it →</button></dd>
    </dl>
  </div>

  <div class="card">
    <div class="card-head"><h3>Model</h3><span>sees everything it is sent</span></div>
    <dl class="kv">
      <dt>Resource</dt><dd class="mono">{f.llm || '—'}</dd>
      <dt>Provider</dt><dd>{llmLabel ? llmLabel.replace(/^.*\(|\)$/g, '') : '—'}{llmLabel && /openai|anthropic|groq/i.test(llmLabel) ? ' — external: prompts, tool results and memory it recalls leave the platform' : ''}</dd>
    </dl>
  </div>

  <div class="card">
    <div class="card-head"><h3>Tools</h3>
      <span>{allow ? `limited to ${allow.length} by its allow-list` : 'not restricted — every tool of these servers may run'}; policies still decide each call</span></div>
    {#if live.toolsLoading}
      <p class="dim">Listing its servers' tools…</p>
    {:else if !live.tools.length}
      <p class="dim">No tool servers.</p>
    {:else}
      {#each live.tools as s (s.server)}
        <div class="srv">
          <div class="row"><b class="mono">{s.server}</b><span class="count">{s.tools.length}</span>{#if s.error}<span class="err">{s.error}</span>{/if}</div>
          <div class="chips-t">
            {#each s.tools as t (t)}
              {@const on = !allow || allow.includes(t)}
              <span class="t" class:off={!on} class:danger={on && isDestructive(t)} title={on ? (isDestructive(t) ? 'Allowed — destructive' : 'Allowed') : 'Not in its allow-list'}>{t}</span>
            {/each}
          </div>
        </div>
      {/each}
      <p class="dim legend"><span class="t">allowed</span> <span class="t danger">destructive, allowed</span> <span class="t off">not allowed</span></p>
    {/if}
  </div>

  <div class="card">
    <div class="card-head"><h3>What its code touches</h3><span>named in {tpl ? `${tpl.name}@${tpl.version}` : 'its template'}</span></div>
    {#if uses.length}
      <table class="tbl"><tbody>{#each uses as u (u.name)}<tr><td class="mono">{u.name}</td><td class="k">{u.kind}</td></tr>{/each}</tbody></table>
    {:else}
      <p class="dim">Its code names no databases, flows or other resources directly — only its model and tools.</p>
    {/if}
  </div>

  <div class="card">
    <div class="card-head"><h3>Memory</h3></div>
    <p class="dim">{agent.mode === 'persistent' ? 'Kept across conversations: what callers tell it can come back later, to any caller of this agent (whether memory is per caller is not confirmed).' : agent.mode === 'session' ? 'Per conversation: kept only while one conversation lasts.' : `Mode ${agent.mode}.`}
      {#if f.memory}<span class="mono"> {JSON.stringify(f.memory)}</span>{/if}</p>
  </div>

  <div class="card flush">
    <div class="card-head pad"><h3>Values</h3><span>as code — what this agent fills into its template</span><span class="grow"></span>
      {#if canEdit}<button class="btn2" onclick={() => (editing = !editing)}>{editing ? 'Done' : 'Edit'}</button>{/if}</div>
    {#if editing}
      <div class="pad-in">{@render editor()}</div>
    {:else}
      <div class="ed"><YamlEditor value={toYaml(bindings, `${agent.name} — values`)} readOnly /></div>
    {/if}
  </div>
</section>

<style>
  .kv { grid-template-columns: 100px 1fr; }
  .warnline { display: block; margin-top: 3px; color: var(--warn, #b45309); font-size: 12.5px; }
  .srv { display: flex; flex-direction: column; gap: 6px; padding: 8px 0; border-top: 1px solid var(--divider, var(--border)); }
  .srv:first-of-type { border-top: 0; }
  .chips-t { display: flex; flex-wrap: wrap; gap: 5px; }
  .t { font: 11.5px var(--font-mono); padding: 2px 8px; border-radius: 999px; border: 1px solid var(--border); background: var(--bg-surface); color: var(--text-2); }
  .t.off { color: var(--text-4); text-decoration: line-through; background: var(--bg-subtle); }
  .t.danger { border-color: color-mix(in srgb, var(--err) 50%, var(--border)); color: var(--err); background: color-mix(in srgb, var(--err) 7%, var(--bg-surface)); }
  .legend { display: flex; gap: 8px; align-items: center; }
  :global(.agt) .card.flush { padding: 0; gap: 0; overflow: hidden; }
  .pad { padding: 12px 16px; border-bottom: 1px solid var(--border); }
  .pad-in { padding: 12px 16px; }
  .ed { height: 300px; }
</style>
