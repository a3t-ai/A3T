<script lang="ts">
  /// API — how a frontend calls this agent, as the server works today, with
  /// copy-ready code. What the API still lacks is said, and listed as Sample.
  import { factsOf, type AgentRec, type TemplateRec } from './agentData';
  import { readTemplate } from '$lib/components/templates/templateSpec';
  import { endpointUrl, requestBody, snippet, copy, type Lang } from './agentApi';

  let { agent, templates, ongo }: {
    agent: AgentRec;
    templates: (TemplateRec & { spec_yaml: string })[];
    ongo: (tab: string) => void;
  } = $props();

  const f = $derived(factsOf(agent, templates));
  const tpl = $derived(templates.find((t) => t.id === agent.template_id));
  const facts = $derived(readTemplate(tpl?.spec_yaml));
  const url = $derived(f.endpoint ? endpointUrl(f.endpoint) : '');
  const body = $derived(requestBody(agent.id, facts.inputs, { llm: f.llm, mcp: f.mcp, tools: f.tools }));
  const inputs = $derived(facts.inputs.length ? facts.inputs : [{ name: 'message', type: 'string', required: true, description: 'What the caller says' }]);

  let lang = $state<Lang>('ts');
  const code = $derived(url ? snippet(lang, url, body) : '');
  let copied = $state('');
  async function cp(what: string, text: string) { if (await copy(text)) { copied = what; setTimeout(() => (copied = ''), 1500); } }
</script>

<section class="agt">
  {#if !url}
    <div class="note warn">It has no endpoint. Give it one in <button class="link" onclick={() => ongo('config')}>Config</button> (<span class="mono">endpoint:</span>).</div>
  {:else}
    <div class="card">
      <div class="card-head"><h3>Endpoint</h3><span>prod — what apps call</span></div>
      <div class="ep"><span class="verb">POST</span><span class="path">{url}</span>
        <button class="btn2" onclick={() => cp('url', url)}>{copied === 'url' ? 'Copied' : 'Copy'}</button></div>
      <dl class="kv">
        <dt>Auth</dt><dd><span class="mono">Authorization: Bearer &lt;token&gt;</span> — a signed-in user's token; the call then runs as that user, and every tool call is checked for the agent <em>and</em> the user.
          <span class="warnline">Webhook paths are public: a call without a token is accepted too and runs without a user (SRV-16).</span></dd>
        <dt>Conversation</dt><dd><span class="mono">session_id: "{agent.id}:&lt;your id&gt;"</span> — send the same id to continue a conversation; a new id starts one.</dd>
        <dt>Answer</dt><dd>Not in the response: it starts a run. Poll it until <span class="mono">completed</span>; <span class="mono">output.content</span> is the answer.</dd>
      </dl>
    </div>

    <div class="card">
      <div class="card-head"><h3>Request</h3><span>from its template's <span class="mono">input</span> schema</span></div>
      <table class="tbl">
        <thead><tr><th>Field</th><th>Type</th><th>What it is</th></tr></thead>
        <tbody>
          {#each inputs as i (i.name)}
            <tr><td class="mono">{i.name}{i.required ? '' : '?'}</td><td class="mono">{i.type}</td><td>{i.description ?? ''}</td></tr>
          {/each}
          <tr><td class="mono">session_id</td><td class="mono">string</td><td>The conversation, prefixed with the agent's id</td></tr>
          {#if f.llm}<tr class="today"><td class="mono">llm_resource</td><td class="mono">string</td><td>Its model — <b>the caller sends it today</b> (see below)</td></tr>{/if}
          {#if f.mcp.length}<tr class="today"><td class="mono">mcp_servers</td><td class="mono">string[]</td><td>Its tool servers — <b>sent by the caller today</b></td></tr>{/if}
          {#if f.tools}<tr class="today"><td class="mono">enabled_tools</td><td class="mono">string[]</td><td>Its tool allow-list — <b>sent by the caller today</b></td></tr>{/if}
        </tbody>
      </table>
      {#if facts.outputs.length}
        <h4>Answer fields</h4>
        <table class="tbl">
          <thead><tr><th>Field</th><th>Type</th><th>What it is</th></tr></thead>
          <tbody>{#each facts.outputs as o (o.name)}<tr><td class="mono">{o.name}</td><td class="mono">{o.type}</td><td>{o.description ?? ''}</td></tr>{/each}</tbody>
        </table>
      {/if}
    </div>

    <div class="card">
      <div class="card-head"><h3>Call it</h3><span class="grow"></span>
        <div class="seg">{#each [['ts', 'TypeScript'], ['python', 'Python'], ['curl', 'curl']] as [k, l] (k)}<button class:on={lang === k} onclick={() => (lang = k as Lang)}>{l}</button>{/each}</div>
      </div>
      <pre class="code">{code}<button class="btn2 copy" onclick={() => cp('code', code)}>{copied === 'code' ? 'Copied' : 'Copy'}</button></pre>
    </div>

    <div class="note bad">
      <b>Today the server does not look the agent up when it is called.</b> Its model and tools only apply because the caller sends them (above), and its
      saved instructions are not sent by any caller — whether they apply depends on its serving flow, <span class="mono">{f.target || '—'}</span> (SRV-14).
      A frontend should not have to carry an agent's settings; the fix is on the server.
    </div>

    <div class="card">
      <div class="card-head"><h3>Coming</h3><span class="tag sample">Sample</span></div>
      <ul class="plain">
        <li><b>One call, one answer</b> — <span class="mono">POST …/agents/{agent.name}/messages</span> returns the answer, or streams it (SSE, AI SDK compatible).</li>
        <li><b>App keys</b> — a key per calling app, its own rate limit, revocable, instead of a user's token; calls without one refused.</li>
        <li><b>Allowed origins</b> — which web apps may call it from the browser (CORS).</li>
        <li><b>Pin a release</b> — <span class="mono">?release=v3</span>, or dev/prod endpoints per environment.</li>
      </ul>
      <p class="dim">SRV-15 in the limitations doc.</p>
    </div>
  {/if}
</section>

<style>
  .kv { grid-template-columns: 100px 1fr; font-size: 12.5px; }
  .kv em { font-style: normal; font-weight: 600; }
  .warnline { display: block; margin-top: 4px; color: var(--warn, #b45309); }
  tr.today td { background: color-mix(in srgb, var(--warn-bg, #fbf0e0) 60%, transparent); }
  .plain { margin: 0; padding-left: 18px; display: flex; flex-direction: column; gap: 6px; font-size: 12.5px; }
  pre.code { max-height: 420px; padding-right: 80px; }
</style>
