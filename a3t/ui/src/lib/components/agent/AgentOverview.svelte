<script lang="ts">
  /// Overview — in the order its maker works: its endpoints and token, a test
  /// here, the code to call it from an app (folded), then its status (open,
  /// keys, turns, memory, lineage — each opens where it is changed) and where to
  /// go next.
  import { agentSpaceApi, type AgentKey } from '$lib/services/api';
  import { factsOf, ago, dur, type AgentRec, type TemplateRec } from './agentData';
  import { insight, control, addresses, setBlocked, describeEvent, type TurnSummary, type Ancestor, type Descendant, type AgentEvent, type MemoryType } from './controlPlane';
  import { copy, chatSnippet, type Lang } from './agentApi';
  import { keepKey, peekKey, dropKey } from './keyOnce';
  import AgentTryIt from './AgentTryIt.svelte';

  let { agent, templates, canEdit, ongo, onstudio, onlifecycle, onopen, onreload }: {
    agent: AgentRec & { description?: string; created_by?: string; created_at?: string; status?: string };
    templates: TemplateRec[];
    canEdit: boolean;
    ongo: (tab: string) => void;
    onstudio: () => void;
    onlifecycle: (kind: 'clone' | 'fork' | 'branch') => void;
    onopen: (id: string) => void;
    /// Its record changed here (blocked, unblocked): reload it.
    onreload: () => void;
  } = $props();

  const f = $derived(factsOf(agent, templates));
  const a = $derived(addresses(agent.id));
  const blocked = $derived(agent.status === 'blocked');
  const instructions = $derived.by(() => { try { const v = typeof agent.variables === 'string' ? JSON.parse(agent.variables || '{}') : (agent.variables ?? {}); return (v.system_prompt ?? v.instructions ?? '') as string; } catch { return ''; } });

  // What is true now: its turns, keys, memory and lineage.
  let turns = $state<TurnSummary | null>(null);
  let keys = $state<AgentKey[] | null>(null);
  let memTotal = $state<number | null>(null);
  let memCounts = $state<Record<MemoryType, number> | null>(null);
  let events = $state<AgentEvent[] | null>(null);
  let lastTurn = $state<{ session_id: string; outcome: string } | null>(null);
  let lineage = $state<{ ancestors: Ancestor[]; descendants: Descendant[] } | null>(null);
  let fresh = $state<{ name: string; key: string } | null>(null);
  let err = $state('');
  async function load(id: string) {
    const [t, k, m, l, e] = await Promise.all([insight.turns(id), agentSpaceApi.keys(id), insight.memory(id), control.lineage(id), insight.events(id)]);
    if (agent.id !== id) return;
    turns = t.ok ? t.data.summary : null;
    keys = k.ok ? k.data?.keys ?? [] : null;
    memTotal = m.ok ? Object.values(m.data.counts ?? {}).reduce((x: number, y) => x + Number(y || 0), 0) : null;
    memCounts = m.ok ? m.data.counts : null;
    events = e.ok ? e.data : null;
    lastTurn = t.ok ? (t.data.turns ?? [])[0] ?? null : null;
    lineage = l.ok ? l.data ?? null : null;
    if (!k.ok) err = k.error ?? '';
  }
  $effect(() => { const id = agent.id; turns = null; keys = null; memTotal = null; lineage = null; err = ''; fresh = peekKey(id); load(id); });

  const live = $derived((keys ?? []).filter((k) => !k.revoked_at));
  const used = $derived(live.filter((k) => k.last_used_at).sort((x, y) => y.last_used_at.localeCompare(x.last_used_at))[0]);
  const steps = $derived({
    answers: (turns?.turns ?? 0) > 0,
    key: live.length > 0,
    called: !!used,
    figures: (turns?.turns ?? 0) > 0 && (memTotal ?? 0) > 0,
  });

  let busy = $state(false);
  async function makeKey() {
    busy = true; err = '';
    const r = await agentSpaceApi.makeKey(agent.id, 'my-app');
    busy = false;
    if (!r.ok || !r.data?.key) { err = r.error ?? 'No key made.'; return; }
    keepKey(agent.id, { name: r.data.name, key: r.data.key });
    fresh = peekKey(agent.id);
    load(agent.id);
  }
  function saved() { dropKey(agent.id); fresh = null; }
  let copied = $state('');
  async function cp(k: string, s: string) { if (await copy(s)) { copied = k; setTimeout(() => (copied = ''), 1500); } }
  let lang = $state<Lang>('curl');
  const code = $derived(chatSnippet(lang, agent.id));
  const parent = $derived(lineage?.ancestors?.[0]);
  const lastBlock = $derived((events ?? []).find((e) => e.type === 'blocked' || e.type === 'unblocked'));
  const vars = $derived.by(() => { try { return typeof agent.variables === 'string' ? JSON.parse(agent.variables || '{}') : (agent.variables ?? {}); } catch { return {}; } });
  const hookCount = $derived({ before: (vars.hooks?.before_turn ?? []).length, after: (vars.hooks?.after_turn ?? []).length });
  const RELATION: Record<string, string> = {
    template: 'Made from the template', inherit: 'Inherits from', clone: 'Cloned from', fork: 'Forked from', branch: 'Branched from', evolved: 'Evolved from',
  };
  const RELATION_MEANS: Record<string, string> = {
    template: 'its code; values are its own', inherit: "its parent's hooks fire first and its updates arrive",
    clone: 'a copy; nothing flows either way', fork: 'nothing goes back', branch: 'merges back what it learns', evolved: 'its own agent now',
  };
  let blocking = $state(false);
  let reason = $state('');
  let askBlock = $state(false);
  async function toggleBlock(on: boolean) {
    blocking = true; err = '';
    const r = await setBlocked(agent.id, on, on ? reason.trim() || undefined : undefined);
    blocking = false; askBlock = false; reason = '';
    if (!r.ok) { err = r.error ?? 'Not changed.'; return; }
    onreload();
    load(agent.id);
  }
</script>

<section class="agt">
  {#if blocked}<div class="note bad">It is blocked: every call is refused. <button class="link" onclick={() => ongo('controls')}>Unblock it in Controls →</button></div>{/if}

  <div class="card">
    <div class="card-head"><h3>1 · Endpoints</h3><span>where your app calls it</span></div>
    <ul class="eps">
      <li><span class="lbl">Data plane</span><span class="mono url">POST {a.data}</span><button class="btn2" onclick={() => cp('data', a.data)}>{copied === 'data' ? 'Copied' : 'Copy'}</button>
        <span class="dim sub">chat, streamed · body <span class="mono">{'{'}"session_id": "{agent.id}:&lt;conversation&gt;", "message": "…"{'}'}</span></span></li>
      <li><span class="lbl">Token</span>
        {#if fresh}
          <span class="mono url">{fresh.key}</span><button class="btn2 primary" onclick={() => cp('key', fresh!.key)}>{copied === 'key' ? 'Copied' : 'Copy'}</button>
          <span class="dim sub">Key <b>{fresh.name}</b> — copy it now, it is not shown again. <button class="link" onclick={saved}>I have saved it</button></span>
        {:else if keys === null}<span class="dim">…</span><span></span>
        {:else if live.length}
          <span>{live.map((k) => `${k.name} (${k.prefix}…)`).join(', ')}</span><button class="btn2" onclick={() => ongo('keys')}>Manage</button>
          <span class="dim sub">Send it as <span class="mono">Authorization: Bearer &lt;key&gt;</span>. A call without a token is refused.</span>
        {:else}
          <span class="bad">No key — apps can't call it.</span>{#if canEdit}<button class="btn2 primary" disabled={busy} onclick={makeKey}>{busy ? 'Making…' : 'Make a key'}</button>{:else}<span></span>{/if}
        {/if}</li>
      <li><span class="lbl">Control plane</span><span class="mono url">POST {a.control.replace(/\/agents\/.*/, '')}/agents/agent_a3t_agents/tools/&lt;tool&gt;</span><button class="btn2" onclick={() => ongo('exposure')}>Tools</button>
        <span class="dim sub">change, block, share or read it from your backend, with your sign-in: <span class="mono">{'{'}"id": "{agent.id}", …{'}'}</span></span></li>
    </ul>
  </div>

  <div class="card">
    <div class="card-head"><h3>2 · Test it</h3><span>{steps.answers ? `it has answered ${turns?.turns} time${turns?.turns === 1 ? '' : 's'}` : 'send it a message'}</span></div>
    <AgentTryIt agentId={agent.id} onanswered={() => load(agent.id)} />
  </div>

  <div class="card">
    <div class="card-head"><h3>3 · Call it from your app</h3><span>{used ? `key ${used.name} last called it ${ago(used.last_used_at)}` : 'not called with a key yet'}</span><span class="grow"></span>
      <button class="link" onclick={() => load(agent.id)}>Check again</button></div>
    <span class="dim">POST the message to its data plane with one of its keys; the answer streams back.</span>
    <div class="term">
      <div class="term-tabs">{#each [['curl', 'curl'], ['ts', 'TypeScript'], ['python', 'Python']] as [k, l] (k)}<button class:on={lang === k} onclick={() => (lang = k as Lang)}>{l}</button>{/each}
        <span class="grow"></span><button class="term-copy" onclick={() => cp('code', code)}>{copied === 'code' ? 'Copied' : 'Copy'}</button></div>
      <pre>{code}</pre>
    </div>
    <span class="dim">Replace <span class="mono">$A3T_AGENT_KEY</span> with its key.</span>
  </div>

  <div class="card">
    <div class="card-head"><h3>Status</h3><span>what is true now, and what to do about it</span><span class="grow"></span><button class="link" onclick={() => load(agent.id)}>Refresh</button></div>
    <ul class="st">
      <li class:bad={blocked}><span class="k">Calls</span>
        <div>{#if blocked}<b>Blocked</b> — every call is refused{lastBlock?.type === 'blocked' ? ` since ${ago(lastBlock.at)}${lastBlock.by ? `, by ${lastBlock.by}` : ''}${lastBlock.payload?.reason ? ` (${lastBlock.payload.reason})` : ''}` : ''}.
          {:else}<b>Open</b> — it answers every call with one of its keys or a user's sign-in{lastBlock?.type === 'unblocked' ? `; unblocked ${ago(lastBlock.at)}${lastBlock.by ? ` by ${lastBlock.by}` : ''}` : ''}.{/if}
          {#if askBlock}<div class="row ask"><input type="text" class="grow" bind:value={reason} placeholder="Why (kept in its history)" />
            <button class="btn2 danger" disabled={blocking} onclick={() => toggleBlock(true)}>{blocking ? 'Blocking…' : 'Block calls'}</button><button class="link" onclick={() => (askBlock = false)}>Cancel</button></div>{/if}</div>
        {#if canEdit}{#if blocked}<button class="btn2" disabled={blocking} onclick={() => toggleBlock(false)}>{blocking ? '…' : 'Unblock'}</button>{:else if !askBlock}<button class="btn2 danger" onclick={() => (askBlock = true)}>Block</button>{:else}<span></span>{/if}{:else}<span></span>{/if}</li>

      <li class:bad={keys !== null && !live.length}><span class="k">Keys</span>
        <div>{#if keys === null}…{:else if !live.length}<b>None</b> — apps can't call it.{:else}
          {#each live as k (k.id)}<span class="line"><b>{k.name}</b> <span class="mono">{k.prefix}…</span> — made by {k.created_by} {ago(k.created_at)}, {k.last_used_at ? `last used ${ago(k.last_used_at)}` : 'never used'}</span>{/each}{/if}</div>
        <button class="btn2" onclick={() => ongo('keys')}>{keys !== null && !live.length ? 'Make a key' : 'Manage keys'}</button></li>

      <li class:bad={(turns?.failed ?? 0) > 0}><span class="k">Answers</span>
        <div>{#if !turns}…{:else if !turns.turns}<b>None yet</b> — test it above.{:else}
          <b>{turns.turns}</b> turn{turns.turns === 1 ? '' : 's'}, <b>{turns.failed}</b> failed, {dur(turns.avg_ms)} on average, {turns.tool_calls} tool call{turns.tool_calls === 1 ? '' : 's'}.
          Last {turns.last_at ? ago(turns.last_at) : ''}{lastTurn ? ` in conversation ${lastTurn.session_id.split(':').slice(1).join(':')}` : ''}.{/if}</div>
        <button class="btn2" onclick={() => ongo('metrics')}>Metrics</button></li>

      <li><span class="k">Memory</span>
        <div>{agent.mode === 'persistent' ? 'Remembers across conversations' : agent.mode === 'ephemeral' ? 'Remembers nothing' : 'Remembers the conversation it is in'}.
          {#if memCounts} <b>{memTotal}</b> entr{memTotal === 1 ? 'y' : 'ies'}: {memCounts.working ?? 0} working, {memCounts.episodic ?? 0} episodic, {memCounts.semantic ?? 0} semantic, {memCounts.procedural ?? 0} procedural.{/if}</div>
        <span class="acts"><button class="btn2" onclick={() => ongo('memory')}>See it</button>{#if canEdit}<button class="btn2" onclick={() => ongo('tools')}>Change</button>{/if}</span></li>

      <li><span class="k">Model</span>
        <div><span class="mono">{f.llm || '—'}</span>{vars.temperature != null ? `, temperature ${vars.temperature}` : ''}{f.mcp.length ? ` · tools from ${f.mcp.map((m) => m.split('/').pop()).join(', ')}${f.tools ? ` (${f.tools.length} allowed)` : ''}` : ' · no tools'}.</div>
        {#if canEdit}<button class="btn2" onclick={() => ongo('tools')}>Change</button>{:else}<span></span>{/if}</li>

      <li><span class="k">Hooks</span>
        <div>{hookCount.before || hookCount.after ? `${hookCount.before} before each turn, ${hookCount.after} after.` : 'None — nothing runs around its turns.'}</div>
        <button class="btn2" onclick={() => ongo('hooks')}>{hookCount.before || hookCount.after ? 'Hooks' : 'Add a hook'}</button></li>

      <li><span class="k">Lineage</span>
        <div>{#if parent}{RELATION[parent.relation] ?? parent.relation} <b class="mono">{parent.id}{parent.version ? '@' + parent.version : ''}</b> — {RELATION_MEANS[parent.relation] ?? ''}.{:else}Made from <span class="mono">{agent.template_id}@{agent.template_version}</span>.{/if}
          {lineage?.descendants?.length ? ` ${lineage.descendants.length} agent${lineage.descendants.length === 1 ? '' : 's'} made from it.` : ' Nothing made from it yet.'}</div>
        <button class="btn2" onclick={() => ongo('lineage')}>Lineage</button></li>

      <li><span class="k">Changes</span>
        <div>{#if events === null}…{:else if !events.length}No changes recorded.{:else}Last: {describeEvent(events[0])}{events[0].by ? `, by ${events[0].by}` : ''}, {ago(events[0].at)}.{/if}</div>
        <button class="btn2" onclick={() => ongo('audit')}>History</button></li>
    </ul>
    {#if err}<p class="err">{err}</p>{/if}
  </div>

  <div class="next">
    <button class="card go" onclick={() => ongo('build')}><b>Build on it →</b><span class="dim">Its instructions, model, tools and hooks.</span></button>
    <button class="card go" onclick={() => ongo('secure')}><b>Secure it →</b><span class="dim">Its keys, who it is shared with, controls and audit.</span></button>
    <div class="card go"><b>Make another from it</b><span class="dim">
      <button class="link" onclick={onstudio}>Test in Studio</button> · <button class="link" onclick={() => onlifecycle('clone')}>Clone</button> · <button class="link" onclick={() => onlifecycle('fork')}>Fork</button> · <button class="link" onclick={() => onlifecycle('branch')}>Branch its memory</button></span></div>
  </div>
</section>

<style>
  .st { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; }
  .st li { display: grid; grid-template-columns: 90px 1fr auto; gap: 12px; align-items: start; padding: 10px 0; border-top: 1px solid var(--divider, var(--border)); font-size: 13px; color: var(--text-2); }
  .st li:first-child { border-top: 0; }
  .st .k { font-weight: 600; color: var(--text-1); padding-top: 2px; }
  .st li.bad .k, .st li.bad b { color: var(--err); }
  .st .line { display: block; }
  .st .acts { display: flex; gap: 6px; }
  .st .ask { margin-top: 8px; gap: 8px; }
  .term { border-radius: 10px; background: #111214; overflow: hidden; }
  .term-tabs { display: flex; align-items: center; gap: 2px; padding: 6px 8px; border-bottom: 1px solid #26282c; }
  .term-tabs button { border: 0; background: none; padding: 4px 10px; border-radius: 6px; font: 500 12px var(--font-sans); color: #9a9da3; cursor: pointer; }
  .term-tabs button.on { background: #26282c; color: #f4f4f5; }
  .term-tabs button:hover:not(.on) { color: #f4f4f5; }
  .term-tabs .term-copy { color: #c7c9cd; border: 1px solid #3a3d42; }
  .term pre { margin: 0; padding: 14px 16px; color: #f4f4f5; background: transparent; font: 12.5px/1.6 var(--font-mono); white-space: pre-wrap; overflow-wrap: anywhere; max-height: 420px; overflow-y: auto; }
  .grow { flex: 1 1 auto; }
  .eps { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; }
  .eps li { display: grid; grid-template-columns: 110px 1fr auto; gap: 4px 12px; align-items: center; padding: 10px 0; border-top: 1px solid var(--divider, var(--border)); }
  .eps li:first-child { border-top: 0; }
  .eps .lbl { font-weight: 600; font-size: 13px; color: var(--text-1); }
  .eps .url { overflow-wrap: anywhere; font-size: 12.5px; }
  .eps .sub { grid-column: 2 / 4; font-size: 12px; }
  .eps .bad, .bad { color: var(--err); }
  .mono { font-family: var(--font-mono); font-size: 0.92em; }
  pre.code { margin: 0; padding-right: 80px; white-space: pre-wrap; overflow-wrap: anywhere; }
  .next { display: grid; grid-template-columns: repeat(auto-fit, minmax(220px, 1fr)); gap: 10px; }
  .go { display: flex; flex-direction: column; gap: 4px; text-align: left; font: inherit; }
  button.go { cursor: pointer; }
  button.go:hover { border-color: var(--spec); }
</style>
