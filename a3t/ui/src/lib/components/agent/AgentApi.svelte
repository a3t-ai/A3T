<script lang="ts">
  /// API — how an app calls this agent, with copy-ready code. Its data plane is
  /// POST /chat, streamed; the server reads the agent's own record from the
  /// session id, so the caller sends only the message. A webhook, when it has one,
  /// is a second way in.
  import { factsOf, type AgentRec, type TemplateRec } from './agentData';
  import { endpointUrl, chatSnippet, webhookSnippet, copy, apiBase, type Lang } from './agentApi';

  let { agent, templates }: {
    agent: AgentRec & { status?: string };
    templates: (TemplateRec & { spec_yaml: string })[];
  } = $props();

  const f = $derived(factsOf(agent, templates));
  const chatUrl = $derived(`${apiBase()}/chat`);
  const hook = $derived(f.endpoint ? endpointUrl(f.endpoint) : '');

  let lang = $state<Lang>('ts');
  const code = $derived(chatSnippet(lang, agent.id));
  let copied = $state('');
  async function cp(what: string, text: string) { if (await copy(text)) { copied = what; setTimeout(() => (copied = ''), 1500); } }
</script>

<section class="agt">
  <div class="card">
    <div class="card-head"><h3>Data plane</h3><span>chat, streamed as it is written</span></div>
    <div class="ep"><span class="verb">POST</span><span class="path">{chatUrl}</span>
      <button class="btn2" onclick={() => cp('url', chatUrl)}>{copied === 'url' ? 'Copied' : 'Copy'}</button></div>
    {#if agent.status === 'blocked'}<div class="note bad">Blocked: every call is refused until it is unblocked in Controls.</div>{/if}
    <dl class="kv">
      <dt>Auth</dt><dd><span class="mono">Authorization: Bearer &lt;key&gt;</span> — one of its keys (Keys tab), kept on the calling app's server; or a signed-in user's token. A call without one is refused. Every tool call is checked for the agent <em>and</em> whoever the key runs as.</dd>
      <dt>Body</dt><dd><span class="mono">{'{'} "session_id": "{agent.id}:&lt;conversation&gt;", "message": "…" {'}'}</span> — the same conversation id continues a conversation; a new one starts one.</dd>
      <dt>Agent</dt><dd>Read from its record by the session id: its model, instructions, tools and memory. The caller sends nothing else.</dd>
      <dt>Answer</dt><dd>Server-sent events (the AI SDK UI message stream): <span class="mono">text-delta</span> events as it writes, then <span class="mono">[DONE]</span>.</dd>
    </dl>
  </div>

  <div class="card">
    <div class="card-head"><h3>Call it</h3><span class="grow"></span>
      <div class="seg">{#each [['ts', 'TypeScript'], ['python', 'Python'], ['curl', 'curl']] as [k, l] (k)}<button class:on={lang === k} onclick={() => (lang = k as Lang)}>{l}</button>{/each}</div>
    </div>
    <pre class="code">{code}<button class="btn2 copy" onclick={() => cp('code', code)}>{copied === 'code' ? 'Copied' : 'Copy'}</button></pre>
  </div>

  {#if hook}
    <div class="card">
      <div class="card-head"><h3>Webhook</h3><span>for events: starts a run, returns no answer</span></div>
      <div class="ep"><span class="verb">POST</span><span class="path">{hook}</span>
        <button class="btn2" onclick={() => cp('hook', hook)}>{copied === 'hook' ? 'Copied' : 'Copy'}</button></div>
      <pre class="code">{webhookSnippet(hook, agent.id)}<button class="btn2 copy" onclick={() => cp('wh', webhookSnippet(hook, agent.id))}>{copied === 'wh' ? 'Copied' : 'Copy'}</button></pre>
    </div>
  {/if}
</section>

<style>
  .kv { grid-template-columns: 100px 1fr; font-size: 12.5px; }
  .kv em { font-style: normal; font-weight: 600; }
  pre.code { max-height: 420px; padding-right: 80px; }
</style>
