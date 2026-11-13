<script lang="ts">
  /// Build › Model & tools — what the agent is, changed here: its instructions,
  /// the model it thinks with, the tool servers and tools it may call, what it
  /// remembers, and how it answers. Save writes its record through the control
  /// plane; the next message uses it. Its values as YAML are folded below.
  import type { Snippet } from 'svelte';
  import { discoverServerTools, type ServerTools } from '$lib/services/mcpTools';
  import type { AgentRec } from './agentData';
  import type { Bindings } from './agentValues';

  type Opt = { value: string; label: string };
  let { agent, bindings = $bindable(), llmResources, mcpResources, canEdit, dirty, saving, saveError, onsave, ondiscard, yaml }: {
    agent: AgentRec;
    bindings: Bindings;
    llmResources: Opt[];
    mcpResources: Opt[];
    canEdit: boolean;
    dirty: boolean;
    saving: boolean;
    saveError: string;
    onsave: () => void;
    ondiscard: () => void;
    /// Its values as YAML (the page's editor).
    yaml: Snippet;
  } = $props();

  // The tools of the servers it has, as they are picked.
  let tools = $state<ServerTools[]>([]);
  let toolsLoading = $state(false);
  let toolsFor = '';
  $effect(() => {
    const key = (bindings.mcp_servers ?? []).join(',');
    if (key === toolsFor) return;
    toolsFor = key;
    toolsLoading = true;
    const servers = [...(bindings.mcp_servers ?? [])];
    (servers.length ? discoverServerTools(servers, agent.namespace || 'a3t').catch(() => []) : Promise.resolve([]))
      .then((t) => { if (toolsFor === key) { tools = t; toolsLoading = false; } });
  });

  function toggleServer(v: string, on: boolean) {
    const cur = bindings.mcp_servers ?? [];
    bindings.mcp_servers = on ? [...cur, v] : cur.filter((x) => x !== v);
  }
  const limited = $derived(Array.isArray(bindings.enabled_tools));
  function setLimited(on: boolean) {
    bindings.enabled_tools = on ? tools.flatMap((t) => t.tools) : null;
  }
  function toggleTool(name: string, on: boolean) {
    const cur = bindings.enabled_tools ?? [];
    bindings.enabled_tools = on ? [...cur, name] : cur.filter((x) => x !== name);
  }
  const MEMORY = [
    { v: 'persistent', t: 'Across conversations', d: 'what it learns and is told comes back in later conversations' },
    { v: 'session', t: 'One conversation', d: 'it remembers the conversation it is in, then starts fresh' },
    { v: 'ephemeral', t: 'Nothing', d: 'every message on its own' },
  ];
</script>

<section class="agt">
  <div class="card">
    <div class="card-head"><h3>Instructions</h3><span>what it is told at the start of every conversation</span></div>
    <textarea rows="6" bind:value={bindings.system_prompt} disabled={!canEdit} placeholder="You are … Answer …"></textarea>
  </div>

  <div class="card">
    <div class="card-head"><h3>Model</h3><span>what it thinks with</span></div>
    <select bind:value={bindings.llm} disabled={!canEdit}>
      {#if !llmResources.some((o) => o.value === bindings.llm)}<option value={bindings.llm}>{bindings.llm || '— choose a model —'}</option>{/if}
      {#each llmResources as o (o.value)}<option value={o.value}>{o.label}</option>{/each}
    </select>
    <div class="two">
      <label>Temperature <small>0 = the same answer each time · 1 = varied</small><input type="number" min="0" max="1" step="0.1" bind:value={bindings.temperature} disabled={!canEdit} /></label>
      <label>Steps per message <small>model and tool calls before it must answer</small><input type="number" min="1" max="50" bind:value={bindings.max_iterations} disabled={!canEdit} /></label>
    </div>
  </div>

  <div class="card">
    <div class="card-head"><h3>Tools</h3><span>the MCP servers it may call, and which of their tools</span></div>
    {#if mcpResources.length}
      <ul class="servers">
        {#each mcpResources as o (o.value)}
          {@const on = (bindings.mcp_servers ?? []).includes(o.value)}
          <li><label class="ck"><input type="checkbox" checked={on} disabled={!canEdit} onchange={(e) => toggleServer(o.value, e.currentTarget.checked)} /> <span class="mono">{o.label}</span></label></li>
        {/each}
      </ul>
    {:else}<p class="dim">No MCP servers in reach. Add one in Resources.</p>{/if}
    {#if (bindings.mcp_servers ?? []).length}
      <label class="ck"><input type="checkbox" checked={limited} disabled={!canEdit} onchange={(e) => setLimited(e.currentTarget.checked)} /> Only the tools ticked below (otherwise every tool of these servers)</label>
      {#if toolsLoading}<p class="dim">Listing their tools…</p>
      {:else}
        {#each tools as s (s.server)}
          <div class="srv"><b class="mono">{s.server}</b>{#if s.error}<span class="err"> {s.error}</span>{/if}
            <div class="tools">
              {#each s.tools as t (t)}
                <label class="ck tool"><input type="checkbox" disabled={!canEdit || !limited} checked={!limited || (bindings.enabled_tools ?? []).includes(t)} onchange={(e) => toggleTool(t, e.currentTarget.checked)} /> <span class="mono">{t}</span></label>
              {:else}<span class="dim">no tools listed</span>{/each}
            </div>
          </div>
        {/each}
      {/if}
    {/if}
  </div>

  <div class="card">
    <div class="card-head"><h3>Memory</h3><span>what it remembers — see it in Memory</span></div>
    <div class="mem">
      {#each MEMORY as m (m.v)}
        <label class="opt" class:on={bindings.mode === m.v}><input type="radio" name="mem-{agent.id}" value={m.v} bind:group={bindings.mode} disabled={!canEdit} /><b>{m.t}</b><span class="dim">{m.d}</span></label>
      {/each}
    </div>
  </div>

  {#if canEdit}
    <div class="save">
      <button class="btn2 primary" disabled={!dirty || saving} onclick={onsave}>{saving ? 'Saving…' : 'Save'}</button>
      <button class="btn2 quiet" disabled={!dirty || saving} onclick={ondiscard}>Discard</button>
      <span class="dim">{dirty ? 'Not saved yet. The next message uses what is saved.' : 'Saved: the next message uses this.'}</span>
      {#if saveError && saveError !== 'Saved'}<span class="err">{saveError}</span>{/if}
    </div>
  {:else}<p class="dim">Only its project's admins change it.</p>{/if}

  <details class="card adv">
    <summary>Its values as YAML</summary>
    {@render yaml()}
  </details>
</section>

<style>
  textarea { width: 100%; font: 13px/1.5 var(--font-sans); resize: vertical; }
  select { width: 100%; }
  .two { display: grid; grid-template-columns: 1fr 1fr; gap: 12px; }
  .two label { display: flex; flex-direction: column; gap: 4px; font-size: 12.5px; color: var(--text-2); text-transform: none; letter-spacing: 0; font-weight: 500; }
  small { font-weight: 400; color: var(--text-4); }
  .servers { list-style: none; margin: 0; padding: 0; display: grid; grid-template-columns: repeat(auto-fill, minmax(220px, 1fr)); gap: 6px; }
  .ck { display: flex; align-items: center; gap: 8px; font-size: 13px; color: var(--text-2); text-transform: none; letter-spacing: 0; font-weight: 400; }
  .ck input, .opt input { width: auto; }
  .srv { display: flex; flex-direction: column; gap: 6px; padding: 8px 10px; border: 1px solid var(--border); border-radius: 8px; }
  .tools { display: grid; grid-template-columns: repeat(auto-fill, minmax(230px, 1fr)); gap: 4px; }
  .tool { font-size: 12px; }
  .mem { display: grid; grid-template-columns: repeat(auto-fit, minmax(200px, 1fr)); gap: 8px; }
  .opt { display: grid; grid-template-columns: auto 1fr; gap: 2px 8px; padding: 10px 12px; border: 1px solid var(--border); border-radius: 8px; cursor: pointer; text-transform: none; letter-spacing: 0; font-weight: 400; }
  .opt.on { border-color: var(--spec); background: var(--spec-bg); }
  .opt .dim { grid-column: 2; font-size: 12px; }
  .save { position: sticky; bottom: 0; display: flex; gap: 10px; align-items: center; padding: 10px 0; background: var(--bg-page); }
  .adv > summary { cursor: pointer; font-size: 12.5px; color: var(--text-3); }
  .mono { font-family: var(--font-mono); font-size: 0.92em; }
</style>
