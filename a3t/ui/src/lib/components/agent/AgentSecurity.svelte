<script lang="ts">
  /// Secure › Security profile — what this agent can reach, what it is bound to,
  /// and its own rules: limits that travel with it when it is copied or sold.
  /// Rules are built as sentences (starters) or written as a policy file
  /// (experts, Advanced); FlowR checks either before it is saved. No Cedar needed.
  import { untrack } from 'svelte';
  import { agentSpaceApi, securityCatalogue, type AgentSecurity, type CatalogueEntry } from '$lib/services/api';
  import type { AgentRec } from './agentData';

  let { agent, canEdit }: { agent: AgentRec; canEdit: boolean } = $props();

  type Rule = { cannot: string[]; on?: { tools?: string[]; models?: { not_in?: string[]; in?: string[] } }; reason: string; who?: unknown };
  let prof = $state<AgentSecurity | null>(null);
  let cat = $state<CatalogueEntry[]>([]);
  let err = $state('');
  let busy = $state('');

  async function load() {
    err = '';
    const [p, c] = await Promise.all([agentSpaceApi.security(agent.id), securityCatalogue()]);
    if (!p.ok) { err = p.error ?? 'Could not read its profile.'; return; }
    prof = p.data!;
    if (c.ok) cat = c.data!.permissions;
    source = prof.policy.source ?? '';
  }
  $effect(() => { agent.id; untrack(() => void load()); });

  // Only what the engine enforces: app visibility (apps.*) is a console matter, not an agent's limit.
  const choices = $derived(cat.filter((p) => !p.enforced_by));
  const label = (id: string) => cat.find((p) => p.id === id)?.label ?? id;
  const rules = $derived<Rule[]>(((prof?.policy as any)?.file?.rules ?? []) as Rule[]);
  const expertCedar = $derived<string | undefined>((prof?.policy as any)?.file?.cedar);

  // ── The file, written back as YAML (JSON values are valid YAML) ──
  function fileOf(rs: Rule[]): string {
    const lines = ['kind: a3t/policy', 'name: profile', `description: ${JSON.stringify(`What ${agent.name} may not do, wherever it runs.`)}`];
    if (rs.length) {
      lines.push('rules:');
      for (const r of rs) {
        lines.push(`  - cannot: ${JSON.stringify(r.cannot)}`);
        if (r.who) lines.push(`    who: ${JSON.stringify(r.who)}`);
        if (r.on && Object.keys(r.on).length) lines.push(`    on: ${JSON.stringify(r.on)}`);
        lines.push(`    reason: ${JSON.stringify(r.reason)}`);
      }
    }
    if (expertCedar) lines.push('cedar: |', ...expertCedar.trimEnd().split('\n').map((l) => `  ${l}`));
    return lines.join('\n') + '\n';
  }

  async function save(yaml: string, what: string) {
    busy = what; err = '';
    const r = yaml.trim() ? await agentSpaceApi.savePolicy(agent.id, yaml) : await agentSpaceApi.dropPolicy(agent.id);
    busy = '';
    if (!r.ok) { err = r.error ?? 'Not saved.'; return false; }
    await load();
    return true;
  }

  // ── Add a rule (starters) ──
  let adding = $state(false);
  let nWhat = $state('');
  let nTools = $state('');
  let nModels = $state('');
  let nReason = $state('');
  const needsTools = $derived(nWhat === 'tools.external');
  const needsModels = $derived(nWhat === 'models.use');
  async function addRule() {
    if (!nWhat || !nReason.trim()) { err = 'Pick what it may not do, and say why: people see the reason when it is refused.'; return; }
    const on: Rule['on'] = {};
    const tools = nTools.split(',').map((t) => t.trim()).filter(Boolean);
    const models = nModels.split(',').map((t) => t.trim()).filter(Boolean);
    if (needsTools && tools.length) on.tools = tools;
    if (needsModels && models.length) on.models = { not_in: models };
    const next = [...rules, { cannot: [nWhat], ...(Object.keys(on).length ? { on } : {}), reason: nReason.trim() }];
    if (await save(fileOf(next), 'add')) { adding = false; nWhat = ''; nTools = ''; nModels = ''; nReason = ''; }
  }
  async function removeRule(i: number) {
    const next = rules.filter((_, j) => j !== i);
    await save(next.length || expertCedar ? fileOf(next) : '', `rm${i}`);
  }

  // ── Advanced: the file itself ──
  let advanced = $state(false);
  let source = $state('');
  const list = (v: any): string[] => Array.isArray(v) ? v.map(String) : v ? [String(v)] : [];
</script>

<section class="agt">
  {#if err}<div class="note bad">{err}</div>{/if}
  {#if !prof}<p class="dim">Reading its profile…</p>
  {:else}
    <div class="card">
      <div class="card-head"><h3>1 · What it can reach</h3><span>{prof.version ? `version ${prof.version}` : 'no version yet'} · from its flows</span></div>
      {#if prof.reach}
        <ul class="rows wide">
          <li><span class="lbl">Models</span><span class="val mono">{list(prof.reach.llm).join(', ') || 'none named'}</span><span></span></li>
          <li><span class="lbl">MCP servers</span><span class="val mono">{list(prof.reach.mcp).join(', ') || 'none'}</span><span></span></li>
          <li><span class="lbl">Web hosts</span><span class="val mono">{list(prof.reach.hosts).join(', ') || 'none'}</span><span></span></li>
          <li><span class="lbl">Memory</span><span class="val mono">{list(prof.reach.memory).join(', ') || 'none'}</span><span></span></li>
          <li><span class="lbl">Other flows</span><span class="val mono">{list(prof.reach.subflows).join(', ') || 'none'}</span><span></span></li>
        </ul>
      {:else}<p class="dim">Make a version to see what its flows reach.</p>{/if}
    </div>

    <div class="card">
      <div class="card-head"><h3>2 · What it is bound to</h3><span>set in its Model &amp; tools</span></div>
      <ul class="rows wide">
        <li><span class="lbl">Model</span><span class="val mono">{list(prof.bound_to.model).join(', ') || 'the platform default'}</span><span></span></li>
        <li><span class="lbl">MCP servers</span><span class="val mono">{list(prof.bound_to.mcp_servers).join(', ') || 'none'}</span><span></span></li>
        <li><span class="lbl">Tools it may call</span><span class="val">{Array.isArray(prof.bound_to.enabled_tools) ? (prof.bound_to.enabled_tools.length ? prof.bound_to.enabled_tools.join(', ') : 'none') : 'any its servers offer'}</span><span></span></li>
      </ul>
    </div>

    <div class="card">
      <div class="card-head"><h3>3 · Its own rules</h3><span>travel with it when it is copied or sold</span>
        {#if canEdit}<span class="grow"></span><button class="btn2 quiet" onclick={() => { advanced = !advanced; source = prof?.policy.source ?? ''; }}>{advanced ? 'Rules' : 'Advanced'}</button>{/if}</div>
      {#if !advanced}
        {#if rules.length}
          <ul class="rows wide">
            {#each prof.policy.rules as sentence, i (i)}
              <li><span class="val">{sentence}</span><span></span>
                {#if canEdit}<button class="btn2" disabled={!!busy} onclick={() => removeRule(i)}>{busy === `rm${i}` ? 'Removing…' : 'Remove'}</button>{:else}<span></span>{/if}</li>
            {/each}
          </ul>
        {:else}<p class="dim">No rules of its own: it may do whatever its bindings and the policies where it runs allow.</p>{/if}
        {#if expertCedar}<p class="dim">Plus expert Cedar rules (see Advanced).</p>{/if}

        {#if canEdit}
          {#if adding}
            <div class="review">
              <div class="fields">
                <label class="fld full">It may not
                  <select bind:value={nWhat}>
                    <option value="">Pick…</option>
                    {#each choices as p (p.id)}<option value={p.id}>{p.label}</option>{/each}
                  </select></label>
                {#if needsTools}<label class="fld full">Only tools named <small>comma-separated; * matches anything, e.g. github*</small><input bind:value={nTools} placeholder="empty: every tool of a connected server" /></label>{/if}
                {#if needsModels}<label class="fld full">Except these models <small>comma-separated resource names</small><input class="mono" bind:value={nModels} placeholder="a3t/claude-cli-max" /></label>{/if}
                <label class="fld full">Why <small>shown when it is refused</small><input bind:value={nReason} placeholder="It works only with the customer's own systems." /></label>
              </div>
              {#if nWhat}<p class="dim">This agent may not {label(nWhat).toLowerCase()}{needsTools && nTools ? ` (tools named ${nTools})` : ''}{needsModels && nModels ? ` (models other than ${nModels})` : ''}.</p>{/if}
              <div class="acts"><button class="btn2 primary" disabled={!!busy} onclick={addRule}>{busy === 'add' ? 'Checking…' : 'Add rule'}</button><button class="btn2 quiet" onclick={() => (adding = false)}>Cancel</button></div>
            </div>
          {:else}<div class="acts"><button class="btn2 primary" onclick={() => (adding = true)}>+ Rule</button></div>{/if}
        {/if}
      {:else}
        <p class="dim">The policy file: <span class="mono">kind: a3t/policy</span> with <span class="mono">rules</span>, and for experts a <span class="mono">cedar:</span> block of <span class="mono">forbid</span> statements. It applies to this agent only. FlowR checks it before saving.</p>
        <textarea class="cedar" rows="16" spellcheck="false" bind:value={source} readonly={!canEdit}></textarea>
        {#if canEdit}<div class="acts"><button class="btn2 primary" disabled={!!busy || source === (prof.policy.source ?? '')} onclick={() => save(source, 'raw')}>{busy === 'raw' ? 'Checking…' : source.trim() ? 'Check and save' : 'Remove the policy'}</button>
          <button class="btn2 quiet" disabled={source === (prof.policy.source ?? '')} onclick={() => (source = prof?.policy.source ?? '')}>Discard</button></div>{/if}
      {/if}
    </div>

    <p class="dim">{prof.note}</p>
  {/if}
</section>
