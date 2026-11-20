<script lang="ts">
  /// Secure › Security profile — what this agent can reach, what it is bound to,
  /// and its own rules: limits that travel with it when it is copied or sold.
  /// Rules are built as sentences (starters) or written as a policy file
  /// (experts, Advanced); FlowR checks either before it is saved. No Cedar needed.
  import { untrack } from 'svelte';
  import { agentSpaceApi, securityCatalogue, securityPolicies, type AgentSecurity, type CatalogueEntry } from '$lib/services/api';
  import type { AgentRec } from './agentData';

  // The agent's rules are security: its org's admins and security admins change
  // them (FlowR says so in `policy.can_edit`), not its project's writers.
  let { agent }: { agent: AgentRec; canEdit?: boolean } = $props();

  type Rule = { cannot?: string[]; needs_approval?: string[]; timeout?: string; on?: { tools?: string[]; models?: { not_in?: string[]; in?: string[] } }; reason: string; who?: unknown };
  let prof = $state<AgentSecurity | null>(null);
  let cat = $state<CatalogueEntry[]>([]);
  let err = $state('');
  /// GOV-4: its policy no longer compiles, so it is stopped until fixed.
  let brokenErr = $state('');
  let busy = $state('');
  const canEdit = $derived(!!prof?.policy.can_edit);

  async function load() {
    err = '';
    const [p, c, h] = await Promise.all([agentSpaceApi.security(agent.id), securityCatalogue(), securityPolicies.health()]);
    brokenErr = h.ok ? (h.data?.broken ?? []).find((b) => b.kind === 'agent' && b.owner === agent.id)?.errors.join('; ') ?? '' : '';
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
  function fileOf(rs: Rule[], ceiling: string[] | null = ceilingNow, uses: string[] = usesNow, egress: string[] | null = egressNow, budget: Budget | null = budgetNow, redact: boolean = piiNow): string {
    const lines = ['kind: a3t/policy', 'name: profile', `description: ${JSON.stringify(`What ${agent.name} may not do, wherever it runs.`)}`];
    if (ceiling) lines.push(`role_ceiling: ${JSON.stringify(ceiling)}`);
    if (uses.length) lines.push(`uses: ${JSON.stringify(uses)}`);
    if (egress) lines.push(`egress: ${JSON.stringify(egress)}`);
    if (budget && Object.keys(budget).length) lines.push(`budget: ${JSON.stringify(budget)}`);
    if (redact) lines.push('pii: redact');
    if (rs.length) {
      lines.push('rules:');
      for (const r of rs) {
        if (r.needs_approval?.length) {
          lines.push(`  - needs_approval: ${JSON.stringify(r.needs_approval)}`);
          if (r.timeout) lines.push(`    timeout: ${JSON.stringify(r.timeout)}`);
        } else lines.push(`  - cannot: ${JSON.stringify(r.cannot ?? [])}`);
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
    // GOV-3: the org wants a second approver; nothing changed yet.
    if ((r.data as any)?.pending) { err = (r.data as any).message ?? 'Held for a second approver.'; return true; }
    await load();
    return true;
  }

  // ── Role ceiling (UI-3): the most authority it carries, whoever runs it ──
  const ceilingNow = $derived<string[] | null>(((prof?.policy as any)?.file?.role_ceiling ?? null) as string[] | null);
  const ROLES = ['Admin', 'Operator', 'Viewer'];
  let editCeiling = $state(false);
  let ceilPick = $state<Record<string, boolean>>({});
  function startCeiling() { ceilPick = Object.fromEntries(ROLES.map((r) => [r, ceilingNow ? ceilingNow.includes(r) : true])); editCeiling = true; }
  async function saveCeiling(clear = false) {
    const picked = ROLES.filter((r) => ceilPick[r]);
    const next = clear ? null : picked;
    const empty = !rules.length && !expertCedar && !next && !usesNow.length && !egressNow && !budgetNow && !piiNow;
    if (await save(empty ? '' : fileOf(rules, next), 'ceiling')) editCeiling = false;
  }

  // ── Org policies it runs under (GOV-6): a policy with no applies_to binds
  // only where it is named, here or in a flow's metadata.a3t.policyRef ──
  const usesNow = $derived<string[]>(((prof?.policy as any)?.file?.uses ?? []) as string[]);
  let newUse = $state('');
  async function setUses(next: string[]) {
    const empty = !rules.length && !expertCedar && !ceilingNow && !next.length && !egressNow && !budgetNow && !piiNow;
    if (await save(empty ? '' : fileOf(rules, ceilingNow, next), 'uses')) newUse = '';
  }

  // ── Where it may call out to (RT-1): within its org's egress files ──
  const egressNow = $derived<string[] | null>(((prof?.policy as any)?.file?.egress ?? null) as string[] | null);
  let newHost = $state('');
  async function setEgress(next: string[] | null) {
    const empty = !rules.length && !expertCedar && !ceilingNow && !usesNow.length && !next && !budgetNow && !piiNow;
    if (await save(empty ? '' : fileOf(rules, ceilingNow, usesNow, next), 'egress')) newHost = '';
  }

  // ── Budget (RT-4): over it, it stops until tomorrow (UTC) and an alert is logged ──
  type Budget = { runs_per_day?: number; tokens_per_day?: number; key_calls_per_minute?: number };
  const budgetNow = $derived<Budget | null>(((prof?.policy as any)?.file?.budget ?? null) as Budget | null);
  const usedToday = $derived<{ turns: number; tokens: number } | null>((prof as any)?.budget?.used_today ?? null);
  let bRuns = $state(''); let bTokens = $state(''); let bKey = $state(''); let editBudget = $state(false);
  function startBudget() { bRuns = String(budgetNow?.runs_per_day ?? ''); bTokens = String(budgetNow?.tokens_per_day ?? ''); bKey = String(budgetNow?.key_calls_per_minute ?? ''); editBudget = true; }
  async function saveBudget(clear = false) {
    const n = (v: string) => (v.trim() && Number(v) > 0 ? Math.floor(Number(v)) : undefined);
    const next: Budget | null = clear ? null : Object.fromEntries(Object.entries({ runs_per_day: n(bRuns), tokens_per_day: n(bTokens), key_calls_per_minute: n(bKey) }).filter(([, v]) => v !== undefined)) as Budget;
    const b = next && Object.keys(next).length ? next : null;
    const empty = !rules.length && !expertCedar && !ceilingNow && !usesNow.length && !egressNow && !b && !piiNow;
    if (await save(empty ? '' : fileOf(rules, ceilingNow, usesNow, egressNow, b), 'budget')) editBudget = false;
  }

  // ── Personal data (RT-10): redact what it stores — transcripts, memory ──
  const piiNow = $derived<boolean>(((prof?.policy as any)?.file?.pii ?? '') === 'redact');
  async function setPii(on: boolean) {
    const empty = !rules.length && !expertCedar && !ceilingNow && !usesNow.length && !egressNow && !budgetNow && !on;
    await save(empty ? '' : fileOf(rules, ceilingNow, usesNow, egressNow, budgetNow, on), 'pii');
  }

  // ── Add a rule (starters) ──
  let adding = $state(false);
  let nWhat = $state('');
  /// 'cannot': it may not; 'approval': only once a person approves each call (GOV-8).
  let nKind = $state<'cannot' | 'approval'>('cannot');
  let nTimeout = $state('');
  let nTools = $state('');
  let nModels = $state('');
  let nReason = $state('');
  const needsTools = $derived(nWhat === 'tools.external');
  const needsModels = $derived(nWhat === 'models.use');
  async function addRule() {
    if (!nWhat || !nReason.trim()) { err = nKind === 'approval' ? 'Pick what needs approval, and say why: the approver reads it.' : 'Pick what it may not do, and say why: people see the reason when it is refused.'; return; }
    const on: Rule['on'] = {};
    const tools = nTools.split(',').map((t) => t.trim()).filter(Boolean);
    const models = nModels.split(',').map((t) => t.trim()).filter(Boolean);
    if (needsTools && tools.length) on.tools = tools;
    if (needsModels && models.length) on.models = { not_in: models };
    const what = nKind === 'approval' ? { needs_approval: [nWhat], ...(nTimeout ? { timeout: nTimeout } : {}) } : { cannot: [nWhat] };
    const next = [...rules, { ...what, ...(Object.keys(on).length ? { on } : {}), reason: nReason.trim() }];
    if (await save(fileOf(next), 'add')) { adding = false; nWhat = ''; nTools = ''; nModels = ''; nReason = ''; nTimeout = ''; nKind = 'cannot'; }
  }
  async function removeRule(i: number) {
    const next = rules.filter((_, j) => j !== i);
    await save(next.length || expertCedar || ceilingNow || usesNow.length || egressNow || budgetNow || piiNow ? fileOf(next) : '', `rm${i}`);
  }

  // ── Advanced: the file itself ──
  let advanced = $state(false);
  let source = $state('');
  const list = (v: any): string[] => Array.isArray(v) ? v.map(String) : v ? [String(v)] : [];
</script>

<section class="agt">
  {#if err}<div class="note bad">{err}</div>{/if}
  {#if brokenErr}<div class="note bad"><b>Its policy no longer compiles</b>, so this agent is stopped everywhere until an org admin or security admin fixes it below: <span class="mono">{brokenErr}</span></div>{/if}
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
      <div class="card-head"><h3>2 · What it is bound to</h3><span>model and tools in its Model &amp; tools; the ceiling here</span></div>
      <ul class="rows wide">
        <li><span class="lbl">Model</span><span class="val mono">{list(prof.bound_to.model).join(', ') || 'the platform default'}</span><span></span></li>
        <li><span class="lbl">MCP servers</span><span class="val mono">{list(prof.bound_to.mcp_servers).join(', ') || 'none'}</span><span></span></li>
        <li><span class="lbl">Role ceiling</span><span class="val">{ceilingNow ? (ceilingNow.length ? `at most ${ceilingNow.join(', ')} — its roles are the caller's roles that are also here` : 'no roles: it can do nothing a role grants') : 'not narrowed: it carries the roles of whoever runs it'}</span>
          {#if canEdit}<button class="btn2" onclick={startCeiling}>Change</button>{:else}<span></span>{/if}</li>
        {#if editCeiling}
          <li class="full"><span class="val">
            {#each ROLES as r (r)}<label class="chk"><input type="checkbox" bind:checked={ceilPick[r]} /> {r}</label>{/each}
            <span class="acts"><button class="btn2 primary" disabled={!!busy} onclick={() => saveCeiling()}>{busy === 'ceiling' ? 'Checking…' : 'Save ceiling'}</button>
              {#if ceilingNow}<button class="btn2 quiet" disabled={!!busy} onclick={() => saveCeiling(true)}>Remove the ceiling</button>{/if}
              <button class="btn2 quiet" onclick={() => (editCeiling = false)}>Cancel</button></span></span></li>
        {/if}
        <li><span class="lbl">Runs under</span><span class="val">{usesNow.length ? usesNow.join(', ') : 'no named org policies (those that apply everywhere still do)'}</span><span></span></li>
        {#if canEdit}
          <li class="full"><span class="val">
            {#each usesNow as u (u)}<button class="btn2 quiet" disabled={!!busy} onclick={() => setUses(usesNow.filter((x) => x !== u))}>Drop {u}</button>{/each}
            <input class="mono" bind:value={newUse} placeholder="org/policy-name" />
            <button class="btn2" disabled={!!busy || !/^[^/\s]+\/[a-z0-9][a-z0-9-]*$/.test(newUse.trim())} onclick={() => setUses([...usesNow, newUse.trim()])}>{busy === 'uses' ? 'Checking…' : 'Add'}</button></span></li>
        {/if}
        <li><span class="lbl">Calls out to</span><span class="val">{egressNow ? (egressNow.length ? egressNow.join(', ') : 'nowhere') : 'wherever its org allows (never private or cloud-metadata addresses)'}</span><span></span></li>
        {#if canEdit}
          <li class="full"><span class="val">
            {#each egressNow ?? [] as h (h)}<button class="btn2 quiet" disabled={!!busy} onclick={() => setEgress((egressNow ?? []).filter((x) => x !== h))}>Drop {h}</button>{/each}
            <input class="mono" bind:value={newHost} placeholder="api.github.com or *.slack.com" />
            <button class="btn2" disabled={!!busy || !/^(\*\.)?[a-z0-9.-]+$/i.test(newHost.trim())} onclick={() => setEgress([...(egressNow ?? []), newHost.trim()])}>{busy === 'egress' ? 'Checking…' : 'Allow only these'}</button>
            {#if egressNow}<button class="btn2 quiet" disabled={!!busy} onclick={() => setEgress(null)}>Back to its org's list</button>{/if}</span></li>
        {/if}
        <li><span class="lbl">Budget</span><span class="val">{budgetNow ? [budgetNow.runs_per_day ? `${budgetNow.runs_per_day} turns a day` : '', budgetNow.tokens_per_day ? `${budgetNow.tokens_per_day.toLocaleString()} model tokens a day` : '', budgetNow.key_calls_per_minute ? `${budgetNow.key_calls_per_minute} calls a minute per key` : ''].filter(Boolean).join(' · ') : 'none set'}{usedToday ? ` — today: ${usedToday.turns} turns, ${usedToday.tokens.toLocaleString()} tokens` : ''}</span>
          {#if canEdit}<button class="btn2" onclick={startBudget}>Change</button>{:else}<span></span>{/if}</li>
        {#if editBudget}
          <li class="full"><span class="val">
            <input class="num" type="number" min="1" bind:value={bRuns} placeholder="turns a day" />
            <input class="num" type="number" min="1" bind:value={bTokens} placeholder="model tokens a day" />
            <input class="num" type="number" min="1" bind:value={bKey} placeholder="calls a minute per key" />
            <span class="acts"><button class="btn2 primary" disabled={!!busy} onclick={() => saveBudget()}>{busy === 'budget' ? 'Checking…' : 'Save budget'}</button>
              {#if budgetNow}<button class="btn2 quiet" disabled={!!busy} onclick={() => saveBudget(true)}>No budget</button>{/if}
              <button class="btn2 quiet" onclick={() => (editBudget = false)}>Cancel</button></span></span></li>
        {/if}
        <li><span class="lbl">Personal data</span><span class="val">{piiNow ? 'Redacted from what it stores: transcripts, memory, its memory tools' : 'Stored as said (the decision log is redacted either way)'}</span>
          {#if canEdit}<button class="btn2" disabled={!!busy} onclick={() => setPii(!piiNow)}>{busy === 'pii' ? 'Checking…' : piiNow ? 'Store as said' : 'Redact'}</button>{:else}<span></span>{/if}</li>
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
                <label class="fld">Kind
                  <select bind:value={nKind}>
                    <option value="cannot">It may not</option>
                    <option value="approval">It needs a person's approval to</option>
                  </select></label>
                {#if nKind === 'approval'}<label class="fld">Ask for up to
                  <select bind:value={nTimeout}>
                    <option value="">a day (the default)</option>
                    <option value="15m">15 minutes</option><option value="1h">1 hour</option><option value="4h">4 hours</option>
                    <option value="3d">3 days</option><option value="7d">7 days</option>
                  </select></label>{/if}
                <label class="fld full">{nKind === 'approval' ? 'Needs approval to' : 'It may not'}
                  <select bind:value={nWhat}>
                    <option value="">Pick…</option>
                    {#each choices as p (p.id)}<option value={p.id}>{p.label}</option>{/each}
                  </select></label>
                {#if needsTools}<label class="fld full">Only tools named <small>comma-separated; * matches anything, e.g. github*</small><input bind:value={nTools} placeholder="empty: every tool of a connected server" /></label>{/if}
                {#if needsModels}<label class="fld full">Except these models <small>comma-separated resource names</small><input class="mono" bind:value={nModels} placeholder="a3t/claude-cli-max" /></label>{/if}
                <label class="fld full">Why <small>{nKind === 'approval' ? 'shown to the approver' : 'shown when it is refused'}</small><input bind:value={nReason} placeholder="It works only with the customer's own systems." /></label>
              </div>
              {#if nWhat && nKind === 'approval'}<p class="dim">Each time this agent would {label(nWhat).toLowerCase()}{needsTools && nTools ? ` (tools named ${nTools})` : ''}, it waits until the person it acts for approves that one call in their Approvals inbox or the chat. Unanswered, it is refused.</p>
              {:else if nWhat}<p class="dim">This agent may not {label(nWhat).toLowerCase()}{needsTools && nTools ? ` (tools named ${nTools})` : ''}{needsModels && nModels ? ` (models other than ${nModels})` : ''}.</p>{/if}
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

    {#if !canEdit}<p class="dim">Only the org's admins and security admins change these rules.</p>{/if}

    {#if prof.policy.history?.length}
      <div class="card">
        <div class="card-head"><h3>Changes</h3><span>every change to its rules, kept</span></div>
        <ul class="rows wide">
          {#each prof.policy.history as h (h.version)}
            <li><span class="lbl mono">{h.version}</span><span class="val">{h.action} · {h.by} · {new Date(h.at).toLocaleString()}</span><span></span></li>
          {/each}
        </ul>
      </div>
    {/if}

    <p class="dim">{prof.note}</p>
  {/if}
</section>
