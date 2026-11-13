<script lang="ts">
  /// Build › Hooks — this agent's own flows at each phase of a turn, run by the
  /// agent engine (call:a3t:agent, AH-1): turn start, memory recall, each model
  /// call, each tool call and its result or error, the memory write, turn end.
  /// Where a phase allows it, a hook changes the turn (the message, the prompt, a
  /// tool's arguments or result, what is remembered) or refuses it. Attach any
  /// flow it can reach, or make a starter flow for the phase and open it in Studio.
  import { mcpCall } from '$lib/services/api';
  import { openInStudio } from '$lib/stores/panels.svelte.ts';
  import type { AgentRec } from './agentData';
  import type { Bindings } from './agentValues';

  type Opt = { value: string; label: string };
  let { agent, bindings = $bindable(), flowOptions, canEdit, dirty, saving, saveError, onsave, ondiscard }: {
    agent: AgentRec;
    bindings: Bindings;
    flowOptions: Opt[];
    canEdit: boolean;
    dirty: boolean;
    saving: boolean;
    saveError: string;
    onsave: () => void;
    ondiscard: () => void;
  } = $props();

  /// The phases of a turn, in order (the engine's, call_agent.rs HOOK_PHASES):
  /// what each hook flow gets, what it may return, and a starter body for it.
  type Phase = string;
  const PHASES: { k: Phase; t: string; when: string; gets: string; returns: string; starter: string }[] = [
    { k: 'on_turn_start', t: 'Turn start', when: 'a message arrives', gets: 'message',
      returns: '{ message } replaces it · { refuse: "why" } stops the turn · a failure stops it', starter: "  - pass:\n      set:\n        message: '${ .message }'\n" },
    { k: 'before_recall', t: 'Before memory recall', when: 'before it searches its memory', gets: 'message, query',
      returns: '{ query } is what it searches with', starter: "  - pass:\n      set:\n        query: '${ .query }'\n" },
    { k: 'before_llm', t: 'Before the model', when: 'before each model call', gets: 'message, iteration, system_prompt',
      returns: '{ add_to_prompt } adds to its prompt for this call · { system_prompt } replaces it', starter: "  - add:\n      set:\n        add_to_prompt: ''\n" },
    { k: 'after_llm', t: 'After the model', when: 'after each model call', gets: 'iteration, content, tool_calls',
      returns: 'observes: the text has already streamed', starter: "  - note:\n      set:\n        seen: '${ .tool_calls | length }'\n" },
    { k: 'on_tool_call', t: 'Tool call', when: 'before each tool call', gets: 'tool, input',
      returns: '{ input } replaces the arguments · { refuse: "why" } blocks it (the model is told) · a failure blocks it', starter: "  - pass:\n      set:\n        input: '${ .input }'\n" },
    { k: 'after_tool_call', t: 'Tool result', when: 'after each tool call', gets: 'tool, input, output',
      returns: '{ output } is what the model sees (e.g. redacted)', starter: "  - pass:\n      set:\n        output: '${ .output }'\n" },
    { k: 'on_tool_error', t: 'Tool error', when: 'a tool call failed', gets: 'tool, input, error',
      returns: '{ output } is used in place of the error', starter: "  - note:\n      set:\n        seen: '${ .error }'\n" },
    { k: 'before_store', t: 'Before memory write', when: 'before the turn is remembered', gets: 'message, reply',
      returns: '{ reply } is what is remembered · { skip: true } remembers nothing', starter: "  - pass:\n      set:\n        reply: '${ .reply }'\n" },
    { k: 'on_turn_end', t: 'Turn end', when: 'the caller has its answer', gets: 'message, reply, tool_calls, iterations, outcome',
      returns: 'observes: log, notify, check', starter: "  - note:\n      set:\n        outcome: '${ .outcome }'\n" },
  ];
  const list = (k: Phase) => bindings.hooks?.[k] ?? [];
  function setList(k: Phase, v: string[]) { bindings.hooks = { ...(bindings.hooks ?? {}), [k]: v }; }
  let pick = $state<Record<Phase, string>>({});
  function add(k: Phase) { const v = pick[k]; if (v && !list(k).includes(v)) setList(k, [...list(k), v]); pick[k] = ''; }
  function move(k: Phase, i: number, d: -1 | 1) { const l = [...list(k)]; [l[i], l[i + d]] = [l[i + d], l[i]]; setList(k, l); }
  const total = $derived(Object.values(bindings.hooks ?? {}).reduce((n, l) => n + (l?.length ?? 0), 0));

  // A starter hook flow in its project, attached and opened in the Studio.
  let making = $state('');
  let err = $state('');
  const slug = (s: string) => s.toLowerCase().replace(/[^a-z0-9]+/g, '_').replace(/^_|_$/g, '').slice(0, 40) || 'agent';
  async function makeFlow(k: Phase) {
    const ns = agent.namespace || 'default';
    const name = `flow_${slug(ns)}_${slug(agent.name)}_${k}`;
    const ph = PHASES.find((p) => p.k === k)!;
    const body = `  # ${ph.t}: ${ph.when}. Gets { phase, agent_id, session_id, ${ph.gets} }.\n  # Returns: ${ph.returns}.\n${ph.starter}`;
    const yaml = `document:\n  dsl: '1.0.3'\n  namespace: ${ns}\n  name: ${name}\n  version: '1.0.0'\n  metadata:\n    description: ${ph.t} hook of agent ${agent.name} (${agent.id}).\ndo:\n${body}`;
    making = k; err = '';
    const r = await mcpCall('create_flow', { yaml });
    making = '';
    if (!r.ok && !/exist/i.test(r.error ?? '')) { err = r.error ?? 'The flow was not made.'; return; }
    const ref = `${ns}/${name}`;
    if (!list(k).includes(ref)) setList(k, [...list(k), ref]);
    onsave();
    openInStudio('flow', ns, name);
  }
</script>

<section class="agt">
  <p class="dim">Flows of its own, run at each phase of a turn in the order listed, as the caller. {total ? `${total} attached.` : 'None attached yet.'} A hook flow gets the phase's fields plus <span class="mono">phase, agent_id, session_id</span>.</p>
  <ol class="phases">
    {#each PHASES as ph, pi (ph.k)}
      <li class="card" class:has={list(ph.k).length}>
        <div class="ph-head"><span class="n">{pi + 1}</span><b>{ph.t}</b><span class="dim">{ph.when}</span><span class="grow"></span><span class="mono key">{ph.k}</span></div>
        <div class="dim io">gets <span class="mono">{ph.gets}</span> — {ph.returns}</div>
        {#if list(ph.k).length}
          <ul class="hooks">
            {#each list(ph.k) as ref, i (ref)}
              <li><span class="mono">{ref}</span><span class="grow"></span>
                <button class="link" onclick={() => openInStudio('flow', ref.split('/')[0], ref.split('/').slice(1).join('/'))}>Open</button>
                {#if canEdit}
                  <button class="btn2 quiet" disabled={i === 0} onclick={() => move(ph.k, i, -1)} aria-label="Earlier">↑</button>
                  <button class="btn2 quiet" disabled={i === list(ph.k).length - 1} onclick={() => move(ph.k, i, 1)} aria-label="Later">↓</button>
                  <button class="btn2 quiet" onclick={() => setList(ph.k, list(ph.k).filter((x) => x !== ref))}>Remove</button>
                {/if}</li>
            {/each}
          </ul>
        {/if}
        {#if canEdit}
          <div class="row">
            <select class="grow" bind:value={pick[ph.k]}>
              <option value="">— attach a flow —</option>
              {#each flowOptions.filter((o) => !list(ph.k).includes(o.value)) as o (o.value)}<option value={o.value}>{o.label}</option>{/each}
            </select>
            <button class="btn2" disabled={!pick[ph.k]} onclick={() => add(ph.k)}>Attach</button>
            <button class="btn2" disabled={!!making} onclick={() => makeFlow(ph.k)}>{making === ph.k ? 'Making…' : 'New hook flow'}</button>
          </div>
        {/if}
      </li>
    {/each}
  </ol>
  {#if err}<p class="err">{err}</p>{/if}

  {#if canEdit}
    <div class="save">
      <button class="btn2 primary" disabled={!dirty || saving} onclick={onsave}>{saving ? 'Saving…' : 'Save'}</button>
      <button class="btn2 quiet" disabled={!dirty || saving} onclick={ondiscard}>Discard</button>
      <span class="dim">{dirty ? 'Not saved yet.' : 'Saved: they run from its next message.'}</span>
      {#if saveError && saveError !== 'Saved'}<span class="err">{saveError}</span>{/if}
    </div>
  {/if}
</section>

<style>
  .phases { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 8px; }
  .phases > li { gap: 6px; padding: 10px 14px; }
  .phases > li.has { border-color: var(--spec); }
  .ph-head { display: flex; align-items: baseline; gap: 8px; font-size: 13px; }
  .ph-head b { color: var(--text-1); }
  .key { font-size: 11px; color: var(--text-4); }
  .io { font-size: 12px; }
  .hooks { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 6px; }
  .hooks li { display: flex; align-items: center; gap: 8px; padding: 6px 10px; border: 1px solid var(--border); border-radius: 8px; font-size: 13px; }
  .n { width: 20px; height: 20px; border-radius: 50%; display: inline-flex; align-items: center; justify-content: center; font: 600 11px var(--font-mono); background: var(--bg-subtle); color: var(--text-3); }
  .grow { flex: 1 1 auto; }
  .mono { font-family: var(--font-mono); font-size: 0.92em; }
  .save { display: flex; gap: 10px; align-items: center; }
</style>
