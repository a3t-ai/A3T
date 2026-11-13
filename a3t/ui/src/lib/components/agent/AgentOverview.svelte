<script lang="ts">
  /// Overview — for whoever answers for the agent: what it is, where it runs,
  /// who owns it, what needs attention (each flag links to its evidence), and
  /// the figures that matter. Developer actions are secondary.
  import { factsOf, lineageOf, envOf, ago, dur, type AgentRec, type TemplateRec } from './agentData';
  import type { AgentLive } from './agentLive.svelte';
  import { flagsOf } from './agentRisk';
  import AgentEndpoints from './AgentEndpoints.svelte';
  import AgentMetrics from './AgentMetrics.svelte';

  let { agent, templates, allAgents, live, me, llmLabel, ongo, onstudio, oncode, onlifecycle }: {
    agent: AgentRec & { description?: string; created_by?: string; status?: string };
    templates: TemplateRec[];
    allAgents: AgentRec[];
    live: AgentLive;
    me: string;
    /// Its model resource's label, "<name> (<provider>)".
    llmLabel?: string;
    ongo: (tab: string) => void;
    onstudio: () => void;
    oncode: () => void;
    onlifecycle: (kind: 'clone' | 'fork' | 'branch') => void;
  } = $props();

  const f = $derived(factsOf(agent, templates));
  const lineage = $derived(lineageOf(agent, allAgents));
  const env = $derived(envOf(agent));
  /// What it is for: its own description, else its instructions, else its template's.
  const instructions = $derived.by(() => { try { const v = typeof agent.variables === 'string' ? JSON.parse(agent.variables || '{}') : (agent.variables ?? {}); return (v.system_prompt ?? v.instructions ?? '') as string; } catch { return ''; } });
  const purpose = $derived(agent.description || instructions || f.purpose);
  const flags = $derived(live.loading || live.toolsLoading ? [] : flagsOf(agent, templates, live, llmLabel));
  const serious = $derived(flags.filter((x) => x.level !== 'info'));
  const notes = $derived(flags.filter((x) => x.level === 'info'));
  let showNotes = $state(false);
  const p = $derived(live.placement);
</script>

<section class="agt">
  <div class="card hero">
    <p class="purpose" class:none={!purpose}>{purpose || 'No description — its owner should add one in Settings.'}</p>
    <div class="row status">
      {#if env}<span class="tag" class:prod={env === 'prod'} class:dev={env !== 'prod'}>{env}</span>{:else}<span class="tag">no environment</span>{/if}
      <span class="dot {live.health.tone}"></span><b>{live.health.word}</b>
      {#if p}<span class="sep">·</span><span class="mono">{p.runtime || '—'}</span>{/if}
      {#if live.lastSeen}<span class="sep">·</span><span>last activity {ago(live.lastSeen)}</span>{/if}
      <span class="grow"></span>
      <span class="owner">{agent.created_by ? (agent.created_by === me ? 'owned by you' : `owner ${agent.created_by}`) : 'no owner'}</span>
    </div>
  </div>

  <AgentEndpoints id={agent.id} />

  <AgentMetrics id={agent.id} />

  <div class="card">
    <div class="card-head"><h3>Needs attention</h3>
      {#if live.loading || live.toolsLoading}<span>checking…</span>{:else}<span>{serious.length ? `${serious.length} finding${serious.length === 1 ? '' : 's'}` : 'nothing serious found'}</span>{/if}</div>
    {#if serious.length}
      <ul class="flags">
        {#each serious as fl (fl.title)}
          <li class={fl.level}><span class="lv">{fl.level}</span><div><b>{fl.title}</b><span class="dim">{fl.why}</span></div><button class="link" onclick={() => ongo(fl.tab)}>Look →</button></li>
        {/each}
      </ul>
    {/if}
    {#if notes.length}
      <button class="link" onclick={() => (showNotes = !showNotes)}>{showNotes ? 'Hide' : 'Show'} {notes.length} note{notes.length === 1 ? '' : 's'}</button>
      {#if showNotes}
        <ul class="flags">
          {#each notes as fl (fl.title)}
            <li class="info"><span class="lv">note</span><div><b>{fl.title}</b><span class="dim">{fl.why}</span></div><button class="link" onclick={() => ongo(fl.tab)}>Look →</button></li>
          {/each}
        </ul>
      {/if}
    {/if}
  </div>

  <div class="tiles">
    {#if p?.stats}
      <button class="tile" onclick={() => ongo('audit')}><span class="v">{p.stats.runsLastHour}</span><span class="k">Calls, last hour</span></button>
      <button class="tile" class:bad={p.stats.errorRate > 0.02} onclick={() => ongo('audit')}><span class="v">{(p.stats.errorRate * 100).toFixed(1)}%</span><span class="k">Errors</span></button>
      <button class="tile" onclick={() => ongo('releases')}><span class="v">{dur(p.stats.p95Ms)}</span><span class="k">p95 latency</span></button>
    {:else}
      <button class="tile" class:bad={agent.status === 'blocked'} onclick={() => ongo('controls')}><span class="v">{agent.status === 'blocked' ? 'blocked' : 'open'}</span><span class="k">Its data plane</span></button>
      <button class="tile" onclick={() => ongo('releases')}><span class="v">shared</span><span class="k">Runtime</span></button>
      <button class="tile" onclick={() => ongo('access')}><span class="v">{live.toolsLoading ? '…' : live.allowedTools.length}</span><span class="k">Tools it may call</span></button>
    {/if}
    <button class="tile" class:bad={live.denied.length > 0} onclick={() => ongo('audit')}><span class="v">{live.loading ? '…' : live.decErr ? '—' : live.denied.length}</span><span class="k">Blocked actions</span></button>
  </div>

  <div class="card">
    <div class="card-head"><h3>At a glance</h3></div>
    <dl class="kv">
      <dt>Project</dt><dd class="mono">{agent.namespace}</dd>
      <dt>Code</dt><dd>{#if f.template}<button class="link mono" onclick={oncode}>{f.template.name}@{f.template.version}</button>{:else}<span class="mono">{agent.template_id ?? '—'}</span>{/if}
        {#if lineage.kind !== 'template'}<span class="dim"> · {lineage.kind === 'fork' ? 'forked from' : 'cloned from'} {lineage.from?.name}</span>{/if}</dd>
      <dt>Model</dt><dd><span class="mono">{f.llm || '—'}</span>{#if llmLabel}<span class="dim"> · {llmLabel.replace(/^.*\(|\)$/g, '')}</span>{/if}</dd>
      <dt>Tools</dt><dd>{f.mcp.length ? f.mcp.map((m) => m.split('/').pop()).join(', ') : 'none'}<span class="dim"> · {f.tools ? `limited to ${f.tools.length}` : 'not restricted'}</span> <button class="link" onclick={() => ongo('access')}>Access →</button></dd>
      <dt>Called at</dt><dd><span class="mono">POST /chat</span><span class="dim"> · session {agent.id}:… · {agent.status === 'blocked' ? 'blocked' : 'open'}</span>{#if f.endpoint}<span class="dim"> · webhook </span><span class="mono">/webhook{f.endpoint}</span>{/if} <button class="link" onclick={() => ongo('exposure')}>Exposure →</button></dd>
      <dt>Memory</dt><dd>{agent.mode === 'persistent' ? 'kept across conversations' : agent.mode === 'session' ? 'per conversation' : agent.mode}</dd>
    </dl>
  </div>

  <div class="row devacts">
    <span class="dim">For its developers:</span>
    <button class="link" onclick={onstudio}>Test in Studio</button>
    <button class="link" onclick={() => onlifecycle('branch')}>Branch</button>
    <button class="link" onclick={() => onlifecycle('fork')}>Fork</button>
    <button class="link" onclick={() => onlifecycle('clone')}>Clone</button>
  </div>
</section>

<style>
  .hero { gap: 10px; }
  .purpose { margin: 0; font-size: 15px; line-height: 1.5; color: var(--text-1); }
  .purpose.none { color: var(--text-4); font-size: 14px; }
  .status { gap: 6px; font-size: 12.5px; color: var(--text-3); }
  .status b { color: var(--text-1); font-weight: 600; }
  .sep { color: var(--text-4); }
  .owner { font-size: 12.5px; color: var(--text-2); }
  .flags { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 6px; }
  .flags li { display: grid; grid-template-columns: 64px 1fr auto; gap: 10px; align-items: baseline; padding: 8px 10px; border-radius: 8px; background: var(--bg-subtle); }
  .flags li > div { display: flex; flex-direction: column; gap: 2px; }
  .flags b { font-size: 13px; color: var(--text-1); font-weight: 600; }
  .lv { font: 600 10px var(--font-mono); text-transform: uppercase; letter-spacing: 0.06em; color: var(--text-4); }
  .flags li.high { background: color-mix(in srgb, var(--err) 9%, var(--bg-surface)); }
  .flags li.high .lv { color: var(--err); }
  .flags li.medium { background: color-mix(in srgb, var(--warn) 10%, var(--bg-surface)); }
  .flags li.medium .lv { color: var(--warn); }
  .kv { grid-template-columns: 90px 1fr; }
  .kv .link { margin-left: 4px; }
  .kv .link.mono { margin-left: 0; }
  .devacts { gap: 14px; font-size: 12.5px; }
</style>
