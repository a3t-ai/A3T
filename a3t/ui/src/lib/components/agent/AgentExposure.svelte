<script lang="ts">
  /// Exposure — how the agent exposes itself, and who has called it. Its owner
  /// chooses the interfaces: REST (its webhook — live today), MCP (it is a tool
  /// to Claude Code, VS Code, apps, other agents) and A2A (other agents find it
  /// by its agent card and send it tasks). MCP and A2A are Sample until the
  /// server serves them (SRV-19); REST is open without a token today (SRV-16).
  import { factsOf, ago, type AgentRec, type TemplateRec } from './agentData';
  import { readTemplate } from '$lib/components/templates/templateSpec';
  import type { AgentLive } from './agentLive.svelte';
  import { endpointUrl, copy } from './agentApi';
  import { mcpUrl, toolOf, connectSnippet, agentCard, cardUrl, type Client } from './agentMcp';
  import AgentApi from './AgentApi.svelte';

  let { agent, templates, live, ongo }: {
    agent: AgentRec & { description?: string };
    templates: (TemplateRec & { spec_yaml: string })[];
    live: AgentLive;
    ongo: (tab: string) => void;
  } = $props();

  const f = $derived(factsOf(agent, templates));
  const t = $derived(readTemplate(templates.find((x) => x.id === agent.template_id)?.spec_yaml));
  const restOn = $derived(!!f.endpoint && live.trigger?.enabled !== false);
  // Sample choices for the interfaces the server does not serve yet: not saved.
  let mcpOn = $state(true);
  let a2aOn = $state(false);

  const murl = $derived(mcpUrl(agent.name, agent.namespace));
  const tool = $derived(toolOf(agent, f.purpose, t.inputs, t.outputs));
  let client = $state<Client>('claude');
  const mcode = $derived(connectSnippet(client, agent.name, murl));
  const card = $derived(agentCard(agent, f.purpose, f.template?.version ?? '1.0.0', t.inputs));
  const curl = $derived(cardUrl(agent.name, agent.namespace));

  let view = $state<'rest' | 'mcp' | 'a2a'>('mcp');
  let copied = $state('');
  async function cp(k: string, s: string) { if (await copy(s)) { copied = k; setTimeout(() => (copied = ''), 1500); } }

  /// Who has called it, from the decision log: the users its calls ran for.
  const callers = $derived.by(() => {
    const m = new Map<string, { who: string; n: number; last: number; blocked: number }>();
    for (const d of live.decisions) {
      const who = d.obo || (d.principal.startsWith('agent:') ? '' : d.principal);
      if (!who) continue;
      const e = m.get(who) ?? { who, n: 0, last: 0, blocked: 0 };
      e.n++; e.last = Math.max(e.last, d.ts); if (!d.allow) e.blocked++;
      m.set(who, e);
    }
    return [...m.values()].sort((a, b) => b.last - a.last);
  });
</script>

<section class="agt">
  <div class="card">
    <div class="card-head"><h3>How it is exposed</h3><span>its owner chooses; each interface has its own address</span></div>
    <ul class="ifs">
      <li class:sel={view === 'rest'}>
        <button class="pick" onclick={() => (view = 'rest')}><b>REST</b><span class="dim">HTTP webhook — live</span></button>
        <span class="st">{#if !f.endpoint}<span class="dim">no endpoint</span>{:else if !live.trigger}…{:else}<span class="tag" class:prod={restOn}>{restOn ? 'on' : 'blocked'}</span>{/if}</span>
        <button class="link" onclick={() => ongo('controls')}>{restOn ? 'Block' : 'Unblock'} in Controls</button>
      </li>
      <li class:sel={view === 'mcp'}>
        <button class="pick" onclick={() => (view = 'mcp')}><b>MCP</b><span class="dim">a tool to Claude Code, VS Code, apps, agents</span></button>
        <span class="st"><span class="tag sample">sample</span></span>
        <label class="sw"><input type="checkbox" bind:checked={mcpOn} /> {mcpOn ? 'on' : 'off'}</label>
      </li>
      <li class:sel={view === 'a2a'}>
        <button class="pick" onclick={() => (view = 'a2a')}><b>A2A</b><span class="dim">Agent2Agent — other agents send it tasks</span></button>
        <span class="st"><span class="tag sample">sample</span></span>
        <label class="sw"><input type="checkbox" bind:checked={a2aOn} /> {a2aOn ? 'on' : 'off'}</label>
      </li>
    </ul>
    <p class="dim">Every interface needs a user's sign-in or an app key — except REST today, which also takes calls with neither (SRV-16).</p>
  </div>

  {#if view === 'rest'}
    <div class="card">
      <div class="card-head"><h3>REST</h3><span>live today</span></div>
      {#if f.endpoint}
        <div class="ep"><span class="verb">POST</span><span class="path">{endpointUrl(f.endpoint)}</span><button class="btn2" onclick={() => cp('rest', endpointUrl(f.endpoint))}>{copied === 'rest' ? 'Copied' : 'Copy'}</button></div>
        {#if restOn}<div class="note bad">Accepts calls <b>without a token</b>: they run with no user (SRV-16).</div>{/if}
        <AgentApi {agent} {templates} {ongo} />
      {:else}
        <p class="dim">It has no webhook. Its developers give it one in its values (<span class="mono">endpoint:</span>).</p>
      {/if}
    </div>
  {:else if view === 'mcp'}
    <div class="card">
      <div class="card-head"><h3>MCP</h3><span class="tag sample">sample</span><span>{mcpOn ? 'it is one tool to whoever connects' : 'off — not offered over MCP'}</span></div>
      {#if mcpOn}
        <div class="ep"><span class="verb">MCP</span><span class="path">{murl}</span><button class="btn2" onclick={() => cp('murl', murl)}>{copied === 'murl' ? 'Copied' : 'Copy'}</button></div>
        <dl class="kv">
          <dt>Tool</dt><dd><b class="mono">{tool.name}</b> — {tool.description}</dd>
          <dt>Takes</dt><dd class="mono">{Object.keys((tool.inputSchema as any).properties).join(', ')}</dd>
          <dt>Sign-in</dt><dd>OAuth (the client opens a browser), or an app key for a service.</dd>
        </dl>
        <div class="row between">
          <div class="seg">{#each [['claude', 'Claude Code'], ['vscode', 'VS Code'], ['ts', 'TypeScript app']] as [k, l] (k)}<button class:on={client === k} onclick={() => (client = k as Client)}>{l}</button>{/each}</div>
          <button class="link" onclick={() => cp('tool', JSON.stringify(tool, null, 2))}>{copied === 'tool' ? 'Copied' : 'Copy the tool definition'}</button>
        </div>
        <pre class="code">{mcode}<button class="btn2 copy" onclick={() => cp('mcode', mcode)}>{copied === 'mcode' ? 'Copied' : 'Copy'}</button></pre>
      {/if}
      <p class="dim">Not served yet: each agent needs its own MCP endpoint on the server (SRV-19). The switch is not saved.</p>
    </div>
  {:else}
    <div class="card">
      <div class="card-head"><h3>A2A</h3><span class="tag sample">sample</span><span>{a2aOn ? 'other agents discover it by its card' : 'off — not offered to other agents'}</span></div>
      {#if a2aOn}
        <div class="ep"><span class="verb">CARD</span><span class="path">{curl}</span><button class="btn2" onclick={() => cp('curl', curl)}>{copied === 'curl' ? 'Copied' : 'Copy'}</button></div>
        <pre class="code">{JSON.stringify(card, null, 2)}<button class="btn2 copy" onclick={() => cp('card', JSON.stringify(card, null, 2))}>{copied === 'card' ? 'Copied' : 'Copy'}</button></pre>
      {:else}
        <p class="dim">Turn it on to publish an agent card: its name, what it does, its skill, and how to sign in.</p>
      {/if}
      <p class="dim">Not served yet (SRV-19). The switch is not saved.</p>
    </div>
  {/if}

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
  .ifs li.sel { border-color: var(--spec); background: var(--spec-bg); }
  .ifs .pick { display: flex; flex-direction: column; gap: 2px; border: 0; background: none; padding: 0; text-align: left; cursor: pointer; font: inherit; }
  .ifs .pick b { font: 600 13px var(--font-mono); color: var(--text-1); }
  .ifs .link { justify-self: end; }
  .sw { justify-self: end; display: flex; align-items: center; gap: 6px; font-size: 12.5px; color: var(--text-2); text-transform: none; letter-spacing: 0; font-weight: 400; }
  .sw input { width: auto; }
  .kv { grid-template-columns: 70px 1fr; }
  pre.code { max-height: 340px; padding-right: 80px; }
  td.err { color: var(--err); }
</style>
