<script lang="ts">
  /// A template as people meet it: a blueprint to understand, then copy or use.
  /// About (what it is, and the choice: clone and edit, or use an agent already
  /// built from it) and Versions (its releases, its copies, its agents — and,
  /// for a copy, its base). The page's Design shows how it is built — read-only
  /// on a base template, which is never edited in place.
  ///
  /// Facts come from the spec (templateSpec.ts) and the registry; nothing is
  /// made up.
  import { navigateTo, navigateToDetail } from '$lib/stores/panels.svelte.ts';
  import { readTemplate } from './templateSpec';

  let { view, template, readOnly, agents = [], agentsLoading = false, allTemplates = [], projects = [], copyRequest = 0, oncopy, onview, onopentemplate }: {
    view: 'about' | 'agents' | 'versions';
    template: any;
    /// A base template (not yours): read-only, copied rather than edited.
    readOnly: boolean;
    agents?: any[];
    agentsLoading?: boolean;
    allTemplates?: any[];
    /// Projects you may create a copy in.
    projects?: { name: string; label: string }[];
    /// A new value opens the copy form (the header's Clone and edit).
    copyRequest?: number;
    oncopy?: (name: string, namespace: string) => Promise<string | null>;
    onview?: (id: string) => void;
    onopentemplate?: (t: any) => void;
  } = $props();

  const facts = $derived(readTemplate(template?.spec_yaml));
  const title = $derived(facts.title || template?.name || 'Template');
  const ref = $derived(`${template?.namespace ?? facts.namespace ?? ''}/${template?.name ?? ''}`);
  const createAgent = () => navigateTo('agents', 'Agents', { createFromTemplate: template.id });

  // Copies made from this template, and — for a copy — the template it came from.
  const copies = $derived(allTemplates.filter((t) => t !== template && String(readTemplate(t.spec_yaml).copiedFrom ?? '').startsWith(`${ref}@`)));
  const base = $derived.by(() => {
    if (!facts.copiedFrom) return null;
    const [path, version] = facts.copiedFrom.split('@');
    const [ns, name] = path.split('/');
    const t = allTemplates.find((x) => x.name === name && (x.namespace ?? '') === ns);
    const current = t ? String(t.activeVersion ?? t.version) : '';
    return { path, version, template: t, newer: t && version && current !== version ? current : null };
  });

  /// An agent's description, or — agent records often have none — the start of
  /// its prompt, said to be that.
  function describe(a: any): { text: string; fromPrompt: boolean } {
    if (a?.description) return { text: String(a.description), fromPrompt: false };
    let v: any = a?.variables;
    if (typeof v === 'string') { try { v = JSON.parse(v); } catch { v = null; } }
    const p = String(v?.system_prompt ?? '').trim();
    return p ? { text: p.length > 180 ? `${p.slice(0, 180).trimEnd()}…` : p, fromPrompt: true } : { text: '', fromPrompt: false };
  }

  // ── Clone and edit ──
  let copying = $state(false);
  let copyName = $state('');
  let copyProject = $state('');
  let copyBusy = $state(false);
  let copyError = $state('');
  function freeName(ns: string) {
    const taken = new Set(allTemplates.filter((t) => (t.namespace ?? '') === ns).map((t) => t.name));
    let n = `${template.name}-copy`;
    for (let i = 2; taken.has(n); i++) n = `${template.name}-copy-${i}`;
    return n;
  }
  function openCopy() {
    copyProject = projects[0]?.name ?? '';
    copyName = freeName(copyProject);
    copyError = '';
    copying = true;
  }
  let seenRequest = 0;
  $effect(() => { if (copyRequest && copyRequest !== seenRequest) { seenRequest = copyRequest; openCopy(); } });
  async function doCopy() {
    if (!oncopy || !copyName.trim() || !copyProject) return;
    if (!/^[a-z0-9][a-z0-9-]{0,62}$/.test(copyName.trim())) { copyError = 'A name is lowercase letters, digits and dashes.'; return; }
    if (allTemplates.some((t) => t.name === copyName.trim() && (t.namespace ?? '') === copyProject)) { copyError = `${copyProject} already has a template ${copyName.trim()}.`; return; }
    copyBusy = true; copyError = '';
    copyError = (await oncopy(copyName.trim(), copyProject)) ?? '';
    copyBusy = false;
    if (!copyError) copying = false;
  }
</script>

{#snippet agentRows(list: any[])}
  <ul class="tg-agents">
    {#each list as a (a.id)}
      <li><button onclick={() => navigateToDetail('agents', 'Agents', a.name ?? a.id)}><span class="mono">{a.name ?? a.id}</span><small>{a.namespace ?? ''}{a.mode ? ` · ${a.mode}` : ''}</small><span class="tg-st">{a.status ?? ''}</span></button></li>
    {/each}
  </ul>
{/snippet}

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
        <p class="tg-dim">{readOnly ? 'It has no summary or description.' : 'No summary or description yet — add document.summary in Code so people know what it is for.'}</p>
      {/if}
      {#if base}<p class="tg-lineage">Your copy of <button class="link" onclick={() => base?.template && onopentemplate?.(base.template)}>{base.path}</button> v{base.version}{#if base.newer} — <b>v{base.newer} is out</b>{/if}</p>{/if}
    </section>

    <!-- The choice: change it (as a copy), or use an agent already built from it. -->
    {#if readOnly}
      <div class="tg-choice">
        <section class="tg-card tg-option" class:on={copying}>
          <h3>Clone and edit</h3>
          <p class="tg-dim">Make your own copy in one of your projects and change it there. This template stays as it is.</p>
          {#if !copying}
            <div><button class="tg-btn primary" onclick={openCopy} disabled={!projects.length}>Clone and edit</button></div>
            {#if !projects.length}<p class="tg-dim">You need a project you can write to.</p>{/if}
          {:else}
            <div class="tg-form">
              <label><span>Name of your copy</span><input type="text" class="mono" bind:value={copyName} /></label>
              <label><span>In project</span>
                <select bind:value={copyProject} onchange={() => (copyName = freeName(copyProject))}>{#each projects as p (p.name)}<option value={p.name}>{p.label}</option>{/each}</select>
              </label>
              {#if copyError}<p class="tg-err">{copyError}</p>{/if}
              <div class="tg-next"><button class="tg-btn primary" onclick={doCopy} disabled={copyBusy || !copyName.trim()}>{copyBusy ? 'Copying…' : 'Make my copy'}</button><button class="tg-btn" onclick={() => (copying = false)}>Cancel</button></div>
            </div>
          {/if}
        </section>
        <section class="tg-card tg-option">
          <h3>Use an agent built from it</h3>
          {#if agentsLoading}
            <!-- the header bar shows loading -->
          {:else if agents.length}
            <p class="tg-dim">{agents.length} ready to use — open one to chat with it or deploy it.</p>
            {@render agentRows(agents.slice(0, 4))}
            {#if agents.length > 4}<button class="link tg-more" onclick={() => onview?.('agents')}>All {agents.length} →</button>{/if}
          {:else}<p class="tg-dim">None yet.</p>{/if}
          <button class="link tg-more" onclick={createAgent}>Or create a new agent from it</button>
        </section>
      </div>
    {:else}
      <div class="tg-next"><button class="tg-btn primary" onclick={createAgent}>Create an agent from it</button><button class="tg-btn" onclick={() => onview?.('yaml')}>Edit its code</button></div>
    {/if}

    <section class="tg-card">
      <h3>What it is made of</h3>
      <div class="tg-chips">
        <span class="tg-chip"><b>Model</b>{facts.agent.model ?? 'chosen per agent'}</span>
        <span class="tg-chip"><b>Tools</b>{facts.agent.mcpServers.length + facts.agent.tools.length ? [...facts.agent.mcpServers, ...facts.agent.tools].join(', ') : 'none'}</span>
        <span class="tg-chip"><b>Memory</b>{facts.agent.memory ? 'yes' : 'none'}</span>
        <span class="tg-chip"><b>Hooks</b>{facts.hooks.length ? facts.hooks.join(', ') : 'none'}</span>
      </div>
      <button class="link tg-more" onclick={() => onview?.('yaml')}>See its code →</button>
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
      <div><dt>From</dt><dd>{template.namespace ?? facts.namespace ?? '—'}</dd></div>
      <div><dt>By</dt><dd>{template.created_by || '—'}</dd></div>
      <div><dt>Used by</dt><dd><button class="link" onclick={() => onview?.('agents')}>{agentsLoading ? '…' : `${agents.length} agent${agents.length === 1 ? '' : 's'} · ${copies.length} cop${copies.length === 1 ? 'y' : 'ies'}`}</button></dd></div>
    </dl>

  {:else if view === 'agents'}
    <section class="tg-card">
      <header class="tg-row"><h3>Agents built from it <span class="tg-count">{agents.length}</span></h3><button class="tg-btn" onclick={createAgent}>Create an agent from it</button></header>
      {#if agentsLoading}
        <!-- the header bar shows loading -->
      {:else if agents.length === 0}<p class="tg-dim">None yet. An agent is a working copy of this template with its own model, tools and way of being started.</p>
      {:else}
        <ul class="tg-agentlist">
          {#each agents as a (a.id)}
            {@const about = describe(a)}
            <li><button onclick={() => navigateToDetail('agents', 'Agents', a.name ?? a.id)} title="Open {a.name ?? a.id}">
              <span class="tg-agent-head"><b class="mono">{a.name ?? a.id}</b><small>{a.namespace ?? ''}{a.mode ? ` · ${a.mode}` : ''}</small><span class="tg-st">{a.status ?? ''}</span></span>
              {#if about.text}<span class="tg-agent-desc">{#if about.fromPrompt}<em>Prompt:</em>&nbsp;{/if}{about.text}</span>
              {:else}<span class="tg-agent-desc tg-dim">No description.</span>{/if}
            </button></li>
          {/each}
        </ul>
      {/if}
    </section>

  {:else if view === 'versions'}
    {#if base}
      <section class="tg-card">
        <h3>Copied from</h3>
        <p class="tg-dim"><button class="link" onclick={() => base?.template && onopentemplate?.(base.template)}>{base.path}</button>, version {base.version}.
          {#if base.newer}<b>Version {base.newer} is out</b> — compare it, and bring changes into your copy in Code.{:else if base.template}It is still the latest.{:else}It is no longer listed.{/if}</p>
      </section>
    {/if}
    <section class="tg-card">
      <h3>Versions</h3>
      <ul class="tg-facts">
        {#each [...(template.versions ?? [template.activeVersion ?? template.version])].reverse() as v (v)}
          <li><b class="mono">v{v}</b><span>{v === (template.activeVersion ?? template.version) ? 'Current — what new agents and copies get.' : 'Earlier release.'}</span></li>
        {/each}
      </ul>
    </section>
    <section class="tg-card">
      <h3>Copies of it <span class="tg-count">{copies.length}</span></h3>
      {#if copies.length}
        <ul class="tg-agents">
          {#each copies as c (c.id)}
            <li><button onclick={() => onopentemplate?.(c)}><span class="mono">{c.name}</span><small>{c.namespace ?? ''}</small><span class="tg-st">from v{String(readTemplate(c.spec_yaml).copiedFrom ?? '').split('@')[1] ?? '?'}</span></button></li>
          {/each}
        </ul>
      {:else}<p class="tg-dim">None you can see.</p>{/if}
    </section>
  {/if}
</div>

<style>
  .tg { display: flex; flex-direction: column; gap: 14px; padding: 18px 20px 28px; max-width: 760px; }
  .tg-warn { padding: 9px 12px; border: 1px solid var(--warn, #b45309); border-radius: 8px; background: var(--warn-bg, #fbf0e0); font-size: 12.5px; color: var(--text-2); }
  .tg-hero h2 { margin: 0; font: 400 26px/1.25 var(--font-serif); color: var(--text-1); }
  .tg-summary { margin: 8px 0 0; font-size: 16px; color: var(--text-1); line-height: 1.45; }
  .tg-desc { margin: 8px 0 0; font-size: 13.5px; color: var(--text-2); line-height: 1.6; max-width: 68ch; }
  .tg-lineage { margin: 10px 0 0; font-size: 13px; color: var(--text-3); }
  .tg-lineage b { color: var(--warn, #b45309); font-weight: 600; }
  .tg-choice { display: grid; grid-template-columns: repeat(auto-fit, minmax(260px, 1fr)); gap: 14px; }
  .tg-option.on { border-color: var(--spec); }
  .tg-card { display: flex; flex-direction: column; gap: 10px; padding: 16px 18px; border: 1px solid var(--border); border-radius: 12px; background: var(--bg-surface); }
  .tg-card h3 { margin: 0; font: 400 17px var(--font-serif); color: var(--text-1); display: flex; align-items: center; gap: 8px; }
  .tg-chips { display: flex; flex-wrap: wrap; gap: 8px; }
  .tg-chip { display: inline-flex; gap: 6px; align-items: baseline; padding: 5px 10px; border: 1px solid var(--border); border-radius: 999px; background: var(--bg-subtle); font-size: 12.5px; color: var(--text-1); }
  .tg-chip b { font: 500 10.5px var(--font-mono); letter-spacing: 0.05em; text-transform: uppercase; color: var(--text-4); }
  .tg-more { align-self: flex-start; }
  .tg-two { display: grid; grid-template-columns: repeat(auto-fit, minmax(240px, 1fr)); gap: 14px; }
  .tg-fields { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 8px; }
  .tg-fields li { display: flex; flex-wrap: wrap; gap: 4px 8px; align-items: baseline; font-size: 13px; }
  .tg-fields small { color: var(--text-4); font-size: 11.5px; }
  .tg-fields em { flex-basis: 100%; font-style: normal; font-size: 12px; color: var(--text-3); }
  .tg-facts { list-style: none; margin: 0; padding: 0; }
  .tg-facts li { display: flex; gap: 12px; padding: 8px 2px; border-bottom: 1px solid var(--divider, var(--border)); font-size: 13px; }
  .tg-facts li:last-child { border-bottom: 0; }
  .tg-facts b { flex: 0 0 110px; color: var(--text-3); font-weight: 500; }
  .tg-dim { margin: 0; font-size: 13px; color: var(--text-3); }
  .tg-err { margin: 0; font-size: 12.5px; color: var(--err, #b42318); }
  .tg-kv { display: grid; grid-template-columns: repeat(auto-fit, minmax(120px, 1fr)); gap: 14px; margin: 0; padding: 4px 2px; }
  .tg-kv dt { font: 500 10.5px var(--font-mono); letter-spacing: 0.06em; text-transform: uppercase; color: var(--text-4); margin-bottom: 4px; }
  .tg-kv dd { margin: 0; font-size: 13.5px; color: var(--text-1); }
  .tg-next { display: flex; gap: 8px; flex-wrap: wrap; }
  .tg-btn { height: 32px; padding: 0 14px; border-radius: 8px; border: 1px solid var(--border); background: var(--bg-surface); font: 500 13px var(--font-sans); color: var(--text-1); cursor: pointer; }
  .tg-btn:hover:not(:disabled) { border-color: var(--spec); }
  .tg-btn.primary { background: var(--green, #1a7158); border-color: var(--green, #1a7158); color: #fff; }
  .tg-btn:disabled { opacity: 0.55; cursor: default; }
  .tg-form { display: flex; flex-direction: column; gap: 10px; }
  .tg-form label { display: flex; flex-direction: column; gap: 5px; font-size: 12.5px; color: var(--text-2); text-transform: none; letter-spacing: 0; font-weight: 500; }
  .tg-count { font: 500 10.5px var(--font-mono); color: var(--text-4); background: var(--bg-subtle); border-radius: 4px; padding: 1px 6px; }
  .tg-row { display: flex; align-items: center; justify-content: space-between; gap: 12px; }
  .tg-agentlist { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 8px; }
  .tg-agentlist button { width: 100%; display: flex; flex-direction: column; gap: 4px; padding: 10px 12px; border: 1px solid var(--border); border-radius: 10px; background: var(--bg-surface); cursor: pointer; font: inherit; text-align: left; }
  .tg-agentlist button:hover { border-color: var(--spec); }
  .tg-agent-head { display: flex; align-items: baseline; gap: 10px; }
  .tg-agent-head b { flex: 1 1 auto; font-size: 13.5px; color: var(--text-1); }
  .tg-agent-head small { font-size: 12px; color: var(--text-4); }
  .tg-agent-desc { font-size: 12.5px; color: var(--text-2); line-height: 1.5; }
  .tg-agent-desc em { font-style: normal; color: var(--text-4); }
  .tg-agents { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 6px; }
  .tg-agents button { width: 100%; display: flex; align-items: center; gap: 10px; padding: 8px 12px; border: 1px solid var(--border); border-radius: 8px; background: var(--bg-surface); cursor: pointer; font: inherit; font-size: 13px; text-align: left; }
  .tg-agents button:hover { border-color: var(--spec); }
  .tg-agents .mono { flex: 1 1 auto; }
  .tg-agents small { color: var(--text-4); }
  .tg-st { font: 500 10.5px var(--font-mono); letter-spacing: 0.05em; text-transform: uppercase; color: var(--text-3); }
  .link { border: 0; background: none; padding: 0; color: var(--spec); cursor: pointer; font: inherit; }
  .mono { font-family: var(--font-mono); }
</style>
