<script lang="ts">
  /// The agent tabs that change it: Configure (the few settings most people
  /// touch — the rest is changed, tested and debugged in the Studio), Branches
  /// (prod, dev branches, releases, forks) and Settings (name, where it came
  /// from, delete). Overview, Deploy and Activity are drawn by AgentHub.
  import { untrack } from 'svelte';
  import ResourceSelect from '$lib/components/editor/ResourceSelect.svelte';
  import {
    factsOf, placementOf, lineageOf, forksOf, ago,
    type AgentRec, type TemplateRec, type Placement,
  } from './agentData';
  import { branchesOf, addBranch, dropBranch, mergeBranch } from './agentBranches.svelte';

  type Opt = { value: string; label: string };
  let {
    section, agent, allAgents, templates, bindings = $bindable(), dirty, saveError = '', llmResources, mcpResources, me,
    ongo, onstudio, onopen, onsave, saving = false, ondiscard, onupgrade, ondelete, branchForm = $bindable(false),
  }: {
    section: 'configure' | 'branches' | 'settings';
    agent: AgentRec & { description?: string; created_by?: string; created_at?: string };
    allAgents: AgentRec[];
    templates: (TemplateRec & { namespace?: string })[];
    bindings: any;
    dirty: boolean;
    /// The last save's error, if it failed.
    saveError?: string;
    llmResources: Opt[];
    mcpResources: Opt[];
    me: string;
    ongo: (tab: string) => void;
    onstudio: () => void;
    /// Open another agent.
    onopen: (id: string) => void;
    /// Save Configure / Settings: the one Save, in the bar that shows while there are edits.
    onsave: () => void;
    saving?: boolean;
    ondiscard: () => void;
    onupgrade: (templateId: string, version: string) => Promise<string | null>;
    /// Delete it (and its endpoint and canvas); an error, or null.
    ondelete: () => Promise<string | null>;
    /// Open the "Branch to dev" form when the Branches tab shows.
    branchForm?: boolean;
  } = $props();

  const f = $derived(factsOf(agent, templates));
  const lineage = $derived(lineageOf(agent, allAgents));
  const forks = $derived(forksOf(agent, allAgents));

  // Where it runs, for Settings (delete needs it undeployed first).
  let placement = $state<Placement | null>(null);
  let placeLoaded = $state(false);
  let placedFor = '';
  $effect(() => {
    if (section !== 'settings' || agent.id === placedFor) return;
    placedFor = agent.id;
    placeLoaded = false;
    untrack(() => placementOf(agent, f).then((p) => { placement = p.placement; placeLoaded = true; }));
  });

  // ── Configure ──
  const tplPrompt = $derived(String(f.spec?.do ? (() => {
    for (const t of f.spec.do ?? []) for (const d of Object.values(t ?? {}) as any[]) if (typeof d?.with?.config?.system_prompt === 'string') return d.with.config.system_prompt;
    return '';
  })() : '').trim());
  /// Servers to show: the project's MCP resources, plus any it uses that are not listed.
  const servers = $derived.by((): Opt[] => {
    const known = new Set(mcpResources.map((m) => m.value));
    return [...mcpResources, ...((bindings.mcp_servers ?? []) as string[]).filter((s) => !known.has(s)).map((s) => ({ value: s, label: s.split('/').pop() ?? s }))];
  });
  function toggleServer(v: string, on: boolean) {
    const cur: string[] = bindings.mcp_servers ?? [];
    bindings.mcp_servers = on ? [...cur.filter((x) => x !== v), v] : cur.filter((x) => x !== v);
  }
  const remembers = $derived(bindings.mode === 'persistent');
  /// The template's own model (its config, not this agent's override).
  const tplModel = $derived(String(factsOf({ ...agent, resource_bindings: '{}' }, templates).llm ?? ''));

  // ── Branches (sample) ──
  const repo = $derived(branchesOf(agent.id, agent.created_at));
  let brName = $state('');
  let brIntent = $state('');
  let merging = $state('');
  let dropping = $state('');
  let mergedMsg = $state('');
  $effect(() => { if (branchForm) untrack(() => { brName = `dev/${new Date().toISOString().slice(5, 10).replace('-', '')}`; brIntent = ''; }); });
  function makeBranch() {
    const n = brName.trim().replace(/\s+/g, '-');
    if (!n) return;
    addBranch(agent.id, n.startsWith('dev/') ? n : `dev/${n}`, me || 'you', brIntent.trim());
    branchForm = false;
  }

  // ── Settings ──
  const newer = $derived.by(() => {
    if (!f.template) return null;
    const cmp = (a: string, b: string) => { const x = a.split('.').map(Number), y = b.split('.').map(Number); for (let i = 0; i < 3; i++) if ((x[i] || 0) !== (y[i] || 0)) return (x[i] || 0) - (y[i] || 0); return 0; };
    const same = templates.filter((t) => t.name === f.template!.name && cmp(t.version, f.template!.version) > 0);
    return same.sort((a, b) => cmp(b.version, a.version))[0] ?? null;
  });
  let upgrading = $state(false);
  let upgradeMsg = $state('');
  async function upgrade() {
    if (!newer) return;
    upgrading = true; upgradeMsg = '';
    upgradeMsg = (await onupgrade(newer.id, newer.version)) ?? '';
    upgrading = false;
  }
  let confirmName = $state('');
  let deleting = $state(false);
  let deleteErr = $state('');
  const deployed = $derived(!!placement && placement.deploymentState !== 'undeployed');
  async function del() {
    deleting = true; deleteErr = '';
    deleteErr = (await ondelete()) ?? '';
    deleting = false;
  }
</script>

{#snippet saveBar()}
  {#if dirty || saveError}
    <div class="ag-dirty" class:bad={!!saveError}>
      <span>{saveError ? `Not saved: ${saveError}` : 'Unsaved changes'}</span>
      <span class="ag-acts"><button class="ag-btn quiet" disabled={saving} onclick={ondiscard}>Discard</button>
        <button class="ag-btn primary" disabled={saving} onclick={onsave}>{saving ? 'Saving…' : 'Save'}</button></span>
    </div>
  {/if}
{/snippet}

{#if section === 'configure'}
  <section class="ag">
    {@render saveBar()}
    <div class="ag-card">
      <div class="ag-head"><h3>Model</h3><span>What it thinks with</span></div>
      <ResourceSelect value={bindings.llm} options={llmResources} placeholder="— choose a model —" onchange={(v) => (bindings.llm = v)} />
      {#if tplModel && !String(bindings.llm ?? '').endsWith(tplModel.split('/').pop() ?? '')}<p class="ag-dim">Its template uses <span class="mono">{tplModel}</span>. <button class="link" onclick={() => (bindings.llm = tplModel.includes('/') ? tplModel : `a3t/${tplModel}`)}>Use it</button></p>{/if}
    </div>

    <div class="ag-card">
      <div class="ag-head"><h3>Instructions</h3><span>What it is told, at the start of every conversation</span></div>
      <textarea rows="8" bind:value={bindings.system_prompt} placeholder="You are…"></textarea>
      {#if tplPrompt}
        {#if tplPrompt === String(bindings.system_prompt ?? '').trim()}
          <p class="ag-dim">The same as its template's.</p>
        {:else}
          <p class="ag-dim">Its own — the template says something else. <button class="link" onclick={() => (bindings.system_prompt = tplPrompt)}>Use the template's</button></p>
        {/if}
      {/if}
    </div>

    <div class="ag-card">
      <div class="ag-head"><h3>Tools</h3><span>The tool servers it may call — policies still decide every call</span></div>
      <ul class="ag-toggles">
        {#each servers as s (s.value)}
          {@const on = (bindings.mcp_servers ?? []).includes(s.value)}
          <li>
            <label class="ag-switch">
              <input type="checkbox" checked={on} onchange={(e) => toggleServer(s.value, (e.currentTarget as HTMLInputElement).checked)} />
              <span class="ag-knob" aria-hidden="true"></span>
              <span class="ag-sw-text"><b>{s.label}</b><small class="mono">{s.value}</small></span>
            </label>
          </li>
        {:else}
          <li class="ag-dim">No tool servers in this project yet. Add one in Resources.</li>
        {/each}
      </ul>
      {#if Array.isArray(bindings.enabled_tools)}
        <p class="ag-dim">Limited to {bindings.enabled_tools.length} tool{bindings.enabled_tools.length === 1 ? '' : 's'} of these servers. Choosing tools one by one is done in the Studio.</p>
      {/if}
    </div>

    <div class="ag-card">
      <div class="ag-head"><h3>Memory</h3><span>What it keeps between conversations</span></div>
      <label class="ag-switch">
        <input type="checkbox" checked={remembers} onchange={(e) => (bindings.mode = (e.currentTarget as HTMLInputElement).checked ? 'persistent' : 'session')} />
        <span class="ag-knob" aria-hidden="true"></span>
        <span class="ag-sw-text"><b>{remembers ? 'Remembers across conversations' : 'Starts fresh each conversation'}</b>
          <small>{remembers ? 'What happened and what it learned carry over to the next conversation.' : 'It keeps only what is said in the conversation it is in.'}</small></span>
      </label>
      {#if bindings.mode !== 'persistent' && bindings.mode !== 'session'}
        <p class="ag-dim">It runs in <b>{bindings.mode}</b> mode; turning this on makes it persistent.</p>
      {/if}
    </div>

    <div class="ag-more">
      <p>Everything else — temperature ({bindings.temperature}), its step limit ({bindings.max_iterations}), the flow that serves it, hooks and single tools — is changed, tested and debugged in the Studio.</p>
      <button class="ag-btn" onclick={onstudio}>Open in Studio</button>
    </div>
  </section>

{:else if section === 'branches'}
  <section class="ag">
    <div class="ag-sample"><b>Sample</b> Branches, merges and releases are not stored yet — this shows how the screen will work. Nothing here is saved. Forks are real.</div>

    <div class="ag-card ag-prod">
      <div class="ag-row">
        <div><span class="ag-tag prod">prod</span> <b class="ag-big">{repo.releases[0]?.tag ?? 'v1'}</b> <span class="ag-dim">· what callers get</span></div>
        {#if !branchForm}<button class="ag-btn primary" onclick={() => (branchForm = true)}>Branch to dev</button>{/if}
      </div>
      {#if mergedMsg}<p class="ag-ok">{mergedMsg}</p>{/if}
      {#if branchForm}
        <div class="ag-form">
          <p class="ag-dim">A branch is this agent in the dev environment. Change and test it there — prod is not touched until you merge.</p>
          <label>Branch name<input type="text" bind:value={brName} /></label>
          <label>What will you change? <small>(optional)</small><input type="text" bind:value={brIntent} placeholder="e.g. try a smaller model for low-priority alerts" /></label>
          <div class="ag-acts"><button class="ag-btn primary" disabled={!brName.trim()} onclick={makeBranch}>Make the branch</button><button class="ag-btn" onclick={() => (branchForm = false)}>Cancel</button></div>
        </div>
      {/if}
    </div>

    <h4>Dev branches <span class="ag-count">{repo.branches.length}</span></h4>
    <ul class="ag-list">
      {#each repo.branches as b (b.name)}
        <li class="ag-branch">
          <div class="ag-row">
            <div><span class="ag-tag dev">dev</span> <b class="mono">{b.name}</b> <span class="ag-dim">· {b.by}, {ago(b.created)}</span></div>
            <div class="ag-acts">
              <button class="ag-btn" onclick={onstudio}>Test in Studio</button>
              <button class="ag-btn primary" onclick={() => { merging = b.name; dropping = ''; }}>Merge to prod</button>
              <button class="ag-btn quiet" onclick={() => { dropping = b.name; merging = ''; }}>Drop</button>
            </div>
          </div>
          <ul class="ag-changes">
            {#each b.changes as c}<li>{c}</li>{:else}<li class="ag-dim">No changes yet — open it in the Studio to change and test it.</li>{/each}
          </ul>
          {#if merging === b.name}
            <div class="ag-confirm">
              <p>Merging makes <b>v{(Number(repo.releases[0]?.tag.slice(1)) || 0) + 1}</b> of prod. Callers get it from their next conversation.</p>
              <ul>
                <li><b>How it behaves</b> — instructions, model, tools — replaces prod's.</li>
                <li><b>What it learned</b> on the branch — procedures — is added to prod's memory. What prod learned meanwhile stays.</li>
              </ul>
              <div class="ag-acts"><button class="ag-btn primary" onclick={() => { const t = mergeBranch(agent.id, b.name); merging = ''; mergedMsg = `Merged ${b.name} — prod is now ${t}.`; }}>Merge</button><button class="ag-btn" onclick={() => (merging = '')}>Cancel</button></div>
            </div>
          {:else if dropping === b.name}
            <div class="ag-confirm">
              <p>Drop <b class="mono">{b.name}</b>? Its changes and what it learned are thrown away. Prod is not touched.</p>
              <div class="ag-acts"><button class="ag-btn danger" onclick={() => { dropBranch(agent.id, b.name); dropping = ''; }}>Drop it</button><button class="ag-btn" onclick={() => (dropping = '')}>Cancel</button></div>
            </div>
          {/if}
        </li>
      {:else}
        <li class="ag-empty">No dev branches. Branch to dev to change it without touching prod.</li>
      {/each}
    </ul>

    <h4>Releases</h4>
    <ul class="ag-list ag-rel">
      {#each repo.releases as r, i (r.tag)}
        <li><b class="mono">{r.tag}</b>{#if i === 0}<span class="ag-tag prod">prod</span>{/if}<span class="ag-note">{r.note}</span><time>{ago(r.at)}</time></li>
      {/each}
    </ul>

    <h4>Forks <span class="ag-count">{forks.length}</span></h4>
    <ul class="ag-list">
      {#each forks as k (k.id)}
        <li><button class="ag-pick" onclick={() => onopen(k.id)}><b>{k.name}</b><span class="mono ag-dim">{k.namespace}</span></button></li>
      {:else}
        <li class="ag-empty">No forks. A fork is a new agent that keeps its link to this one — make one from Overview.</li>
      {/each}
    </ul>
  </section>

{:else if section === 'settings'}
  <section class="ag">
    {@render saveBar()}
    <div class="ag-card">
      <div class="ag-head"><h3>About it</h3></div>
      <div class="ag-form">
        <label>Name<input type="text" bind:value={bindings.name} /></label>
        <label>What it does <small>(shown in lists and to people picking an agent)</small><textarea rows="3" bind:value={bindings.description} placeholder={f.purpose || 'One or two sentences'}></textarea></label>
      </div>
    </div>

    <div class="ag-card">
      <div class="ag-head"><h3>Where it comes from</h3></div>
      <dl class="ag-kv">
        <dt>Template</dt>
        <dd>{#if f.template}<span class="mono">{f.template.name} v{f.template.version}</span>{:else}<span class="ag-dim">{agent.template_id ?? 'none'}</span>{/if}</dd>
        <dt>Made as</dt>
        <dd>{#if lineage.kind === 'template'}new, from the template{:else}{lineage.kind === 'fork' ? 'a fork of' : 'a clone of'}
          {#if lineage.from?.exists}<button class="link" onclick={() => onopen(lineage.from!.id)}>{lineage.from.name}</button>{:else}<span>{lineage.from?.name} <span class="ag-dim">(deleted)</span></span>{/if}{/if}</dd>
        <dt>Project</dt><dd class="mono">{agent.namespace}</dd>
        <dt>Owner</dt><dd>{agent.created_by || 'unowned'}</dd>
        {#if agent.created_at}<dt>Created</dt><dd>{new Date(agent.created_at).toLocaleString()}</dd>{/if}
      </dl>
      {#if newer}
        <div class="ag-upgrade">
          <span>Its template has a newer release, <b>v{newer.version}</b>.</span>
          <button class="ag-btn" disabled={upgrading} onclick={upgrade}>{upgrading ? 'Moving…' : `Move to v${newer.version}`}</button>
          {#if upgradeMsg}<span class="ag-err">{upgradeMsg}</span>{/if}
        </div>
      {/if}
      <p class="ag-dim">To move it to another project, clone it there and delete this one. Who may use it is set by its project's access.</p>
    </div>

    <div class="ag-card ag-danger">
      <div class="ag-head"><h3>Delete this agent</h3></div>
      {#if !placeLoaded}
        <p class="ag-dim">Checking where it runs…</p>
      {:else if deployed}
        <p>It is deployed{placement?.runtime ? ` on ${placement.runtime}` : ''}. Undeploy it first, so nothing calls an agent that is gone.</p>
        <div class="ag-acts"><button class="ag-btn" onclick={() => ongo('deploy')}>Go to Deploy</button></div>
      {:else}
        <p>This removes, for good:</p>
        <ul class="ag-goes">
          <li>the agent and its settings</li>
          {#if f.endpoint}<li>its endpoint <span class="mono">POST /webhook{f.endpoint}</span> — anything calling it gets an error</li>{/if}
          <li>its deployment canvas, if it has one</li>
        </ul>
        <p class="ag-dim">Conversations stay with the people who had them.{forks.length ? ` Its ${forks.length} fork${forks.length === 1 ? '' : 's'} keep working, on their own.` : ''}</p>
        <label class="ag-confirm-name">Type <b class="mono">{agent.name}</b> to confirm<input type="text" bind:value={confirmName} autocomplete="off" /></label>
        <div class="ag-acts">
          <button class="ag-btn danger" disabled={confirmName !== agent.name || deleting} onclick={del}>{deleting ? 'Deleting…' : 'Delete agent'}</button>
          {#if deleteErr}<span class="ag-err">{deleteErr}</span>{/if}
        </div>
      {/if}
    </div>
  </section>
{/if}

<style>
  .ag { display: flex; flex-direction: column; gap: 14px; max-width: 760px; }
  h4 { margin: 8px 0 -4px; font: 500 10.5px var(--font-mono); letter-spacing: 0.06em; text-transform: uppercase; color: var(--text-4); display: flex; gap: 8px; align-items: center; }
  .mono { font-family: var(--font-mono); font-size: 12.5px; }
  .link { border: 0; background: none; padding: 0; color: var(--spec); cursor: pointer; font: inherit; }
  .link:hover { text-decoration: underline; }
  .ag-dim { margin: 0; font-size: 12.5px; color: var(--text-3); }
  .ag-err { font-size: 12.5px; color: var(--err, #b42318); }
  .ag-ok { margin: 0; font-size: 12.5px; color: var(--ok, #1a7158); }
  .ag-count { font: 500 10.5px var(--font-mono); color: var(--text-4); background: var(--bg-subtle); border-radius: 4px; padding: 1px 6px; letter-spacing: 0; }

  .ag-card { display: flex; flex-direction: column; gap: 10px; padding: 16px 18px; border: 1px solid var(--border); border-radius: 12px; background: var(--bg-surface); }
  .ag-head { display: flex; align-items: baseline; gap: 10px; flex-wrap: wrap; }
  .ag-head h3 { margin: 0; font: 400 17px var(--font-serif); color: var(--text-1); }
  .ag-head span { font-size: 12.5px; color: var(--text-3); }
  .ag-card textarea { width: 100%; font: 13px/1.55 var(--font-sans); resize: vertical; }

  .ag-dirty { display: flex; justify-content: space-between; align-items: center; gap: 12px; padding: 7px 8px 7px 14px; box-shadow: 0 4px 14px rgba(0,0,0,0.06); border: 1px solid var(--spec); border-radius: 8px; background: var(--spec-bg); font-size: 12.5px; color: var(--text-2); position: sticky; top: 0; z-index: 1; }
  .ag-dirty.bad { border-color: var(--err, #b42318); background: var(--err-bg, #fdecea); }
  .ag-sample { display: flex; gap: 10px; align-items: baseline; padding: 9px 12px; border: 1px dashed var(--warn, #b45309); border-radius: 8px; background: var(--warn-bg, #fbf0e0); font-size: 12.5px; color: var(--text-2); }

  .ag-toggles { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 4px; }
  .ag-switch { position: relative; display: flex; align-items: center; gap: 12px; padding: 8px 4px; cursor: pointer; text-transform: none; letter-spacing: 0; font-weight: 400; }
  .ag-switch input { position: absolute; left: 4px; top: 50%; opacity: 0; width: 1px; height: 1px; margin: 0; }
  .ag-knob { flex: 0 0 34px; height: 20px; border-radius: 999px; background: var(--border-strong, #c9c4bb); position: relative; transition: background 0.15s; }
  .ag-knob::after { content: ''; position: absolute; top: 2px; left: 2px; width: 16px; height: 16px; border-radius: 50%; background: #fff; box-shadow: 0 1px 2px rgba(0,0,0,0.2); transition: left 0.15s; }
  .ag-switch input:checked + .ag-knob { background: var(--green, #1a7158); }
  .ag-switch input:checked + .ag-knob::after { left: 16px; }
  .ag-switch input:focus-visible + .ag-knob { outline: 2px solid var(--spec); outline-offset: 2px; }
  .ag-sw-text { display: flex; flex-direction: column; gap: 2px; }
  .ag-sw-text b { font-size: 13px; font-weight: 500; color: var(--text-1); }
  .ag-sw-text small { font-size: 12px; color: var(--text-3); }

  .ag-more { display: flex; align-items: center; gap: 14px; padding: 12px 16px; border-radius: 10px; background: var(--bg-subtle); }
  .ag-more p { margin: 0; flex: 1; font-size: 12.5px; color: var(--text-3); line-height: 1.5; }

  .ag-btn { height: 30px; padding: 0 12px; border-radius: 8px; border: 1px solid var(--border); background: var(--bg-surface); font: 500 12.5px var(--font-sans); color: var(--text-1); cursor: pointer; white-space: nowrap; }
  .ag-btn:hover:not(:disabled) { border-color: var(--spec); }
  .ag-btn.primary { background: var(--green, #1a7158); border-color: var(--green, #1a7158); color: #fff; }
  .ag-btn.danger { background: var(--err, #b42318); border-color: var(--err, #b42318); color: #fff; }
  .ag-btn.quiet { border-color: transparent; background: none; color: var(--text-3); }
  .ag-btn:disabled { opacity: 0.5; cursor: default; }
  .ag-acts { display: flex; gap: 8px; align-items: center; flex-wrap: wrap; }
  .ag-row { display: flex; align-items: center; justify-content: space-between; gap: 12px; flex-wrap: wrap; }

  .ag-form { display: flex; flex-direction: column; gap: 10px; }
  .ag-form label, .ag-confirm-name { display: flex; flex-direction: column; gap: 5px; font-size: 12.5px; color: var(--text-2); text-transform: none; letter-spacing: 0; font-weight: 500; }
  .ag-form small { font-weight: 400; color: var(--text-4); }

  .ag-tag { display: inline-block; font: 600 10px var(--font-mono); letter-spacing: 0.06em; text-transform: uppercase; padding: 2px 7px; border-radius: 4px; vertical-align: 1px; }
  .ag-tag.prod { background: color-mix(in srgb, var(--green, #1a7158) 14%, transparent); color: var(--green, #1a7158); }
  .ag-tag.dev { background: var(--spec-bg); color: var(--spec-ink, var(--spec)); }
  .ag-big { font: 400 20px var(--font-serif); }

  .ag-list { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 8px; }
  .ag-branch { display: flex; flex-direction: column; gap: 8px; padding: 12px 14px; border: 1px solid var(--border); border-radius: 10px; background: var(--bg-surface); }
  .ag-changes { margin: 0; padding-left: 18px; font-size: 12.5px; color: var(--text-2); }
  .ag-changes .ag-dim { list-style: none; margin-left: -18px; }
  .ag-confirm { display: flex; flex-direction: column; gap: 8px; padding: 12px 14px; border-radius: 8px; background: var(--bg-subtle); font-size: 13px; color: var(--text-2); }
  .ag-confirm p { margin: 0; }
  .ag-confirm ul { margin: 0; padding-left: 18px; display: flex; flex-direction: column; gap: 4px; }
  .ag-empty { padding: 14px; border: 1px dashed var(--border); border-radius: 10px; font-size: 12.5px; color: var(--text-4); }
  .ag-rel li { display: flex; align-items: baseline; gap: 10px; padding: 8px 12px; border-bottom: 1px solid var(--divider, var(--border)); font-size: 13px; }
  .ag-rel li:last-child { border-bottom: 0; }
  .ag-rel .ag-note { flex: 1; color: var(--text-2); }
  .ag-rel time { font-size: 11.5px; color: var(--text-4); white-space: nowrap; }
  .ag-pick { width: 100%; display: flex; gap: 10px; align-items: baseline; padding: 9px 12px; border: 1px solid var(--border); border-radius: 8px; background: var(--bg-surface); cursor: pointer; font: inherit; font-size: 13px; text-align: left; }
  .ag-pick:hover { border-color: var(--spec); }
  .ag-pick b { flex: 1; font-weight: 500; }

  .ag-kv { display: grid; grid-template-columns: 110px 1fr; gap: 8px 14px; margin: 0; font-size: 13px; }
  .ag-kv dt { color: var(--text-3); }
  .ag-kv dd { margin: 0; color: var(--text-1); overflow-wrap: anywhere; }
  .ag-upgrade { display: flex; align-items: center; gap: 12px; flex-wrap: wrap; padding: 10px 12px; border-radius: 8px; background: var(--spec-bg); font-size: 13px; }
  .ag-danger { border-color: color-mix(in srgb, var(--err, #b42318) 35%, var(--border)); }
  .ag-danger p { margin: 0; font-size: 13px; color: var(--text-2); }
  .ag-goes { margin: 0; padding-left: 18px; font-size: 13px; color: var(--text-2); display: flex; flex-direction: column; gap: 3px; }
</style>
