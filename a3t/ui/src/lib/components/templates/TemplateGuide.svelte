<script lang="ts">
  /// A template as its users meet it, in order: what it is (About), how to use
  /// it (How to use), does it work (Try it), and taking it live (Production).
  /// Authors' views — the designer and its settings — stay on the Templates page.
  ///
  /// Facts come from the template's spec (templateSpec.ts) and its agents;
  /// nothing is made up. Try it runs through an agent built from the template:
  /// a template has no endpoint of its own, and a changed prompt cannot be tried
  /// until the server accepts one (UI limitations, SRV-10) — it says so.
  import { navigateTo, navigateToDetail } from '$lib/stores/panels.svelte.ts';
  import { webhookChat } from '$lib/services/api';
  import { readTemplate } from './templateSpec';
  import { runsOf, addRun } from './tryRuns.svelte.ts';

  let { view, template, agents = [], agentsLoading = false, onview }: {
    view: 'about' | 'use' | 'try' | 'production';
    template: any;
    agents?: any[];
    agentsLoading?: boolean;
    /// Move to another tab of the template (About ↔ Design, …).
    onview?: (id: string) => void;
  } = $props();

  const facts = $derived(readTemplate(template?.spec_yaml));
  const title = $derived(facts.title || template?.name || 'Template');
  const createAgent = () => navigateTo('agents', 'Agents', { createFromTemplate: template.id });

  // ── Try it ──
  let tryAgent = $state('');
  let tryMessage = $state('Hello — what can you do for me?');
  let tryModel = $state('');
  let running = $state(false);
  // Kept per template for the visit, so they survive switching tabs.
  const results = $derived(runsOf(template?.id));
  const chosen = $derived(agents.find((a) => a.id === tryAgent) ?? agents[0]);
  async function run() {
    const a = chosen;
    if (!a || !tryMessage.trim() || running) return;
    running = true;
    const t0 = performance.now();
    const body: Record<string, unknown> = { message: tryMessage.trim(), session_id: `${a.id}:tryit-${Date.now()}` };
    if (tryModel.trim()) body.llm_resource = tryModel.trim();
    const r = await webhookChat(`/agent/${a.id}/chat`, body, { maxPolls: 90, pollInterval: 1000 });
    const ms = Math.round(performance.now() - t0);
    const d: any = r.data;
    const raw = d?.content ?? d?.output?.content ?? d?.output ?? d?.agent_output ?? d;
    const text = r.ok ? (typeof raw === 'string' ? raw : JSON.stringify(raw, null, 2)) : (r.error ?? 'The agent did not answer');
    addRun(template.id, { ok: r.ok && !!String(text).trim() && d?.stopped_reason !== 'error', text: String(text || '(No answer.)'), ms, toolCalls: d?.tool_calls_made, stopped: d?.stopped_reason, input: tryMessage.trim(), agent: a.name ?? a.id, at: Date.now() });
    running = false;
  }
  const lastOk = $derived(results.some((r) => r.ok));

  // ── Production: what is in place, each linked to where it is done ──
  const checks = $derived([
    { ok: !!(facts.summary || facts.description), label: 'Says what it does', hint: 'A summary and description in its spec, so people can choose it.', go: 'yaml' },
    { ok: !!facts.agent.model, label: 'Has a model', hint: facts.agent.model ? `Uses ${facts.agent.model}.` : 'Agents built from it must choose one.', go: 'yaml' },
    { ok: facts.hooks.length > 0, label: 'Has guardrails or hooks', hint: facts.hooks.length ? `Hooks: ${facts.hooks.join(', ')}.` : 'No hook runs before or after its steps.', go: 'yaml' },
    { ok: !!facts.agent.memory, label: 'Has memory set up', hint: facts.agent.memory ? `Remembers under ${facts.agent.memory}.` : 'Its agents start fresh every time.', go: 'yaml' },
    { ok: lastOk, label: 'Worked when tried', hint: lastOk ? 'A Try it run answered, this session.' : 'Run it once in Try it.', go: 'try' },
    { ok: agents.length > 0, label: 'Has an agent', hint: agents.length ? `${agents.length} agent${agents.length > 1 ? 's' : ''} built from it.` : 'Create one to deploy it.', go: 'use' },
  ]);
  const ready = $derived(checks.filter((c) => c.ok).length);

  async function copy(text: string) { try { await navigator.clipboard.writeText(text); } catch { /* not allowed here */ } }
  const sampleAgent = $derived(agents[0]?.id ?? '<agent-id>');
  const curl = $derived(`curl -X POST "$A3T_URL/agent/${sampleAgent}/chat" \\\n  -H "Authorization: Bearer $A3T_TOKEN" -H "Content-Type: application/json" \\\n  -d '{"message": "Hello", "session_id": "my-session"}'`);
</script>

<div class="tg">
  {#if facts.parseError}<div class="tg-warn">Its spec does not parse, so some of this is missing: {facts.parseError}</div>{/if}

  {#if view === 'about'}
    <section class="tg-hero">
      <h2>{title}</h2>
      {#if facts.summary}<p class="tg-summary">{facts.summary}</p>{/if}
      {#if facts.description}
        <!-- Specs wrap their text; single line breaks join, blank lines part paragraphs. -->
        {#each facts.description.trim().split(/\n\s*\n/) as para}<p class="tg-desc">{para.replace(/\s*\n\s*/g, ' ')}</p>{/each}
      {/if}
      {#if !facts.summary && !facts.description}
        <div class="tg-empty">
          <span>Its spec has no summary or description yet — add <span class="mono">document.summary</span> so people know what it is for.</span>
          <button class="tg-btn" onclick={() => onview?.('yaml')}>Open the design</button>
        </div>
      {/if}
    </section>

    <section class="tg-card">
      <h3>What it is made of</h3>
      <div class="tg-chips">
        <span class="tg-chip"><b>Model</b>{facts.agent.model ?? 'chosen per agent'}</span>
        <span class="tg-chip"><b>Tools</b>{facts.agent.mcpServers.length + facts.agent.tools.length ? [...facts.agent.mcpServers, ...facts.agent.tools].join(', ') : 'none'}</span>
        <span class="tg-chip"><b>Memory</b>{facts.agent.memory ? 'yes' : 'none'}</span>
        <span class="tg-chip"><b>Hooks</b>{facts.hooks.length ? facts.hooks.join(', ') : 'none'}</span>
        {#if facts.agent.maxIterations}<span class="tg-chip"><b>Step limit</b>{facts.agent.maxIterations}</span>{/if}
      </div>
    </section>

    <div class="tg-two">
      <section class="tg-card">
        <h3>It takes</h3>
        {#if facts.inputs.length}
          <ul class="tg-fields">{#each facts.inputs as f (f.name)}<li><span class="mono">{f.name}</span><small>{f.type}{f.required ? ' · required' : ''}</small>{#if f.description}<em>{f.description}</em>{/if}</li>{/each}</ul>
        {:else}<p class="tg-dim">A message.</p>{/if}
      </section>
      <section class="tg-card">
        <h3>It gives back</h3>
        {#if facts.outputs.length}
          <ul class="tg-fields">{#each facts.outputs as f (f.name)}<li><span class="mono">{f.name}</span><small>{f.type}</small>{#if f.description}<em>{f.description}</em>{/if}</li>{/each}</ul>
        {:else}<p class="tg-dim">A reply.</p>{/if}
      </section>
    </div>

    <dl class="tg-kv">
      <div><dt>Version</dt><dd>{template.activeVersion ?? template.version ?? facts.version ?? '—'}</dd></div>
      <div><dt>Project</dt><dd>{template.namespace ?? facts.namespace ?? '—'}</dd></div>
      <div><dt>By</dt><dd>{template.created_by || '—'}</dd></div>
      <div><dt>Agents from it</dt><dd><button class="link" onclick={() => onview?.('production')}>{agentsLoading ? '…' : agents.length}</button></dd></div>
    </dl>
    <div class="tg-next"><button class="tg-btn primary" onclick={() => onview?.('try')}>Try it</button><button class="tg-btn" onclick={() => onview?.('use')}>How to use it</button></div>

  {:else if view === 'use'}
    <section class="tg-card">
      <h3>Make an agent from it</h3>
      <p class="tg-dim">A template is the blueprint; an agent is a working copy with its own choices. You set:</p>
      <ol class="tg-steps">
        <li><b>A model</b><span>{facts.agent.model ? `Defaults to ${facts.agent.model}; pick another if you like.` : 'Required — the template does not set one.'}</span></li>
        <li><b>Tools</b><span>{facts.agent.mcpServers.length ? `It comes with ${facts.agent.mcpServers.join(', ')}; add or remove.` : 'Optional — connect MCP servers it may call.'}</span></li>
        <li><b>The prompt</b><span>Optional — it starts from the template's; adjust it for your case.</span></li>
        <li><b>How it is started</b><span>Chat, an HTTP endpoint, or as a tool other agents call (MCP).</span></li>
      </ol>
      <div><button class="tg-btn primary" onclick={createAgent}>Create an agent from it</button></div>
    </section>
    <section class="tg-card">
      <h3>Call it</h3>
      <p class="tg-dim">Once you have an agent{agents.length ? ` — for example ${agents[0].name}` : ''}:</p>
      <div class="tg-way"><b>Chat</b><span>Talk to it in AI Chat, choosing the agent in the message box.</span><button class="tg-btn" onclick={() => navigateTo('chat', 'AI Chat')}>Open AI Chat</button></div>
      <div class="tg-way"><b>HTTP</b><span>Send a message to its endpoint.</span><button class="tg-btn" onclick={() => copy(curl)}>Copy</button></div>
      <pre class="tg-code">{curl}</pre>
      <div class="tg-way"><b>From other agents</b><span>Expose it as a tool (MCP) when you create the agent; other agents then call it by name.</span></div>
    </section>

  {:else if view === 'try'}
    {#if agentsLoading}
      <!-- the header bar shows loading -->
    {:else if agents.length === 0}
      <section class="tg-card">
        <h3>Try it</h3>
        <p class="tg-dim">A template runs through an agent built from it. Create one to try it — you can keep it as your test agent.</p>
        <div><button class="tg-btn primary" onclick={createAgent}>Create a test agent</button></div>
      </section>
    {:else}
      <section class="tg-card">
        <h3>Quick test</h3>
        <div class="tg-form">
          {#if agents.length > 1}
            <label><span>Through</span>
              <select bind:value={tryAgent}>{#each agents as a (a.id)}<option value={a.id}>{a.name ?? a.id}</option>{/each}</select>
            </label>
          {:else}<p class="tg-dim">Through <b>{agents[0].name ?? agents[0].id}</b>, an agent built from it.</p>{/if}
          <label><span>Your message</span><textarea rows="3" bind:value={tryMessage}></textarea></label>
          <label><span>Model (optional)</span><input type="text" bind:value={tryModel} placeholder={facts.agent.model ?? 'project/llm-resource'} /></label>
          <div class="tg-next"><button class="tg-btn primary" onclick={run} disabled={running || !tryMessage.trim()}>{running ? 'Running…' : 'Run'}</button></div>
        </div>
      </section>

      {#if results.length}
        <div class="tg-results" class:two={results.length > 1}>
          {#each results.slice(0, 2) as r, i (r.at)}
            <section class="tg-card tg-result" class:bad={!r.ok}>
              <header><span class="tg-verdict">{r.ok ? 'Worked' : 'Did not work'}</span><small>{i === 0 ? 'Latest' : 'Before'} · {(r.ms / 1000).toFixed(1)} s{r.toolCalls != null ? ` · ${r.toolCalls} tool call${r.toolCalls === 1 ? '' : 's'}` : ''}{r.stopped ? ` · ${r.stopped.replace('_', ' ')}` : ''}</small></header>
              <p class="tg-in">“{r.input}”</p>
              <pre class="tg-out">{r.text}</pre>
            </section>
          {/each}
        </div>
      {/if}

      <section class="tg-card">
        <h3>Its prompt</h3>
        {#if facts.agent.systemPrompt}<pre class="tg-out">{facts.agent.systemPrompt}</pre>{:else}<p class="tg-dim">It sets no prompt; agents bring their own.</p>{/if}
        <p class="tg-dim">Trying a changed prompt here needs the server to accept one per run (SRV-10). Change it in Design, or in an agent built from it.</p>
      </section>
    {/if}

  {:else if view === 'production'}
    <section class="tg-card">
      <header class="tg-head"><h3>Ready for production?</h3><span class="tg-score">{ready} of {checks.length}</span></header>
      <ul class="tg-checks">
        {#each checks as c (c.label)}
          <li class:ok={c.ok}>
            <span class="tg-mark" aria-hidden="true">{c.ok ? '✓' : '○'}</span>
            <span><b>{c.label}</b><small>{c.hint}</small></span>
            {#if !c.ok}<button class="link" onclick={() => onview?.(c.go)}>Fix</button>{/if}
          </li>
        {/each}
      </ul>
    </section>
    <section class="tg-card">
      <h3>Take it live</h3>
      <ol class="tg-steps">
        <li><b>Create an agent from it</b><span>Choose its model, tools and how it is started.</span></li>
        <li><b>Deploy it</b><span>A canvas puts the agent on a runtime, in dev first.</span></li>
        <li><b>Promote it</b><span>From dev to staging to prod, in the project's Environments.</span></li>
      </ol>
      <div class="tg-next"><button class="tg-btn primary" onclick={createAgent}>Create an agent</button><button class="tg-btn" onclick={() => navigateTo('canvas', 'Canvas', { create: Date.now() })}>New canvas</button></div>
    </section>
    <section class="tg-card">
      <h3>Agents built from it <span class="tg-count">{agents.length}</span></h3>
      {#if agentsLoading}
        <!-- the header bar shows loading -->
      {:else if agents.length === 0}<p class="tg-dim">None yet.</p>
      {:else}
        <ul class="tg-agents">
          {#each agents as a (a.id)}
            <li><button onclick={() => navigateToDetail('agents', 'Agents', a.name ?? a.id)}><span class="mono">{a.name ?? a.id}</span><small>{a.mode ?? ''}</small><span class="tg-st">{a.status ?? ''}</span></button></li>
          {/each}
        </ul>
      {/if}
    </section>
  {/if}
</div>

<style>
  .tg { display: flex; flex-direction: column; gap: 14px; padding: 18px 20px 28px; max-width: 760px; }
  .tg-warn { padding: 9px 12px; border: 1px solid var(--warn, #b45309); border-radius: 8px; background: var(--warn-bg, #fbf0e0); font-size: 12.5px; color: var(--text-2); }
  .tg-hero h2 { margin: 0; font: 400 26px/1.25 var(--font-serif); color: var(--text-1); }
  .tg-summary { margin: 8px 0 0; font-size: 16px; color: var(--text-1); line-height: 1.45; }
  .tg-desc { margin: 8px 0 0; font-size: 13.5px; color: var(--text-2); line-height: 1.6; max-width: 68ch; }
  .tg-empty { margin-top: 10px; display: flex; align-items: center; gap: 14px; flex-wrap: wrap; padding: 12px 14px; border: 1px dashed var(--border-strong, #d9d4c6); border-radius: 10px; font-size: 13px; color: var(--text-3); }
  .tg-empty > span { flex: 1 1 260px; }
  .tg-card { display: flex; flex-direction: column; gap: 10px; padding: 16px 18px; border: 1px solid var(--border); border-radius: 12px; background: var(--bg-surface); }
  .tg-card h3 { margin: 0; font: 400 17px var(--font-serif); color: var(--text-1); display: flex; align-items: center; gap: 8px; }
  .tg-head { display: flex; align-items: baseline; justify-content: space-between; }
  .tg-score { font: 500 12px var(--font-mono); color: var(--text-3); }
  .tg-chips { display: flex; flex-wrap: wrap; gap: 8px; }
  .tg-chip { display: inline-flex; gap: 6px; align-items: baseline; padding: 5px 10px; border: 1px solid var(--border); border-radius: 999px; background: var(--bg-subtle); font-size: 12.5px; color: var(--text-1); }
  .tg-chip b { font: 500 10.5px var(--font-mono); letter-spacing: 0.05em; text-transform: uppercase; color: var(--text-4); }
  .tg-two { display: grid; grid-template-columns: repeat(auto-fit, minmax(240px, 1fr)); gap: 14px; }
  .tg-fields { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 8px; }
  .tg-fields li { display: flex; flex-wrap: wrap; gap: 4px 8px; align-items: baseline; font-size: 13px; }
  .tg-fields small { color: var(--text-4); font-size: 11.5px; }
  .tg-fields em { flex-basis: 100%; font-style: normal; font-size: 12px; color: var(--text-3); }
  .tg-dim { margin: 0; font-size: 13px; color: var(--text-3); }
  .tg-kv { display: grid; grid-template-columns: repeat(auto-fit, minmax(120px, 1fr)); gap: 14px; margin: 0; padding: 4px 2px; }
  .tg-kv dt { font: 500 10.5px var(--font-mono); letter-spacing: 0.06em; text-transform: uppercase; color: var(--text-4); margin-bottom: 4px; }
  .tg-kv dd { margin: 0; font-size: 13.5px; color: var(--text-1); }
  .tg-next { display: flex; gap: 8px; flex-wrap: wrap; }
  .tg-btn { height: 32px; padding: 0 14px; border-radius: 8px; border: 1px solid var(--border); background: var(--bg-surface); font: 500 13px var(--font-sans); color: var(--text-1); cursor: pointer; }
  .tg-btn:hover:not(:disabled) { border-color: var(--spec); }
  .tg-btn.primary { background: var(--green, #1a7158); border-color: var(--green, #1a7158); color: #fff; }
  .tg-btn:disabled { opacity: 0.55; cursor: default; }
  .tg-steps { margin: 0; padding-left: 20px; display: flex; flex-direction: column; gap: 8px; }
  .tg-steps li { font-size: 13px; color: var(--text-1); }
  .tg-steps li span { display: block; color: var(--text-3); font-size: 12.5px; }
  .tg-way { display: flex; align-items: center; gap: 10px; flex-wrap: wrap; font-size: 13px; }
  .tg-way b { flex: 0 0 120px; }
  .tg-way span { flex: 1 1 220px; color: var(--text-3); }
  .tg-code, .tg-out { margin: 0; padding: 10px 12px; border-radius: 8px; background: var(--bg-subtle); border: 1px solid var(--border); font: 12px/1.5 var(--font-mono); color: var(--text-1); white-space: pre-wrap; overflow-x: auto; }
  .tg-out { max-height: 260px; overflow-y: auto; }
  .tg-form { display: flex; flex-direction: column; gap: 10px; }
  .tg-form label { display: flex; flex-direction: column; gap: 5px; font-size: 12.5px; color: var(--text-2); text-transform: none; letter-spacing: 0; font-weight: 500; }
  .tg-results { display: grid; gap: 14px; }
  .tg-results.two { grid-template-columns: repeat(auto-fit, minmax(260px, 1fr)); }
  .tg-result header { display: flex; align-items: baseline; justify-content: space-between; gap: 8px; }
  .tg-verdict { font: 600 11px var(--font-mono); letter-spacing: 0.06em; text-transform: uppercase; color: var(--green, #15803d); }
  .tg-result.bad .tg-verdict { color: var(--err, #b42318); }
  .tg-result small { font-size: 12px; color: var(--text-3); }
  .tg-in { margin: 0; font-size: 12.5px; color: var(--text-3); font-style: italic; }
  .tg-checks { list-style: none; margin: 0; padding: 0; }
  .tg-checks li { display: flex; align-items: center; gap: 12px; padding: 9px 2px; border-bottom: 1px solid var(--divider, var(--border)); }
  .tg-checks li:last-child { border-bottom: 0; }
  .tg-checks li > span:nth-child(2) { flex: 1 1 auto; display: flex; flex-direction: column; gap: 2px; }
  .tg-checks b { font-size: 13px; color: var(--text-1); }
  .tg-checks small { font-size: 12px; color: var(--text-3); }
  .tg-mark { flex: 0 0 22px; width: 22px; height: 22px; border-radius: 50%; display: grid; place-items: center; border: 1px solid var(--border); color: var(--text-4); font-size: 12px; }
  .tg-checks li.ok .tg-mark { background: var(--green, #1a7158); border-color: var(--green, #1a7158); color: #fff; }
  .tg-count { font: 500 10.5px var(--font-mono); color: var(--text-4); background: var(--bg-subtle); border-radius: 4px; padding: 1px 6px; }
  .tg-agents { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 6px; }
  .tg-agents button { width: 100%; display: flex; align-items: center; gap: 10px; padding: 8px 12px; border: 1px solid var(--border); border-radius: 8px; background: var(--bg-surface); cursor: pointer; font: inherit; font-size: 13px; text-align: left; }
  .tg-agents button:hover { border-color: var(--spec); }
  .tg-agents .mono { flex: 1 1 auto; }
  .tg-agents small { color: var(--text-4); }
  .tg-st { font: 500 10.5px var(--font-mono); letter-spacing: 0.05em; text-transform: uppercase; color: var(--text-3); }
  .link { border: 0; background: none; padding: 0; color: var(--spec); cursor: pointer; font: inherit; }
  .mono { font-family: var(--font-mono); }
</style>
