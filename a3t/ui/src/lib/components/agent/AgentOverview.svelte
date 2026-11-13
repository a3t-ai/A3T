<script lang="ts">
  /// Overview — in the order its maker works: its endpoints and token, a test
  /// here, the code to call it from an app (folded), then its status (open,
  /// keys, turns, memory, lineage — each opens where it is changed) and where to
  /// go next.
  import { agentSpaceApi, type AgentKey } from '$lib/services/api';
  import { factsOf, ago, dur, type AgentRec, type TemplateRec } from './agentData';
  import { insight, control, addresses, type TurnSummary, type Ancestor, type Descendant } from './controlPlane';
  import { copy, chatSnippet, type Lang } from './agentApi';
  import { keepKey, peekKey, dropKey } from './keyOnce';
  import AgentTryIt from './AgentTryIt.svelte';

  let { agent, templates, canEdit, ongo, onstudio, onlifecycle, onopen }: {
    agent: AgentRec & { description?: string; created_by?: string; created_at?: string; status?: string };
    templates: TemplateRec[];
    canEdit: boolean;
    ongo: (tab: string) => void;
    onstudio: () => void;
    onlifecycle: (kind: 'clone' | 'fork' | 'branch') => void;
    onopen: (id: string) => void;
  } = $props();

  const f = $derived(factsOf(agent, templates));
  const a = $derived(addresses(agent.id));
  const blocked = $derived(agent.status === 'blocked');
  const instructions = $derived.by(() => { try { const v = typeof agent.variables === 'string' ? JSON.parse(agent.variables || '{}') : (agent.variables ?? {}); return (v.system_prompt ?? v.instructions ?? '') as string; } catch { return ''; } });

  // What is true now: its turns, keys, memory and lineage.
  let turns = $state<TurnSummary | null>(null);
  let keys = $state<AgentKey[] | null>(null);
  let memTotal = $state<number | null>(null);
  let lineage = $state<{ ancestors: Ancestor[]; descendants: Descendant[] } | null>(null);
  let fresh = $state<{ name: string; key: string } | null>(null);
  let err = $state('');
  async function load(id: string) {
    const [t, k, m, l] = await Promise.all([insight.turns(id), agentSpaceApi.keys(id), insight.memory(id), control.lineage(id)]);
    if (agent.id !== id) return;
    turns = t.ok ? t.data.summary : null;
    keys = k.ok ? k.data?.keys ?? [] : null;
    memTotal = m.ok ? Object.values(m.data.counts ?? {}).reduce((x: number, y) => x + Number(y || 0), 0) : null;
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
    <div class="card-head"><h3>Status</h3><span class="grow"></span><button class="link" onclick={() => load(agent.id)}>Refresh</button></div>
    <div class="row facts">
      <button class="fact" class:bad={blocked} onclick={() => ongo('controls')}>{blocked ? 'blocked — calls refused' : 'open — answering calls'}</button>
      <button class="fact" class:bad={keys !== null && !live.length} onclick={() => ongo('keys')}>{keys === null ? 'keys …' : live.length ? `${live.length} key${live.length === 1 ? '' : 's'}` : 'no key'}</button>
      <button class="fact" onclick={() => ongo('metrics')}>{turns ? `${turns.turns} turn${turns.turns === 1 ? '' : 's'} · ${turns.failed} failed · ${turns.turns ? dur(turns.avg_ms) : '—'} avg` : 'turns …'}{turns?.last_at ? ` · last ${ago(turns.last_at)}` : ''}</button>
      <button class="fact" onclick={() => ongo('memory')}>{memTotal === null ? 'memory …' : `${memTotal} memory entr${memTotal === 1 ? 'y' : 'ies'}`}</button>
      <button class="fact" onclick={() => ongo('lineage')}>{parent ? `${parent.relation === 'template' ? 'from' : parent.relation} ${parent.id}${parent.version ? '@' + parent.version : ''}` : `from ${agent.template_id}`}{lineage?.descendants?.length ? ` · ${lineage.descendants.length} made from it` : ''}</button>
    </div>
    <p class="purpose" class:none={!(agent.description || instructions)}>{agent.description || instructions || 'No description yet.'}
      {#if canEdit}<button class="link" onclick={() => ongo('settings')}>{agent.description ? 'Edit' : 'Describe it'}</button>{/if}</p>
    <span class="dim">{agent.namespace} · {agent.created_by ? `owner ${agent.created_by}` : 'no owner'} · model <span class="mono">{f.llm || '—'}</span> · {agent.mode === 'persistent' ? 'remembers across conversations' : 'remembers one conversation'}</span>
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
  .term { border-radius: 10px; background: #111214; overflow: hidden; }
  .term-tabs { display: flex; align-items: center; gap: 2px; padding: 6px 8px; border-bottom: 1px solid #26282c; }
  .term-tabs button { border: 0; background: none; padding: 4px 10px; border-radius: 6px; font: 500 12px var(--font-sans); color: #9a9da3; cursor: pointer; }
  .term-tabs button.on { background: #26282c; color: #f4f4f5; }
  .term-tabs button:hover:not(.on) { color: #f4f4f5; }
  .term-tabs .term-copy { color: #c7c9cd; border: 1px solid #3a3d42; }
  .term pre { margin: 0; padding: 14px 16px; color: #f4f4f5; background: transparent; font: 12.5px/1.6 var(--font-mono); white-space: pre-wrap; overflow-wrap: anywhere; max-height: 420px; overflow-y: auto; }
  .hero { gap: 10px; }
  .purpose { margin: 0; font-size: 15px; line-height: 1.5; color: var(--text-1); }
  .purpose.none { color: var(--text-4); font-size: 14px; }
  .facts { gap: 6px; flex-wrap: wrap; align-items: center; font-size: 12.5px; }
  .fact { border: 1px solid var(--border); background: var(--bg-surface); border-radius: 999px; padding: 3px 10px; font: inherit; font-size: 12.5px; color: var(--text-2); cursor: pointer; }
  .fact:hover { border-color: var(--spec); color: var(--text-1); }
  .fact.bad { border-color: color-mix(in srgb, var(--err) 40%, var(--border)); color: var(--err); }
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
