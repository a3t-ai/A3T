<script lang="ts">
  /// Exposure — the addresses it answers on, and who has called it. Every agent
  /// has a data plane (POST /chat, streamed) and a control plane (the A3T agents'
  /// tools, by its id); a webhook if its values give it one. Blocking it in
  /// Controls closes its data plane and webhook.
  import { ago, factsOf, type AgentRec, type TemplateRec } from './agentData';
  import type { AgentLive } from './agentLive.svelte';
  import { addresses } from './controlPlane';
  import { endpointUrl, copy } from './agentApi';
  import AgentApi from './AgentApi.svelte';

  let { agent, templates, live, ongo }: {
    agent: AgentRec & { description?: string; status?: string };
    templates: (TemplateRec & { spec_yaml: string })[];
    live: AgentLive;
    ongo: (tab: string) => void;
  } = $props();

  const f = $derived(factsOf(agent, templates));
  const a = $derived(addresses(agent.id));
  const open = $derived(agent.status !== 'blocked');
  let copied = $state('');
  async function cp(k: string, s: string) { if (await copy(s)) { copied = k; setTimeout(() => (copied = ''), 1500); } }

  /// Who has called it, from the decision log: the users its calls ran for.
  const callers = $derived.by(() => {
    const m = new Map<string, { who: string; n: number; last: number; blocked: number }>();
    for (const d of live.decisions) {
      const who = d.obo || (d.principal.startsWith('agent:') ? '' : d.principal);
      if (!who) continue;
      const e = m.get(who) ?? { who, n: 0, last: 0, blocked: 0 };
      e.n++; e.last = Math.max(e.last, d.ts); if (d.decision === 'deny') e.blocked++;
      m.set(who, e);
    }
    return [...m.values()].sort((a, b) => b.last - a.last);
  });
</script>

<section class="agt">
  <div class="card">
    <div class="card-head"><h3>Where it answers</h3><span>its own addresses</span></div>
    <ul class="ifs">
      <li>
        <div><b>Data plane</b><span class="dim mono">POST {a.data} · session {a.session}</span></div>
        <span class="tag" class:prod={open}>{open ? 'open' : 'blocked'}</span>
        <button class="link" onclick={() => ongo('controls')}>{open ? 'Block' : 'Unblock'} in Controls</button>
      </li>
      <li>
        <div><b>Control plane</b><span class="dim mono">{a.control} · the A3T agents' tools with its id</span></div>
        <span class="tag prod">on</span>
        <button class="link" onclick={() => cp('ctl', a.control)}>{copied === 'ctl' ? 'Copied' : 'Copy'}</button>
      </li>
      <li>
        <div><b>Webhook</b><span class="dim mono">{f.endpoint ? `POST ${endpointUrl(f.endpoint)}` : 'none — its values give it one (endpoint:)'}</span></div>
        {#if f.endpoint}<span class="tag" class:prod={open}>{open ? 'open' : 'blocked'}</span>{:else}<span class="dim">—</span>{/if}
        {#if f.endpoint}<button class="link" onclick={() => cp('wh', endpointUrl(f.endpoint))}>{copied === 'wh' ? 'Copied' : 'Copy'}</button>{:else}<span></span>{/if}
      </li>
    </ul>
    <p class="dim">MCP and A2A addresses per agent are not served yet (SRV-19); an MCP client reaches it through the A3T agents' tools.</p>
  </div>

  <AgentApi {agent} {templates} />

  <div class="card">
    <div class="card-head"><h3>Who has called it</h3><span>users its calls ran for, from the decision log</span></div>
    {#if live.loading}<p class="dim">…</p>
    {:else if live.decErr}<p class="dim">{live.decErr}</p>
    {:else if callers.length}
      <table class="tbl">
        <thead><tr><th>Caller</th><th>Decisions</th><th>Blocked</th><th>Last</th></tr></thead>
        <tbody>{#each callers as c (c.who)}<tr><td class="mono">{c.who}</td><td>{c.n}</td><td class:err={c.blocked > 0}>{c.blocked}</td><td class="k">{ago(c.last)}</td></tr>{/each}</tbody>
      </table>
    {:else}
      <p class="dim">No calls with a user in the latest {live.scanned} decisions. Calls without a token leave no caller to show.</p>
    {/if}
    <p class="dim">Which app, MCP client or agent called needs app keys (SRV-15).</p>
  </div>
</section>

<style>
  .ifs { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 6px; }
  .ifs li { display: grid; grid-template-columns: 1fr auto 150px; gap: 12px; align-items: center; padding: 8px 12px; border: 1px solid var(--border); border-radius: 8px; }
  .ifs li > div { display: flex; flex-direction: column; gap: 2px; min-width: 0; }
  .ifs li > div .mono { font-size: 11.5px; overflow-wrap: anywhere; }
  .ifs b { font: 600 13px var(--font-sans); color: var(--text-1); }
  .mono { font-family: var(--font-mono); }
  .ifs li.sel { border-color: var(--spec); background: var(--spec-bg); }
  .ifs .pick { display: flex; flex-direction: column; gap: 2px; border: 0; background: none; padding: 0; text-align: left; cursor: pointer; font: inherit; }
  .ifs .pick b { font: 600 13px var(--font-mono); color: var(--text-1); }
  .ifs .link { justify-self: end; }
  .sw { justify-self: end; display: flex; align-items: center; gap: 6px; font-size: 12.5px; color: var(--text-2); text-transform: none; letter-spacing: 0; font-weight: 400; }
  .sw input { width: auto; }
  .kv { grid-template-columns: 70px 1fr; }
  /* Narrow pane: what it is on one line, its state and action below. */
  @container details (max-width: 560px) {
    .ifs li { grid-template-columns: auto 1fr; }
    .ifs li > :first-child { grid-column: 1 / -1; min-width: 0; overflow-wrap: anywhere; }
  }
  pre.code { max-height: 340px; padding-right: 80px; }
  td.err { color: var(--err); }
</style>
