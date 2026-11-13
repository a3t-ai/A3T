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
  // ── Servers as folded rows: sized for ~20 servers, each with dozens of tools ──
  const toolsOf = (v: string) => tools.find((t) => t.server === v || t.server.endsWith(`/${v}`) || v.endsWith(`/${t.server}`));
  const isOn = (v: string) => (bindings.mcp_servers ?? []).includes(v);
  let srvFilter = $state('');
  let showAll = $state(false);
  let open = $state<Record<string, boolean>>({});
  let toolFilter = $state<Record<string, string>>({});
  const rows = $derived.by(() => {
    const f = srvFilter.trim().toLowerCase();
    return mcpResources
      // Connected servers always; the others when unfolded, searched for, or few.
      .filter((o) => (isOn(o.value) || showAll || !!f || mcpResources.length <= 8) && (!f || o.label.toLowerCase().includes(f) || o.value.toLowerCase().includes(f)))
      .sort((a, b) => Number(isOn(b.value)) - Number(isOn(a.value)) || a.label.localeCompare(b.label));
  });
  const others = $derived(mcpResources.filter((o) => !isOn(o.value)).length);
  const totalTools = $derived(tools.reduce((n, t) => n + t.tools.length, 0));
  const pickedTools = $derived(limited ? (bindings.enabled_tools ?? []).length : totalTools);
  /// "all 63 tools", "12 of 63 tools", or why there are none.
  function count(v: string): string {
    if (!isOn(v)) return 'not connected';
    if (toolsLoading) return 'listing tools…';
    const t = toolsOf(v);
    if (!t) return 'tools unknown';
    if (t.error) return 'could not list tools';
    const n = t.tools.length;
    if (!limited) return `all ${n} tool${n === 1 ? '' : 's'}`;
    const k = t.tools.filter((x) => (bindings.enabled_tools ?? []).includes(x)).length;
    return `${k} of ${n} tool${n === 1 ? '' : 's'}`;
  }
  function setAll(v: string, on: boolean) {
    const names = toolsOf(v)?.tools ?? [];
    const cur = bindings.enabled_tools ?? [];
    bindings.enabled_tools = on ? [...new Set([...cur, ...names])] : cur.filter((x) => !names.includes(x));
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
    <div class="card-head"><h3>Tools</h3><span>the MCP servers it may call, and which of their tools</span><span class="grow"></span>
      <span class="sum">{(bindings.mcp_servers ?? []).length} of {mcpResources.length} servers {#if (bindings.mcp_servers ?? []).length && !toolsLoading}· {limited ? `${pickedTools} of ${totalTools}` : `all ${totalTools}`} tools{/if}</span></div>
    {#if mcpResources.length}
      <div class="bar">
        {#if mcpResources.length > 8}
          <input type="search" bind:value={srvFilter} placeholder="Find a server" aria-label="Find a server" />
        {/if}
        {#if (bindings.mcp_servers ?? []).length}
          <label class="ck"><input type="checkbox" checked={limited} disabled={!canEdit} onchange={(e) => setLimited(e.currentTarget.checked)} /> Only the tools ticked (otherwise every tool of the connected servers)</label>
        {/if}
      </div>
      <ul class="servers">
        {#each rows as o (o.value)}
          {@const on = isOn(o.value)}
          {@const t = toolsOf(o.value)}
          {@const unfolded = on && !!open[o.value]}
          <li class="srv" class:on>
            <div class="row">
              <input type="checkbox" checked={on} disabled={!canEdit} aria-label="Connect {o.label}" onchange={(e) => toggleServer(o.value, e.currentTarget.checked)} />
              <button class="fold" disabled={!on || !t?.tools.length} aria-expanded={unfolded} onclick={() => (open[o.value] = !open[o.value])}>
                <span class="chev" class:down={unfolded}>›</span>
                <span class="mono name">{o.label}</span>
                <span class="cnt" class:err={!!t?.error}>{count(o.value)}</span>
              </button>
            </div>
            {#if unfolded && t}
              <div class="inner">
                {#if t.error}<p class="err">{t.error}</p>{/if}
                <div class="tbar">
                  {#if t.tools.length > 12}<input type="search" placeholder="Filter {t.tools.length} tools" bind:value={toolFilter[o.value]} aria-label="Filter tools of {o.label}" />{/if}
                  {#if limited && canEdit}<button class="link" onclick={() => setAll(o.value, true)}>All</button><button class="link" onclick={() => setAll(o.value, false)}>None</button>{/if}
                  {#if !limited}<span class="dim">Every tool is allowed. Tick “Only the tools ticked” to choose.</span>{/if}
                </div>
                <div class="tools">
                  {#each t.tools.filter((x) => !(toolFilter[o.value] ?? '').trim() || x.toLowerCase().includes((toolFilter[o.value] ?? '').trim().toLowerCase())) as name (name)}
                    <label class="ck tool"><input type="checkbox" disabled={!canEdit || !limited} checked={!limited || (bindings.enabled_tools ?? []).includes(name)} onchange={(e) => toggleTool(name, e.currentTarget.checked)} /> <span class="mono">{name}</span></label>
                  {:else}<span class="dim">{t.tools.length ? 'Nothing matches.' : 'no tools listed'}</span>{/each}
                </div>
              </div>
            {/if}
          </li>
        {:else}<li class="row dim">{srvFilter.trim() ? 'No server matches.' : 'No server connected yet.'}</li>{/each}
        {#if mcpResources.length > 8 && !srvFilter.trim() && others > 0}
          <li class="srv more"><button class="fold" onclick={() => (showAll = !showAll)}><span class="chev" class:down={showAll}>›</span>
            {showAll ? 'Hide the servers not connected' : `+ ${others} more server${others === 1 ? '' : 's'} in reach — connect one`}</button></li>
        {/if}
      </ul>
    {:else}<p class="dim">No MCP servers in reach. Add one in Resources.</p>{/if}
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
  .grow { flex: 1 1 auto; }
  .sum { font: 12px var(--font-mono); color: var(--text-3); white-space: nowrap; }
  .bar { display: flex; flex-wrap: wrap; gap: 8px 16px; align-items: center; margin-bottom: 8px; }
  .bar input[type='search'] { width: 220px; height: 30px; }
  .servers { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; border: 1px solid var(--border); border-radius: 8px; overflow: hidden; }
  .srv { border-top: 1px solid var(--border); }
  .srv:first-child { border-top: 0; }
  .srv.on { background: var(--bg-surface); }
  .more .fold { padding: 0 12px; color: var(--spec); font-size: 12.5px; }
  li.row.dim { font-size: 12.5px; }
  .row { display: flex; align-items: center; gap: 10px; padding: 0 12px; min-height: 38px; }
  .row > input { width: auto; }
  .fold { flex: 1; display: flex; align-items: center; gap: 8px; min-width: 0; height: 38px; border: 0; background: none; padding: 0; font: inherit; color: var(--text-1); cursor: pointer; text-align: left; }
  .fold:disabled { cursor: default; color: var(--text-3); }
  .chev { width: 12px; color: var(--text-4); transition: transform 0.12s; display: inline-block; }
  .chev.down { transform: rotate(90deg); }
  .fold:disabled .chev { visibility: hidden; }
  .name { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .cnt { margin-left: auto; font: 12px var(--font-mono); color: var(--text-3); white-space: nowrap; }
  .cnt.err { color: var(--err); }
  .inner { padding: 4px 12px 12px 34px; display: flex; flex-direction: column; gap: 8px; }
  .tbar { display: flex; flex-wrap: wrap; align-items: center; gap: 12px; }
  .tbar input[type='search'] { width: 220px; height: 28px; }
  .ck { display: flex; align-items: center; gap: 8px; font-size: 13px; color: var(--text-2); text-transform: none; letter-spacing: 0; font-weight: 400; }
  .ck input, .opt input { width: auto; }
  .tools { display: grid; grid-template-columns: repeat(auto-fill, minmax(210px, 1fr)); gap: 4px 12px; max-height: 320px; overflow: auto; }
  .tool { font-size: 12px; }
  @container details (max-width: 600px) { .bar input[type='search'], .tbar input[type='search'] { width: 100%; } .cnt { display: none; } }
  .mem { display: grid; grid-template-columns: repeat(auto-fit, minmax(200px, 1fr)); gap: 8px; }
  .opt { display: grid; grid-template-columns: auto 1fr; gap: 2px 8px; padding: 10px 12px; border: 1px solid var(--border); border-radius: 8px; cursor: pointer; text-transform: none; letter-spacing: 0; font-weight: 400; }
  .opt.on { border-color: var(--spec); background: var(--spec-bg); }
  .opt .dim { grid-column: 2; font-size: 12px; }
  .save { position: sticky; bottom: 0; display: flex; gap: 10px; align-items: center; padding: 10px 0; background: var(--bg-page); }
  .adv > summary { cursor: pointer; font-size: 12.5px; color: var(--text-3); }
  .mono { font-family: var(--font-mono); font-size: 0.92em; }
</style>
