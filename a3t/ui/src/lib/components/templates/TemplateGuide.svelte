<script lang="ts">
  /// A template as people meet it: a blueprint to understand, then copy or use.
  /// About (what it is, and the choice: clone and edit, or use an agent already
  /// built from it) and Versions (its releases, its copies, its agents — and,
  /// for a copy, its base). The page's Design shows how it is built — read-only
  /// on a base template, which is never edited in place.
  ///
  /// Facts come from the spec (templateSpec.ts) and the registry; nothing is
  /// made up.
  import { navigateTo, navigateToDetail, openInStudio } from '$lib/stores/panels.svelte.ts';
  import { releasesApi } from '$lib/services/api';
  import { readTemplate } from './templateSpec';

  let { view, template, readOnly, agents = [], agentsLoading = false, allTemplates = [], projects = [], copyRequest = 0, oncopy, onview, onopentemplate, onlive }: {
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
    /// Its live version, once FlowR has said which (the page's header shows it).
    onlive?: (v: string) => void;
  } = $props();

  const facts = $derived(readTemplate(template?.spec_yaml));
  const title = $derived(facts.title || template?.name || 'Template');
  const ref = $derived(`${template?.namespace ?? facts.namespace ?? ''}/${template?.name ?? ''}`);
  const createAgent = () => navigateTo('agents', 'Agents', { createFromTemplate: template.id });
  const ns = $derived(template?.namespace ?? facts.namespace ?? 'a3t');

  // Its versions as FlowR holds them: which is live (what every agent made from
  // it runs, and what new agents get) and which came before.
  let versions = $state<string[] | null>(null);
  let live = $state<string>('');
  let verErr = $state('');
  async function loadVersions(n: string, name: string) {
    versions = null; verErr = '';
    const r = await releasesApi.versions(n, name);
    if (template?.name !== name) return;
    if (r.ok) { const d: any = r.data; versions = [...(d?.versions ?? [])].reverse(); live = d?.active ?? ''; if (live) onlive?.(live); }
    else { versions = []; verErr = r.error ?? 'Its versions could not be read.'; }
  }
  $effect(() => { if (template?.name && !template?.isNew) loadVersions(ns, template.name); });
  const catalogVersion = $derived(String(template?.activeVersion ?? template?.version ?? ''));
  let activating = $state('');
  let confirmVer = $state('');
  async function makeLive(v: string) {
    activating = v; verErr = '';
    const r = await releasesApi.activate(ns, template.name, v);
    activating = ''; confirmVer = '';
    if (!r.ok) { verErr = r.error ?? 'Not changed.'; return; }
    await loadVersions(ns, template.name);
  }
  /// Agents made from it, by the version they were made from.
  const byVersion = $derived(agents.reduce((m: Record<string, number>, a: any) => { const v = String(a.template_version ?? '?'); m[v] = (m[v] ?? 0) + 1; return m; }, {}));
  let howOpen = $state(false);
  /// What it is for, in a sentence: its summary, else the first sentence of its description.
  const purpose = $derived(facts.summary || (facts.description ?? '').trim().replace(/\s+/g, ' ').split(/(?<=[.!?])\s/)[0] || '');

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
      <h2 class="mono">{template.name}</h2>
      {#if purpose}<p class="tg-summary">{purpose}</p>{:else}<p class="tg-dim">{readOnly ? 'It says nothing about what it is for.' : 'Say what it is for: add document.summary in its code.'}</p>{/if}
      {#if facts.description && facts.description.trim() !== purpose}
        <button class="link tg-more" onclick={() => (howOpen = !howOpen)}>{howOpen ? 'Less' : 'How it works'}</button>
        {#if howOpen}{#each facts.description.trim().split(/\n\s*\n/) as para}<p class="tg-desc">{para.replace(/\s*\n\s*/g, ' ')}</p>{/each}{/if}
      {/if}
      <div class="tg-next">
        <button class="tg-btn primary" onclick={createAgent}>Create an agent from it</button>
        {#if readOnly}<button class="tg-btn" onclick={openCopy} disabled={!projects.length || copying}>Clone to make your own</button>
        {:else}<button class="tg-btn" onclick={() => openInStudio('template', ns, template.name, template.id)}>Edit in Studio</button>{/if}
      </div>
      {#if copying}
        <div class="tg-card tg-form">
          <p class="tg-dim">Your own copy, in a project of yours, to change as you like. This one stays as it is.</p>
          <label><span>Name of your copy</span><input type="text" class="mono" bind:value={copyName} /></label>
          <label><span>In project</span>
            <select bind:value={copyProject} onchange={() => (copyName = freeName(copyProject))}>{#each projects as p (p.name)}<option value={p.name}>{p.label}</option>{/each}</select>
          </label>
          {#if copyError}<p class="tg-err">{copyError}</p>{/if}
          <div class="tg-next"><button class="tg-btn primary" onclick={doCopy} disabled={copyBusy || !copyName.trim()}>{copyBusy ? 'Copying…' : 'Make my copy'}</button><button class="tg-btn" onclick={() => (copying = false)}>Cancel</button></div>
        </div>
      {/if}
      {#if readOnly && !projects.length}<p class="tg-dim">To make your own copy you need a project you can write to.</p>{/if}
    </section>

    <section class="tg-card">
      <h3>Status</h3>
      <ul class="tg-st-rows">
        <li><span class="k">Version</span>
          <div>{#if versions === null}…{:else if live}<b class="mono">v{live}</b> is live — every agent made from it runs it, and new agents get it. {versions.length} version{versions.length === 1 ? '' : 's'} in all.
            {#if catalogVersion && catalogVersion !== live}<span class="tg-warn-in">Its catalogue entry still says v{catalogVersion}.</span>{/if}{:else}Not published as a flow yet.{/if}</div>
          <button class="tg-btn sm" onclick={() => onview?.('versions')}>Versions</button></li>
        <li><span class="k">Used by</span>
          <div>{#if agentsLoading}…{:else if agents.length}<b>{agents.length}</b> agent{agents.length === 1 ? '' : 's'}: {agents.slice(0, 3).map((a) => a.name ?? a.id).join(', ')}{agents.length > 3 ? ` and ${agents.length - 3} more` : ''}.{:else}No agent yet.{/if}
            {#if copies.length} {copies.length} cop{copies.length === 1 ? 'y' : 'ies'} of it in other projects.{/if}</div>
          <button class="tg-btn sm" onclick={() => onview?.('agents')}>Agents</button></li>
        {#if base}
          <li><span class="k">Copied from</span>
            <div><span class="mono">{base.path}</span> v{base.version}{#if base.newer} — <b>v{base.newer} is out</b>: bring its changes into this copy in Studio.{:else if base.template} — still the latest.{/if}</div>
            <button class="tg-btn sm" onclick={() => base?.template && onopentemplate?.(base.template)} disabled={!base.template}>Open it</button></li>
        {/if}
        <li><span class="k">Owner</span>
          <div>{readOnly ? 'Built into A3T or shared with you — read-only here.' : 'Yours.'} Project <span class="mono">{ns}</span>{template.created_by ? `, made by ${template.created_by}` : ''}.</div>
          <span></span></li>
      </ul>
    </section>

    <section class="tg-card">
      <h3>An agent made from it sets</h3>
      {#if facts.onCore}
        <ul class="tg-st-rows">
          <li><span class="k">Instructions</span><div>{facts.agent.systemPrompt ? 'Its own; the template starts it with its own text.' : 'Its own: what it is told at the start of every conversation.'}</div><span></span></li>
          <li><span class="k">Model</span><div>{facts.agent.model ? `${facts.agent.model} unless the agent picks another.` : 'Any model resource it can reach.'}</div><span></span></li>
          <li><span class="k">Tools</span><div>{facts.agent.mcpServers.length ? `${facts.agent.mcpServers.join(', ')} by default; ` : ''}the MCP servers and tools it is given, all or only some.</div><span></span></li>
          <li><span class="k">Memory</span><div>Across conversations, one conversation, or none — in its own space on the org's ADB.</div><span></span></li>
          <li><span class="k">Hooks</span><div>Its own flows at any phase of a turn: start, recall, each model and tool call, memory write, end.</div><span></span></li>
          <li><span class="k">Knowledge</span><div>Semantic spaces it reads from, besides its own memory.</div><span></span></li>
        </ul>
      {:else if facts.inputs.length}
        <ul class="tg-fields">{#each facts.inputs as f (f.name)}<li><span class="mono">{f.name}</span><small>{f.type}{f.required ? ' · required' : ''}</small>{#if f.description}<em>{f.description}</em>{/if}</li>{/each}</ul>
      {:else}<p class="tg-dim">Nothing declared: it takes what its caller sends.</p>{/if}
    </section>

    <section class="tg-card">
      <h3>What it runs and reaches</h3>
      <ul class="tg-st-rows">
        {#if facts.extends}<li><span class="k">Extends</span><div class="mono">{facts.extends}</div>
          <button class="tg-btn sm" onclick={() => { const [n, rest] = facts.extends!.split('/'); openInStudio('flow', n, rest.split('@')[0]); }}>Open</button></li>{/if}
        <li><span class="k">Flows</span><div>{#if facts.reach.flows.length}{#each facts.reach.flows as fl (fl)}<button class="link mono chip" onclick={() => { const [n, ...r] = fl.split('/'); openInStudio('flow', n, r.join('/')); }}>{fl}</button>{/each}{:else}none{/if}</div><span></span></li>
        <li><span class="k">Resources</span><div>{#if facts.reach.resources.length}{#each facts.reach.resources as r (r)}<span class="mono chip">{r}</span>{/each}{:else}none named{/if}</div>
          <button class="tg-btn sm" onclick={() => navigateTo('resources', 'Resources')}>Resources</button></li>
        <li><span class="k">Calls</span><div>{Object.entries(facts.reach.calls).map(([c, n]) => `${c}${n > 1 ? ` ×${n}` : ''}`).join(' · ') || 'none'}{facts.reach.dynamic ? ` · ${facts.reach.dynamic} chosen at run time` : ''}</div><span></span></li>
      </ul>
      <button class="link tg-more" onclick={() => onview?.('yaml')}>See its code →</button>
    </section>

  {:else if view === 'agents'}
    <section class="tg-card">
      <header class="tg-row"><h3>Agents built from it <span class="tg-count">{agents.length}</span></h3><button class="tg-btn" onclick={createAgent}>Create an agent from it</button></header>
      {#if agentsLoading}
        <!-- the header bar shows loading -->
      {:else if agents.length === 0}<p class="tg-dim">None yet. An agent is this template with its own instructions, model, tools and memory.</p>
      {:else}
        {#if live}<p class="tg-dim">Every one runs the live version, v{live}; the version each was made from is noted.</p>{/if}
        <ul class="tg-agentlist">
          {#each agents as a (a.id)}
            {@const about = describe(a)}
            <li><button onclick={() => navigateToDetail('agents', 'Agents', a.name ?? a.id)} title="Open {a.name ?? a.id}">
              <span class="tg-agent-head"><b class="mono">{a.name ?? a.id}</b><small>{a.namespace ?? ''}{a.mode ? ` · ${a.mode}` : ''}{a.template_version ? ` · made from v${a.template_version}` : ''}</small><span class="tg-st">{a.status ?? ''}</span></span>
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
      <h3>Versions <span class="tg-count">{versions?.length ?? '…'}</span></h3>
      {#if verErr}<p class="tg-err">{verErr}</p>{/if}
      {#if versions === null}<p class="tg-dim">…</p>
      {:else if !versions.length}<p class="tg-dim">Not published as a flow yet.</p>
      {:else}
        <ul class="tg-vers">
          {#each versions as v (v)}
            <li class:live={v === live}>
              <b class="mono">v{v}</b>
              <span>{v === live ? 'Live — every agent made from it runs this, and new agents get it.' : 'Earlier.'}{byVersion[v] ? ` ${byVersion[v]} agent${byVersion[v] === 1 ? ' was' : 's were'} made from it.` : ''}</span>
              <span class="grow"></span>
              <button class="link" onclick={() => openInStudio('flow', ns, template.name)}>Code</button>
              {#if !readOnly && v !== live}
                {#if confirmVer === v}
                  <button class="tg-btn sm primary" disabled={!!activating} onclick={() => makeLive(v)}>{activating === v ? '…' : `Make v${v} live`}</button><button class="link" onclick={() => (confirmVer = '')}>Cancel</button>
                {:else}<button class="tg-btn sm" onclick={() => (confirmVer = v)}>Make live</button>{/if}
              {/if}
            </li>
          {/each}
        </ul>
        {#if !readOnly}<p class="tg-dim">Making a version live changes what every agent made from it runs, from their next message. A new version is made by saving its code in Studio.</p>{/if}
      {/if}
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
  .tg { display: flex; flex-direction: column; gap: 14px; padding: 18px 20px 28px; max-width: var(--detail-w, 980px); }
  .tg-warn { padding: 9px 12px; border: 1px solid var(--warn, #b45309); border-radius: 8px; background: var(--warn-bg, #fbf0e0); font-size: 12.5px; color: var(--text-2); }
  .tg-hero h2 { margin: 0; font: 400 26px/1.25 var(--font-serif); color: var(--text-1); }
  .tg-summary { margin: 8px 0 0; font-size: 16px; color: var(--text-1); line-height: 1.45; }
  .tg-desc { margin: 8px 0 0; font-size: 13.5px; color: var(--text-2); line-height: 1.6; max-width: 68ch; }
  .tg-card { display: flex; flex-direction: column; gap: 10px; padding: 16px 18px; border: 1px solid var(--border); border-radius: 12px; background: var(--bg-surface); }
  .tg-card h3 { margin: 0; font: 400 17px var(--font-serif); color: var(--text-1); display: flex; align-items: center; gap: 8px; }
  .tg-more { align-self: flex-start; }
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
  .tg-st-rows { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; }
  .tg-st-rows li { display: grid; grid-template-columns: 110px 1fr auto; gap: 10px; align-items: start; padding: 9px 0; border-top: 1px solid var(--divider, var(--border)); font-size: 13px; color: var(--text-2); }
  .tg-st-rows li:first-child { border-top: 0; }
  .tg-st-rows .k { font-weight: 600; color: var(--text-1); }
  .tg-st-rows .chip { display: inline-block; margin: 0 6px 4px 0; padding: 1px 7px; border-radius: 5px; background: var(--bg-subtle); font-size: 12px; }
  .tg-warn-in { display: block; color: var(--warn, #b45309); font-size: 12px; }
  .tg-btn.sm { height: 26px; padding: 0 10px; font-size: 12px; }
  .tg-vers { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 6px; }
  .tg-vers li { display: flex; align-items: center; gap: 10px; flex-wrap: wrap; padding: 8px 12px; border: 1px solid var(--border); border-radius: 8px; font-size: 13px; color: var(--text-2); }
  .tg-vers li.live { border-color: var(--spec); background: var(--spec-bg); }
  .tg-vers b { color: var(--text-1); }
  .grow { flex: 1 1 auto; }
  @container details (max-width: 560px) {
    .tg-st-rows li { grid-template-columns: 1fr auto; }
    .tg-st-rows li > div { grid-column: 1 / 3; grid-row: 2; }
  }
  .link { border: 0; background: none; padding: 0; color: var(--spec); cursor: pointer; font: inherit; }
  .mono { font-family: var(--font-mono); }
</style>
