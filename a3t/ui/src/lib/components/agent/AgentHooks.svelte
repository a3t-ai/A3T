<script lang="ts">
  /// Build › Hooks — this agent's own flows around each turn, run by its
  /// template (a3t/tmpl_a3t_agent): before each turn (they can change the
  /// message or refuse it) and after it (they see the message and the answer —
  /// to log, notify or check; the answer has already streamed). Attach any flow
  /// it can reach, or make a starter hook flow and open it in the Studio.
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

  type Phase = 'before_turn' | 'after_turn';
  const PHASES: { k: Phase; t: string; gets: string; returns: string }[] = [
    { k: 'before_turn', t: 'Before each turn', gets: '{ agent_id, session_id, message }',
      returns: 'Return { message } to change what it gets, or { refuse: "why" } to stop the turn. A hook that fails stops the turn.' },
    { k: 'after_turn', t: 'After each turn', gets: '{ agent_id, session_id, message, reply }',
      returns: 'For logging, notifying or checking: the answer has already streamed, so it can\'t change it. A failure is reported, never shown to the caller.' },
  ];
  const list = (k: Phase) => bindings.hooks?.[k] ?? [];
  function setList(k: Phase, v: string[]) {
    bindings.hooks = { before_turn: [...(bindings.hooks?.before_turn ?? [])], after_turn: [...(bindings.hooks?.after_turn ?? [])], [k]: v };
  }
  let pick = $state<Record<Phase, string>>({ before_turn: '', after_turn: '' });
  function add(k: Phase) { if (pick[k] && !list(k).includes(pick[k])) setList(k, [...list(k), pick[k]]); pick[k] = ''; }
  function move(k: Phase, i: number, d: -1 | 1) { const l = [...list(k)]; [l[i], l[i + d]] = [l[i + d], l[i]]; setList(k, l); }

  // A starter hook flow in its project, attached and opened in the Studio.
  let making = $state('');
  let err = $state('');
  const slug = (s: string) => s.toLowerCase().replace(/[^a-z0-9]+/g, '_').replace(/^_|_$/g, '').slice(0, 40) || 'agent';
  async function makeFlow(k: Phase) {
    const ns = agent.namespace || 'default';
    const name = `flow_${slug(ns)}_${slug(agent.name)}_${k}`;
    const body = k === 'before_turn'
      ? `  # Gets { agent_id, session_id, message }. Return { message } to change it,\n  # or { refuse: "why" } to stop the turn.\n  - pass:\n      set:\n        message: '\${ .message }'\n`
      : `  # Gets { agent_id, session_id, message, reply } after the answer has streamed.\n  - note:\n      set:\n        seen: '\${ (.reply // "") | length }'\n`;
    const yaml = `document:\n  dsl: '1.0.3'\n  namespace: ${ns}\n  name: ${name}\n  version: '1.0.0'\n  metadata:\n    description: ${k === 'before_turn' ? 'Before' : 'After'} each turn of agent ${agent.name} (${agent.id}).\ndo:\n${body}`;
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
  {#each PHASES as ph (ph.k)}
    <div class="card">
      <div class="card-head"><h3>{ph.t}</h3><span>gets <span class="mono">{ph.gets}</span></span></div>
      <p class="dim">{ph.returns}</p>
      {#if list(ph.k).length}
        <ol class="hooks">
          {#each list(ph.k) as ref, i (ref)}
            <li><span class="n">{i + 1}</span><span class="mono">{ref}</span><span class="grow"></span>
              <button class="link" onclick={() => openInStudio('flow', ref.split('/')[0], ref.split('/').slice(1).join('/'))}>Open</button>
              {#if canEdit}
                <button class="btn2 quiet" disabled={i === 0} onclick={() => move(ph.k, i, -1)} aria-label="Earlier">↑</button>
                <button class="btn2 quiet" disabled={i === list(ph.k).length - 1} onclick={() => move(ph.k, i, 1)} aria-label="Later">↓</button>
                <button class="btn2 quiet" onclick={() => setList(ph.k, list(ph.k).filter((x) => x !== ref))}>Remove</button>
              {/if}</li>
          {/each}
        </ol>
      {:else}<p class="dim">None.</p>{/if}
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
    </div>
  {/each}
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
  .hooks { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 6px; }
  .hooks li { display: flex; align-items: center; gap: 8px; padding: 6px 10px; border: 1px solid var(--border); border-radius: 8px; font-size: 13px; }
  .n { width: 20px; height: 20px; border-radius: 50%; display: inline-flex; align-items: center; justify-content: center; font: 600 11px var(--font-mono); background: var(--bg-subtle); color: var(--text-3); }
  .grow { flex: 1 1 auto; }
  .mono { font-family: var(--font-mono); font-size: 0.92em; }
  .save { display: flex; gap: 10px; align-items: center; }
</style>
