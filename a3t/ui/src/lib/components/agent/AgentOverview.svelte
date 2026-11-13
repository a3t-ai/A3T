<script lang="ts">
  /// Overview — getting it working, in the order its maker does it: it is made;
  /// it answers here; there is a key for your app; your app has called it; its
  /// figures and memory are working. Each step says what is true now and offers
  /// the action that moves it on. Then what it is, where it came from, and where
  /// to go next (Build, Access).
  import { agentSpaceApi, type AgentKey } from '$lib/services/api';
  import { factsOf, ago, dur, type AgentRec, type TemplateRec } from './agentData';
  import { insight, control, addresses, type TurnSummary, type Ancestor, type Descendant } from './controlPlane';
  import { copy, chatSnippet } from './agentApi';
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
  const curl = $derived(chatSnippet('curl', agent.id).replace('$A3T_AGENT_KEY', fresh?.key ?? '$A3T_AGENT_KEY'));
  const parent = $derived(lineage?.ancestors?.[0]);
</script>

<section class="agt">
  <div class="card hero">
    <p class="purpose" class:none={!(agent.description || instructions)}>{agent.description || instructions || 'No description yet.'}
      {#if canEdit}<button class="link" onclick={() => ongo('settings')}>{agent.description ? 'Edit' : 'Describe it'}</button>{/if}</p>
    <div class="row facts">
      <button class="fact" class:bad={blocked} onclick={() => ongo('controls')}>{blocked ? 'Blocked — calls refused' : 'Open — answering calls'}</button>
      <button class="fact" class:bad={keys !== null && !live.length} onclick={() => ongo('keys')}>{keys === null ? 'keys …' : live.length ? `${live.length} key${live.length === 1 ? '' : 's'}` : 'no key'}</button>
      <button class="fact" onclick={() => ongo('metrics')}>{turns?.last_at ? `last answer ${ago(turns.last_at)}` : 'no answer yet'}</button>
      <button class="fact" onclick={() => ongo('lineage')}>{parent ? `${parent.relation === 'template' ? 'from' : parent.relation} ${parent.id}${parent.version ? '@' + parent.version : ''}` : `from ${agent.template_id}`}{lineage?.descendants?.length ? ` · ${lineage.descendants.length} made from it` : ''}</button>
      <span class="grow"></span>
      <span class="dim">{agent.namespace} · {agent.created_by ? `owner ${agent.created_by}` : 'no owner'}</span>
    </div>
  </div>

  {#if blocked}<div class="note bad">It is blocked: every call is refused. <button class="link" onclick={() => ongo('controls')}>Unblock it in Controls →</button></div>{/if}

  <div class="card">
    <div class="card-head"><h3>Get it working</h3><span>{[true, steps.answers, steps.key, steps.called, steps.figures].filter(Boolean).length} of 5 done</span></div>
    <ol class="steps">
      <li class="done"><span class="n">1</span><div><b>Made</b>
        <span class="dim">{agent.created_at ? ago(agent.created_at) : ''} from <span class="mono">{agent.template_id}@{agent.template_version}</span> in <span class="mono">{agent.namespace}</span> · model <span class="mono">{f.llm || '—'}</span> · {agent.mode === 'persistent' ? 'remembers across conversations' : 'remembers one conversation'}</span></div></li>

      <li class:done={steps.answers}><span class="n">2</span><div><b>Try it here</b>
        <span class="dim">{steps.answers ? `It has answered ${turns?.turns} time${turns?.turns === 1 ? '' : 's'}. Try another message, or move on.` : 'Send it a message: it answers as it will answer your app.'}</span>
        <AgentTryIt agentId={agent.id} onanswered={() => load(agent.id)} /></div></li>

      <li class:done={steps.key}><span class="n">3</span><div><b>A key for your app</b>
        {#if fresh}
          <span class="dim">Key <b>{fresh.name}</b> — copy it now, it is not shown again:</span>
          <div class="ep"><span class="verb">KEY</span><span class="path">{fresh.key}</span><button class="btn2 primary" onclick={() => cp('key', fresh!.key)}>{copied === 'key' ? 'Copied' : 'Copy'}</button></div>
          <button class="link" onclick={saved}>I have saved it</button>
        {:else if steps.key}
          <span class="dim">{live.length} key{live.length === 1 ? '' : 's'}: {live.map((k) => `${k.name} (${k.prefix}…)`).join(', ')}. <button class="link" onclick={() => ongo('keys')}>Manage keys →</button></span>
        {:else}
          <span class="dim">Every call needs a token. Apps use one of its keys, kept on their server.</span>
          {#if canEdit}<div><button class="btn2 primary" disabled={busy} onclick={makeKey}>{busy ? 'Making…' : 'Make a key'}</button></div>{/if}
        {/if}</div></li>

      <li class:done={steps.called}><span class="n">4</span><div><b>Call it from your app</b>
        <span class="dim">{used ? `Key ${used.name} last called it ${ago(used.last_used_at)}.` : 'Not called with a key yet. From your app or a terminal:'}</span>
        <pre class="code">{curl}<button class="btn2 copy" onclick={() => cp('curl', curl)}>{copied === 'curl' ? 'Copied' : 'Copy'}</button></pre>
        <span class="dim">TypeScript and Python, and the control plane: <button class="link" onclick={() => ongo('exposure')}>Exposure →</button> · <button class="link" onclick={() => load(agent.id)}>Check again</button></span></div></li>

      <li class:done={steps.figures}><span class="n">5</span><div><b>Its figures and memory</b>
        {#if turns}
          <span class="dim">{turns.turns} turn{turns.turns === 1 ? '' : 's'}, {turns.failed} failed, {turns.turns ? dur(turns.avg_ms) : '—'} on average · {memTotal ?? '…'} memory entr{memTotal === 1 ? 'y' : 'ies'}</span>
        {:else}<span class="dim">…</span>{/if}
        <span><button class="link" onclick={() => ongo('metrics')}>Metrics →</button> <button class="link" onclick={() => ongo('memory')}>Memory →</button></span></div></li>
    </ol>
    {#if err}<p class="err">{err}</p>{/if}
  </div>

  <div class="next">
    <button class="card go" onclick={() => ongo('build')}><b>Build on it →</b><span class="dim">Its model, tools and MCP servers, hooks, memory and lineage.</span></button>
    <button class="card go" onclick={() => ongo('secure')}><b>Secure it →</b><span class="dim">Its keys, who it is shared with, controls and audit.</span></button>
    <div class="card go"><b>Make another from it</b><span class="dim">
      <button class="link" onclick={onstudio}>Test in Studio</button> · <button class="link" onclick={() => onlifecycle('clone')}>Clone</button> · <button class="link" onclick={() => onlifecycle('fork')}>Fork</button> · <button class="link" onclick={() => onlifecycle('branch')}>Branch its memory</button></span></div>
  </div>
</section>

<style>
  .hero { gap: 10px; }
  .purpose { margin: 0; font-size: 15px; line-height: 1.5; color: var(--text-1); }
  .purpose.none { color: var(--text-4); font-size: 14px; }
  .facts { gap: 6px; flex-wrap: wrap; align-items: center; font-size: 12.5px; }
  .fact { border: 1px solid var(--border); background: var(--bg-surface); border-radius: 999px; padding: 3px 10px; font: inherit; font-size: 12.5px; color: var(--text-2); cursor: pointer; }
  .fact:hover { border-color: var(--spec); color: var(--text-1); }
  .fact.bad { border-color: color-mix(in srgb, var(--err) 40%, var(--border)); color: var(--err); }
  .grow { flex: 1 1 auto; }
  .steps { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; }
  .steps li { display: grid; grid-template-columns: 28px 1fr; gap: 10px; padding: 12px 0; border-top: 1px solid var(--divider, var(--border)); }
  .steps li:first-child { border-top: 0; }
  .steps li > div { display: flex; flex-direction: column; gap: 6px; min-width: 0; }
  .steps b { font-size: 13.5px; color: var(--text-1); }
  .n { width: 22px; height: 22px; border-radius: 50%; display: inline-flex; align-items: center; justify-content: center; font: 600 11.5px var(--font-mono); border: 1px solid var(--border-strong, var(--border)); color: var(--text-3); }
  .steps li.done .n { background: var(--green, #1a7158); border-color: var(--green, #1a7158); color: #fff; }
  .mono { font-family: var(--font-mono); font-size: 0.92em; }
  pre.code { margin: 0; padding-right: 80px; white-space: pre-wrap; overflow-wrap: anywhere; }
  .next { display: grid; grid-template-columns: repeat(auto-fit, minmax(220px, 1fr)); gap: 10px; }
  .go { display: flex; flex-direction: column; gap: 4px; text-align: left; font: inherit; }
  button.go { cursor: pointer; }
  button.go:hover { border-color: var(--spec); }
</style>
