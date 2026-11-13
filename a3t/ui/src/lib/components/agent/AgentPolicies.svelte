<script lang="ts">
  /// Audit › Policies — the policies that have decided about this agent, and a
  /// test of what they would decide: this agent, called by someone with a given
  /// role, doing something (a tool, its model, a flow). The test runs FlowR's
  /// /security/policies/test against the saved policies (and a draft, if given),
  /// with the agent's own tool ceiling, the way the real check is made.
  import { securityPolicies } from '$lib/services/api';
  import { navigateTo } from '$lib/stores/panels.svelte.ts';
  import { factsOf, type AgentRec, type TemplateRec } from './agentData';
  import type { AgentLive } from './agentLive.svelte';

  let { agent, templates, live }: { agent: AgentRec & { created_by?: string }; templates: TemplateRec[]; live: AgentLive } = $props();
  const f = $derived(factsOf(agent, templates));

  /// Policies named in the decisions about it.
  const decided = $derived.by(() => {
    const m = new Map<string, { id: string; allow: number; deny: number }>();
    for (const d of live.decisions) for (const id of (d.reason || '').split(/,\s*/).filter((x) => x && !/\s/.test(x))) {
      const e = m.get(id) ?? { id, allow: 0, deny: 0 };
      d.allow ? e.allow++ : e.deny++;
      m.set(id, e);
    }
    return [...m.values()].sort((a, b) => b.deny - a.deny || b.allow - a.allow);
  });

  // ── The test ──
  const ROLES = ['Viewer', 'Operator', 'Admin', 'SecurityAdmin', 'SuperAdmin'];
  const DOES = [
    { k: 'tool', t: 'call a tool', action: 'mcp:call', type: 'Tool' },
    { k: 'model', t: 'use its model', action: 'llm:call', type: 'Llm' },
    { k: 'flow', t: 'start a flow', action: 'flow:start', type: 'Flow' },
  ] as const;
  let does = $state<'tool' | 'model' | 'flow'>('tool');
  let target = $state('');
  let role = $state('Viewer');
  let caller = $state('someone@example.com');
  let draft = $state('');
  let withSaved = $state(true);
  let running = $state(false);
  let result = $state<{ allowed?: boolean; decision?: string; reason?: string; policy_ids?: string[]; scopes?: string[]; error?: string } | null>(null);

  const targets = $derived(does === 'tool' ? live.allowedTools : does === 'model' ? (f.llm ? [f.llm] : []) : (f.target ? [f.target] : []));
  $effect(() => { if (!targets.includes(target)) target = targets[0] ?? ''; });

  async function run() {
    const d = DOES.find((x) => x.k === does)!;
    running = true; result = null;
    const all = live.tools.flatMap((s) => s.tools);
    const r = await securityPolicies.test({
      principal: { kind: 'agent', agent_id: `agent:${agent.id}:policy-test`, on_behalf_of: caller.trim() || undefined, roles: [role], owner: agent.created_by || undefined, flow_id: f.target || undefined } as any,
      action: d.action,
      resource: { type: d.type, id: target.trim() },
      attributes: does === 'tool' ? { template: { exposedTools: all, ...(f.tools ? { enabledTools: f.tools } : {}) } } : {},
      draft_policies: draft.trim() ? [draft] : [],
      include_registry: withSaved,
    });
    running = false;
    result = r.ok ? r.data : { error: r.status === 403 ? 'Only security admins can run policy tests.' : r.error ?? `HTTP ${r.status}` };
  }
  const verdict = $derived(!result || result.error ? null : result.allowed ? 'allowed' : result.scopes?.length || /ask/i.test(result.decision ?? '') ? 'ask' : 'denied');
</script>

<section class="agt">
  <div class="card">
    <div class="card-head"><h3>Policies that decided about it</h3><span>in the latest {live.scanned} decisions</span><span class="grow"></span>
      <button class="link" onclick={() => navigateTo('security', 'Security', { selectedId: 'policies' })}>All policies →</button></div>
    {#if live.decErr}<p class="dim">{live.decErr}</p>
    {:else if live.loading}<p class="dim">…</p>
    {:else if decided.length}
      <table class="tbl">
        <thead><tr><th>Policy</th><th class="num">Allowed</th><th class="num">Blocked</th></tr></thead>
        <tbody>{#each decided as p (p.id)}<tr><td class="mono">{p.id}</td><td class="num">{p.allow}</td><td class="num" class:bad={p.deny > 0}>{p.deny}</td></tr>{/each}</tbody>
      </table>
    {:else}<p class="dim">No decision about it names a policy yet: what it has done was allowed by the baseline. Test what a policy would decide below.</p>{/if}
  </div>

  <div class="card">
    <div class="card-head"><h3>Test a policy</h3><span>what would be decided if it did this, now</span></div>
    <div class="sentence">
      <span>Called by</span>
      <input class="who" type="text" bind:value={caller} aria-label="Caller" />
      <span>as</span>
      <select class="role" bind:value={role} aria-label="Role">{#each ROLES as r (r)}<option>{r}</option>{/each}</select>
      <span>to</span>
      <select class="does" bind:value={does} aria-label="What it does">{#each DOES as d (d.k)}<option value={d.k}>{d.t}</option>{/each}</select>
      {#if targets.length}
        <select class="tgt-pick" bind:value={target}>{#each targets as t (t)}<option>{t}</option>{/each}</select>
      {:else}
        <input class="tgt" type="text" bind:value={target} placeholder={does === 'tool' ? 'tool name' : does === 'model' ? 'ns/model' : 'ns/flow'} />
      {/if}
    </div>
    <details class="draft">
      <summary>With a draft policy {draft.trim() ? '(1)' : ''}</summary>
      <textarea rows="6" bind:value={draft} placeholder={'forbid (principal, action == Action::"mcp:call", resource)\nwhen { resource.name == "delete_flow" };'}></textarea>
      <label class="ck"><input type="checkbox" bind:checked={withSaved} /> and the saved policies (off: the draft against the baseline alone)</label>
    </details>
    <div class="row"><button class="btn2 primary" disabled={running || !target.trim()} onclick={run}>{running ? 'Testing…' : 'Test'}</button>
      <span class="dim">Nothing runs: the policies are asked what they would decide.</span></div>
    {#if result?.error}<p class="err">{result.error}</p>{/if}
    {#if verdict}
      <div class="verdict {verdict}">
        <b>{verdict === 'allowed' ? '✓ Allowed' : verdict === 'ask' ? '? Would ask first' : '✕ Denied'}</b>
        {#if result?.reason}<span>{result.reason}</span>{/if}
        {#if result?.policy_ids?.length}<span class="dim">by <span class="mono">{result.policy_ids.join(', ')}</span></span>{/if}
        {#if result?.scopes?.length}<span class="dim">the prompt would offer: {result.scopes.join(', ')}</span>{/if}
      </div>
    {/if}
  </div>
</section>

<style>
  .grow { flex: 1 1 auto; }
  .num { text-align: right; font-variant-numeric: tabular-nums; }
  td.bad { color: var(--err); font-weight: 600; }
  /* One line: the fields share it and shrink to fit; it wraps only on a phone. */
  .sentence { display: flex; flex-wrap: nowrap; align-items: center; gap: 8px; font-size: 13px; color: var(--text-2); }
  .sentence > span { white-space: nowrap; flex: 0 0 auto; }
  .sentence select, .sentence input { width: auto; height: 30px; min-width: 0; padding-top: 0; padding-bottom: 0; font-size: 12.5px; }
  .sentence .who { flex: 1 1 180px; min-width: 120px; }
  .sentence .role { flex: 0 1 130px; min-width: 100px; }
  .sentence .does { flex: 0 1 140px; min-width: 110px; }
  .sentence .tgt, .sentence .tgt-pick { flex: 1 1 180px; min-width: 100px; }
  @media (max-width: 700px) { .sentence { flex-wrap: wrap; } }
  .draft > summary { cursor: pointer; font-size: 12.5px; color: var(--spec); width: fit-content; }
  .draft textarea { width: 100%; margin-top: 8px; font: 12.5px/1.5 var(--font-mono); }
  .ck { display: flex; align-items: center; gap: 8px; font-size: 12.5px; color: var(--text-2); text-transform: none; letter-spacing: 0; font-weight: 400; }
  .ck input { width: auto; }
  .verdict { display: flex; flex-direction: column; gap: 3px; padding: 10px 12px; border-radius: 8px; font-size: 13px; }
  .verdict.allowed { background: color-mix(in srgb, var(--green, #1a7158) 10%, var(--bg-surface)); }
  .verdict.allowed b { color: var(--green, #1a7158); }
  .verdict.ask { background: color-mix(in srgb, var(--warn) 12%, var(--bg-surface)); }
  .verdict.ask b { color: var(--warn); }
  .verdict.denied { background: color-mix(in srgb, var(--err) 10%, var(--bg-surface)); }
  .verdict.denied b { color: var(--err); }
  .mono { font-family: var(--font-mono); font-size: 0.92em; }
</style>
