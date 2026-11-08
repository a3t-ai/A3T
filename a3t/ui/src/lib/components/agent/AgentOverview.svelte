<script lang="ts">
  /// Overview — the agent as a product: what it does, whether it is serving,
  /// where its code is, how to call it, and the ways to make another from it.
  import { factsOf, lineageOf, forksOf, ago, dur, type AgentRec, type TemplateRec } from './agentData';
  import type { AgentLive } from './agentLive.svelte';
  import { endpointUrl, copy } from './agentApi';

  let { agent, templates, allAgents, live, me, ongo, onstudio, oncode, onlifecycle }: {
    agent: AgentRec & { description?: string; created_by?: string };
    templates: TemplateRec[];
    allAgents: AgentRec[];
    live: AgentLive;
    me: string;
    ongo: (tab: string) => void;
    onstudio: () => void;
    /// Open its template's code.
    oncode: () => void;
    onlifecycle: (kind: 'clone' | 'fork' | 'branch') => void;
  } = $props();

  const f = $derived(factsOf(agent, templates));
  const lineage = $derived(lineageOf(agent, allAgents));
  const forks = $derived(forksOf(agent, allAgents));
  const purpose = $derived(agent.description || f.purpose);
  const mine = $derived(!!me && agent.created_by === me);
  const p = $derived(live.placement);
  const url = $derived(f.endpoint ? endpointUrl(f.endpoint) : '');
  let copied = $state(false);
</script>

<section class="agt">
  <div class="card hero">
    <p class="purpose" class:none={!purpose}>{purpose || 'No description yet — add one in Settings.'}</p>
    <div class="row status">
      <span class="dot {live.health.tone}"></span><b>{live.health.word}</b>
      <span class="sep">·</span>
      <span>{live.loading ? '…' : p ? `${p.runtime || 'runtime being chosen'}${p.canvas ? ` · ${p.canvas}` : ''}` : 'answers on the shared runtime'}</span>
      {#if live.lastSeen}<span class="sep">·</span><span>last call {ago(live.lastSeen)}</span>{/if}
      <span class="grow"></span>
      <span class="tag" class:dev={mine}>{mine ? 'yours' : agent.created_by ? `owner ${agent.created_by}` : 'unowned'}</span>
    </div>
    <dl class="kv src">
      <dt>Code</dt>
      <dd>{#if f.template}<button class="link mono" onclick={oncode}>{f.template.name}@{f.template.version}</button>{:else}<span class="mono">{agent.template_id ?? '—'}</span>{/if}
        {#if lineage.kind !== 'template'}<span class="dim"> · {lineage.kind === 'fork' ? 'forked from' : 'cloned from'} {lineage.from?.name}</span>{/if}
        {#if forks.length}<span class="dim"> · </span><button class="link" onclick={() => ongo('releases')}>{forks.length} fork{forks.length === 1 ? '' : 's'}</button>{/if}</dd>
      <dt>Built with</dt>
      <dd><span class="mono">{f.llm || 'no model'}</span><span class="dim"> · {f.mcp.length ? f.mcp.map((m) => m.split('/').pop()).join(', ') : 'no tools'} · memory {agent.mode}</span>
        <button class="link" onclick={() => ongo('config')}>Config →</button></dd>
    </dl>
    {#if url}
      <div class="ep">
        <span class="verb">POST</span><span class="path">{url}</span>
        <button class="btn2" onclick={async () => { copied = await copy(url); setTimeout(() => (copied = false), 1500); }}>{copied ? 'Copied' : 'Copy'}</button>
        <button class="btn2" onclick={() => ongo('api')}>API →</button>
      </div>
    {/if}
    <div class="acts">
      <button class="btn2 primary" onclick={onstudio}>Test in Studio</button>
      <span class="sep-v"></span>
      <button class="btn2" onclick={() => onlifecycle('branch')} title="Change and test it without touching prod">Branch to dev</button>
      <button class="btn2" onclick={() => onlifecycle('fork')} title="A new agent that keeps its link to this one">Fork</button>
      <button class="btn2" onclick={() => onlifecycle('clone')} title="A new, independent agent, the same to start with">Clone</button>
    </div>
  </div>

  <div class="tiles">
    {#if p?.stats}
      <button class="tile" onclick={() => ongo('observe')}><span class="v">{p.stats.runsLastHour}</span><span class="k">Calls, last hour</span></button>
      <button class="tile" class:bad={p.stats.errorRate > 0.02} onclick={() => ongo('observe')}><span class="v">{(p.stats.errorRate * 100).toFixed(1)}%</span><span class="k">Errors</span></button>
      <button class="tile" onclick={() => ongo('observe')}><span class="v">{dur(p.stats.p95Ms)}</span><span class="k">p95 latency</span></button>
    {:else}
      <button class="tile" onclick={() => ongo('observe')}><span class="v">{live.loading ? '…' : live.runsDay.length}</span><span class="k">Runs of its flow, 24 h</span></button>
      <button class="tile" class:bad={live.failed.length > 0} onclick={() => ongo('observe')}><span class="v">{live.loading ? '…' : live.failed.length}</span><span class="k">Failed, its flow</span></button>
      <button class="tile" onclick={() => ongo('observe')}><span class="v">{live.loading ? '…' : live.convs.length}</span><span class="k">Your test chats</span></button>
    {/if}
    <button class="tile" class:bad={live.denied.length > 0} onclick={() => ongo('observe')}><span class="v">{live.loading ? '…' : live.decErr ? '—' : live.denied.length}</span><span class="k">Blocked actions</span></button>
  </div>
  {#if !p && !live.loading}<p class="dim">Not deployed: the figures are of its serving flow, which other agents share. Deploy it in Releases for its own.</p>{/if}
</section>

<style>
  .hero { gap: 12px; padding: 16px 18px; }
  .purpose { margin: 0; font-size: 15px; line-height: 1.5; color: var(--text-1); }
  .purpose.none { color: var(--text-4); font-size: 14px; }
  .status { gap: 6px; font-size: 12.5px; color: var(--text-3); }
  .status b { color: var(--text-1); font-weight: 600; }
  .sep { color: var(--text-4); }
  .src { grid-template-columns: 80px 1fr; font-size: 12.5px; }
  .src .link { margin-left: 4px; }
  .src .link.mono { margin-left: 0; }
  .sep-v { width: 1px; height: 18px; background: var(--border); margin: 0 4px; }
  .tiles { grid-template-columns: repeat(4, 1fr); }
</style>
