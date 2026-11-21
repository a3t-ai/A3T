<script lang="ts">
  /// One agent seen by security (not by its builder): who it is and who it acts
  /// for, what it can reach, which rules apply to it, what it was refused, and
  /// what changed. Real where the API has it (inventory row, decision log, org
  /// rules); the rest is sample data, tagged "sample", until the server has it.
  import { untrack } from 'svelte';
  import * as YAML from 'yaml';
  import PropertiesTree from '$lib/components/shared/PropertiesTree.svelte';
  import WizardHead from '$lib/components/shared/WizardHead.svelte';
  import '$lib/components/agent/agent.css';
  import { audit, orgPolicy, securityCatalogue, type InventoryRow } from '$lib/services/api';
  import { navigateToDetail } from '$lib/stores/panels.svelte.ts';
  import { rulesOf, sentence, type Rule } from './rules/model';

  let { row, onback }: { row: InventoryRow; onback: () => void } = $props();

  const I = (d: string) => `<path d="${d}"/>`;
  const STEPS = [
    { id: 'overview', label: 'Overview', icon: I('M2 2h12v12H2zM2 6h12'), hint: 'Risk, owner and what to do about it' },
    { id: 'identity', label: 'Identity', icon: I('M8 8a3 3 0 100-6 3 3 0 000 6zM2 15a6 6 0 0112 0'), hint: 'Its own keys and ceiling, and the people it acts for' },
    { id: 'access', label: 'Access', icon: I('M3 8h10M9 4l4 4-4 4'), hint: 'What it can reach after the rules are applied' },
    { id: 'rules', label: 'Rules', icon: I('M3 3h10M3 8h10M3 13h6'), hint: 'Every rule that applies to this agent' },
    { id: 'decisions', label: 'Decisions', icon: I('M3 8l3 3 7-7'), hint: 'What it was refused, asked, or would be refused, last 7 days' },
    { id: 'changes', label: 'Changes', icon: I('M8 2v6l4 2'), hint: 'Who changed its config, keys and rules' },
  ];
  let tab = $state('overview');
  const step = $derived(STEPS.find((s) => s.id === tab)!);

  // Decision log for this agent, last 7 days (real).
  let recs = $state<any[] | null>(null);
  let recErr = $state('');
  // The org's rules, to find the ones that apply to this agent (real, worked out here).
  let files = $state<any[]>([]);
  let perms = $state<{ id: string; label: string }[]>([]);
  async function load(r: InventoryRow) {
    recs = null; recErr = '';
    const [a, f, c] = await Promise.all([
      audit.query({ org: r.org, agent: r.id, since_ms: Date.now() - 7 * 864e5, limit: 1000 }),
      orgPolicy.files(r.org), securityCatalogue(),
    ]);
    if (r.id !== row.id) return;
    if (a.ok) recs = a.data?.records ?? []; else { recs = []; recErr = a.status === 403 ? 'You can\'t read this org\'s decisions.' : (a.error ?? 'Decisions could not be read.'); }
    files = f.ok ? (f.data?.files ?? []) : [];
    if (c.ok) perms = (c.data?.permissions ?? []).map((p: any) => ({ id: p.id, label: p.label }));
  }
  $effect(() => { const r = row; untrack(() => void load(r)); });

  const label = (id: string) => perms.find((p) => p.id === id)?.label ?? (id.startsWith('tools.') ? `call ${id.slice(6)}` : id);
  const parse = (t: string): any => { try { return YAML.parse(t) ?? {}; } catch { return {}; } };

  type Applies = { say: string; file: string; n: number; state: string; id: string; why: string };
  const applies = $derived.by(() => {
    const out: Applies[] = [];
    for (const f of files) {
      if (f.kind !== 'policy') continue;
      const y = parse(f.content);
      rulesOf(y).forEach((r: Rule | null, i: number) => {
        if (!r || r.callers === 'people') return;
        if (r.projects.length && !r.projects.includes(row.project)) return;
        const id = `${row.org}/${y.name}/r${i + 1}`;
        const n = (recs ?? []).filter((d) => (d.policy_ids ?? []).some((p: string) => p.startsWith(id))).length;
        out.push({ say: sentence(r, label), file: f.path, n, id, state: f.enforced ? 'on' : f.shadow ? 'shadow' : 'off',
          why: `${r.projects.length ? `project ${row.project}` : 'whole org'} · ${r.callers === 'agents' ? 'agents only' : 'people and agents'}` });
      });
    }
    return out;
  });

  const refused = $derived((recs ?? []).filter((d) => d.decision === 'deny'));
  const asked = $derived((recs ?? []).filter((d) => d.decision === 'ask'));
  const byAction = $derived.by(() => {
    const m = new Map<string, { n: number; rule: string; last: number }>();
    for (const d of refused) {
      const k = `${d.action} ${d.resource_id ?? ''}`.trim();
      const e = m.get(k) ?? { n: 0, rule: (d.policy_ids ?? [])[0] ?? d.reason ?? '—', last: 0 };
      e.n++; e.last = Math.max(e.last, d.ts_ms ?? 0); m.set(k, e);
    }
    return [...m.entries()].sort((a, b) => b[1].n - a[1].n);
  });
  // Delegated identities: the people (and apps) it acted for, from on_behalf_of (real).
  const actedFor = $derived.by(() => {
    const m = new Map<string, { n: number; deny: number; via: Set<string> }>();
    for (const d of recs ?? []) {
      const who = d.on_behalf_of ?? (d.principal_id && d.principal_id !== row.id ? d.principal_id : null);
      if (!who) continue;
      const e = m.get(who) ?? { n: 0, deny: 0, via: new Set<string>() };
      e.n++; if (d.decision === 'deny') e.deny++; if (d.impersonated_by) e.via.add(`acted by ${d.impersonated_by}`);
      m.set(who, e);
    }
    return [...m.entries()].sort((a, b) => b[1].n - a[1].n);
  });

  const toolList = $derived.by((): string[] => {
    const t: any = row.tools;
    if (Array.isArray(t)) return t.map((x) => (typeof x === 'string' ? x : x?.name ?? x?.id ?? JSON.stringify(x)));
    if (t && typeof t === 'object') return Object.keys(t);
    return [];
  });
  const modelName = $derived(typeof row.model === 'string' ? row.model : row.model?.name ?? row.model?.id ?? '—');
  const deniedRes = $derived(new Set(refused.map((d) => String(d.resource_id ?? ''))));
  const accessRows = $derived([
    ...(row.mcp_servers ?? []).map((s) => ({ kind: 'MCP server', what: s, state: deniedRes.has(s) ? 'blocked' : 'allowed', real: true })),
    ...toolList.map((s) => ({ kind: 'Tool', what: s, state: deniedRes.has(s) ? 'blocked' : 'allowed', real: true })),
    { kind: 'Model', what: modelName, state: 'allowed', real: true },
    ...(row.egress ?? []).map((h) => ({ kind: 'Outside service', what: h, state: 'allowed', real: true })),
    { kind: 'Memory space', what: `${row.project}/agent_memory · ${row.id}`, state: 'allowed', real: false },
    { kind: 'Knowledge space', what: 'a3t-docs', state: 'allowed', real: false },
    { kind: 'Secret', what: 'GITHUB_TOKEN', state: 'needs approval', real: false },
  ]);

  const when = (ms?: number | string | null) => { const t = typeof ms === 'number' ? ms : ms ? Date.parse(ms) : NaN; return Number.isFinite(t) ? new Date(t).toLocaleString(undefined, { month: 'short', day: 'numeric', hour: '2-digit', minute: '2-digit' }) : '—'; };
  const SAMPLE_CHANGES = $derived([
    { at: 'Nov 20, 14:02', who: row.owner, what: 'Changed the model and added a tool' },
    { at: 'Nov 19, 09:41', who: row.owner, what: 'Created a key (no expiry)' },
    { at: 'Nov 18, 17:15', who: 'qa-securityadmin@a3t.io', what: 'Set a role ceiling: Operator' },
  ]);

  // While a section has no real data, show sample rows (tagged "sample") so the
  // screen can be reviewed; real rows replace them as soon as there are any.
  const day = 864e5, now = Date.now();
  const blockedRows = $derived(byAction.length ? byAction : [
    ['tools.call github.delete_repo', { n: 4, rule: `${row.org}/no-deletes/r1`, last: now - 2 * 36e5 }],
    ['tools.call http.post api.stripe.com', { n: 2, rule: `${row.org}/egress/r2`, last: now - day }],
    ['memory.write a3t-docs', { n: 1, rule: 'platform/read-only-knowledge', last: now - 3 * day }],
  ] as [string, { n: number; rule: string; last: number }][]);
  const askedRows = $derived(asked.length ? asked : [
    { action: 'tools.call', resource_id: 'jira.create_issue', on_behalf_of: 'jane@acme.com', reason: 'approved by qa-securityadmin@a3t.io', ts_ms: now - 5 * 36e5 },
    { action: 'tools.call', resource_id: 'github.merge_pr', on_behalf_of: 'sam@acme.com', reason: 'timed out after 1 h: refused', ts_ms: now - 2 * day },
  ]);
  const actedRows = $derived(actedFor.length ? actedFor : [
    ['jane@acme.com', { n: 41, deny: 2, via: new Set<string>(['Admin']) }],
    ['sam@acme.com', { n: 12, deny: 0, via: new Set<string>(['Operator']) }],
    ['app: support-portal', { n: 230, deny: 5, via: new Set<string>(['its own key ak_…4f2c']) }],
  ] as [string, { n: number; deny: number; via: Set<string> }][]);
  const appliesRows = $derived(applies.length ? applies : [
    { say: 'Agents may not call tools that delete, anywhere in the org.', file: 'policies/no-deletes.yaml', n: 4, state: 'on', id: 's1', why: 'whole org · agents only' },
    { say: `Agents acting for Operator need approval to merge pull requests in ${row.project}.`, file: 'policies/merges.yaml', n: 1, state: 'on', id: 's2', why: `project ${row.project} · agents only` },
    { say: 'Nobody may call outside services after 20:00.', file: 'policies/office-hours.yaml', n: 0, state: 'shadow', id: 's3', why: 'whole org · people and agents' },
  ] as Applies[]);
</script>

<div class="panel-layout">
  <PropertiesTree icon="AG" color="#5530a8" name={row.name} meta={`agent · ${row.project} · ${row.org}`} nodes={[]} steps={STEPS} activeNode={tab} onselect={(id) => (tab = id)} />
  <div class="panel-col"><div class="panel-content"><div class="pf">
    <WizardHead {step} />
    <section class="agt">
      <div class="row"><button class="link" onclick={onback}>← All agents</button><span class="grow"></span>
        <button class="link" onclick={() => navigateToDetail('agents', 'Agents', row.id)}>Open in Agents app ↗</button></div>

      {#if tab === 'overview'}
        <div class="card">
          <div class="card-head"><h3>{row.name}</h3><span class="risk {row.risk}">{row.risk} risk</span>{#if row.status === 'blocked'}<span class="tag">stopped</span>{/if}</div>
          <p class="why">{row.why}</p>
          <dl class="kv">
            <dt>Owner</dt><dd class="mono">{row.owner}</dd>
            <dt>Project</dt><dd class="mono">{row.project}</dd>
            <dt>Made from</dt><dd class="mono">{row.template || '—'}</dd>
            <dt>Last activity</dt><dd>{when(row.last_activity)}</dd>
            <dt>Created</dt><dd>{when(row.created_at)}</dd>
          </dl>
        </div>
        <div class="tiles4">
          <button class="tile" onclick={() => (tab = 'decisions')}><b>{recs ? refused.length : '…'}</b><span>refused, 7 days</span></button>
          <button class="tile" onclick={() => (tab = 'decisions')}><b>{recs ? asked.length : '…'}</b><span>waited for approval</span></button>
          <button class="tile" onclick={() => (tab = 'rules')}><b>{applies.length + (row.own_policy ? row.rules : 0)}</b><span>rules apply</span></button>
          <button class="tile" onclick={() => (tab = 'identity')}><b>{actedFor.length}</b><span>people it acted for</span></button>
        </div>
        <div class="card">
          <div class="card-head"><h3>Act on it</h3><span class="tag sample">preview</span></div>
          <div class="acts">
            <button class="btn2 danger" disabled>Stop agent</button>
            <button class="btn2" disabled>Revoke all keys ({row.keys_live})</button>
            <button class="btn2" disabled>Start an access review</button>
            <button class="btn2" disabled>+ Rule for this agent</button>
          </div>
          <p class="dim">Stopping keeps its config and history; calls to it are refused until an admin starts it again.</p>
        </div>

      {:else if tab === 'identity'}
        <div class="card">
          <div class="card-head"><h3>Its own identity</h3></div>
          <dl class="kv">
            <dt>Agent id</dt><dd class="mono">{row.id}</dd>
            <dt>Keys</dt><dd>{row.keys_live} active of {row.keys_total}</dd>
            <dt>Role ceiling</dt><dd>{row.role_ceiling?.length ? `at most ${row.role_ceiling.join(', ')}, whoever calls it` : 'none: it can do whatever its caller can'}</dd>
            <dt>Own rules</dt><dd>{row.own_policy ? `${row.rules} rule${row.rules === 1 ? '' : 's'} in its own policy` : 'none'}</dd>
          </dl>
        </div>
        <div class="card">
          <div class="card-head"><h3>Keys</h3><span class="tag sample">sample</span></div>
          <table class="tbl"><thead><tr><th>Key</th><th>Made by</th><th>Created</th><th>Last used</th><th>Expires</th></tr></thead>
            <tbody>
              <tr><td class="mono">ak_…4f2c</td><td class="mono">{row.owner}</td><td>Nov 19</td><td>today</td><td class="warn">never</td></tr>
              <tr><td class="mono">ak_…91ad</td><td class="mono">ci-bot</td><td>Nov 02</td><td>Nov 15</td><td>Feb 01</td></tr>
            </tbody></table>
        </div>
        <div class="card">
          <div class="card-head"><h3>Acts for (delegated identities)</h3><span>last 7 days, from the decision log</span>{#if recs && !actedFor.length}<span class="tag sample">sample</span>{/if}</div>
          {#if !recs}<p class="dim">Reading…</p>
          {:else}
            <table class="tbl"><thead><tr><th>Person or app</th><th>Calls</th><th>Refused</th><th>As</th></tr></thead>
              <tbody>{#each actedRows as [who, e] (who)}<tr><td class="mono">{who}</td><td>{e.n}</td><td>{e.deny || '—'}</td><td class="dim">{[...e.via].join(', ')}</td></tr>{/each}</tbody></table>
          {/if}
        </div>
        <div class="card">
          <div class="card-head"><h3>Passing the identity on</h3><span class="tag sample">preview</span></div>
          <dl class="kv">
            <dt>To A3T tools</dt><dd>as the caller, capped by the role ceiling</dd>
            <dt>To outside services</dt><dd>with its own key; the caller's identity isn't passed on yet (needs token exchange)</dd>
          </dl>
        </div>

      {:else if tab === 'access'}
        <div class="card">
          <div class="card-head"><h3>What it can reach</h3><span>after the rules are applied</span></div>
          <table class="tbl"><thead><tr><th>Kind</th><th>What</th><th>State</th></tr></thead>
            <tbody>{#each accessRows as a, i (i)}<tr><td>{a.kind}</td><td class="mono">{a.what}</td>
              <td><span class="st {a.state.replace(' ', '-')}">{a.state}</span>{#if !a.real} <span class="tag sample">sample</span>{/if}</td></tr>{/each}</tbody></table>
          <p class="dim">MCP servers, tools, model and outside services are this agent's real config; "blocked" means a rule refused it in the last 7 days. Memory, knowledge spaces and secrets are samples until the server returns the effective access.</p>
        </div>

      {:else if tab === 'rules'}
        <div class="card">
          <div class="card-head"><h3>Org rules that apply</h3><span>{row.org} rules for agents{row.project ? ` in ${row.project}` : ''}</span>{#if !applies.length}<span class="tag sample">sample</span>{/if}<span class="grow"></span><button class="btn2" disabled>+ Rule for this agent</button></div>
          {#if true}
            <table class="tbl"><thead><tr><th>Rule</th><th>Why it applies</th><th>State</th><th>Fired, 7 days</th></tr></thead>
              <tbody>{#each appliesRows as a (a.id)}<tr><td><div>{a.say}</div><div class="file mono">{a.file}</div></td><td>{a.why}</td><td><span class="st {a.state}">{a.state}</span></td><td>{a.n || '—'}</td></tr>{/each}</tbody></table>
          {/if}
          <p class="dim">Worked out in the browser from the org's policy files; the exact list (including rules by role) needs a server endpoint.</p>
        </div>
        <div class="card">
          <div class="card-head"><h3>Its own policy</h3></div>
          <p>{row.own_policy ? `${row.rules} rule${row.rules === 1 ? '' : 's'}, written by its developer, inside the org's rules (it can only narrow them).` : 'None: only the org and platform rules apply.'}</p>
        </div>
        <div class="card">
          <div class="card-head"><h3>Platform rules</h3><span class="tag sample">sample</span></div>
          <ul class="plain"><li>Agents can't change policies or identity.</li><li>Agents can't create keys.</li><li>Calls out of A3T go through the egress gateway.</li></ul>
        </div>

      {:else if tab === 'decisions'}
        {#if recErr}<div class="note bad">{recErr}</div>{/if}
        <div class="card">
          <div class="card-head"><h3>Blocked by policy</h3><span>last 7 days</span>{#if recs && !byAction.length}<span class="tag sample">sample</span>{/if}</div>
          {#if !recs}<p class="dim">Reading…</p>
          {:else}
            <table class="tbl"><thead><tr><th>Action</th><th>Times</th><th>Rule</th><th>Last</th></tr></thead>
              <tbody>{#each blockedRows as [k, e] (k)}<tr><td class="mono">{k}</td><td>{e.n}</td><td class="mono">{e.rule}</td><td>{when(e.last)}</td></tr>{/each}</tbody></table>
          {/if}
        </div>
        <div class="card">
          <div class="card-head"><h3>Waited for approval</h3>{#if recs && !asked.length}<span class="tag sample">sample</span>{/if}</div>
          {#if !recs}<p class="dim">Reading…</p>
          {:else}
            <ul class="tl">{#each askedRows.slice(0, 50) as d, i (i)}<li class="ev"><span class="ic">?</span><div><div class="t"><span class="mono">{d.action}</span> {d.resource_id ?? ''}{d.on_behalf_of ? ` for ${d.on_behalf_of}` : ''}</div>{#if d.reason}<div class="m">{d.reason}</div>{/if}</div><time>{when(d.ts_ms)}</time></li>{/each}</ul>
          {/if}
        </div>
        <div class="card">
          <div class="card-head"><h3>Would be blocked (shadow rules)</h3><span class="tag sample">sample</span></div>
          <table class="tbl"><thead><tr><th>Action</th><th>Times</th><th>Rule</th></tr></thead>
            <tbody><tr><td class="mono">tools.call github.delete_repo</td><td>2</td><td class="mono">{row.org}/no-deletes/r1</td></tr></tbody></table>
        </div>

      {:else if tab === 'changes'}
        <div class="card">
          <div class="card-head"><h3>Changes to this agent</h3><span class="tag sample">sample</span></div>
          <ul class="tl">{#each SAMPLE_CHANGES as c, i (i)}<li class="ev"><span class="ic">✎</span><div><div class="t">{c.what}</div><div class="m mono">{c.who}</div></div><time>{c.at}</time></li>{/each}</ul>
          <p class="dim">Config, key, ceiling and policy changes; the server logs them but doesn't return them per agent yet.</p>
        </div>
      {/if}
    </section>
  </div></div></div>
</div>

<style>
  .risk { padding: 1px 8px; border-radius: 10px; font-size: 11.5px; font-weight: 600; text-transform: capitalize; white-space: nowrap; }
  .risk.high { background: color-mix(in srgb, var(--err, #b42318) 14%, transparent); color: var(--err, #b42318); }
  .risk.medium { background: color-mix(in srgb, #b45309 14%, transparent); color: #b45309; }
  .risk.low { background: color-mix(in srgb, var(--ok, #1a7158) 14%, transparent); color: var(--ok, #1a7158); }
  .why { margin: 0; color: var(--text-2); }
  .kv { grid-template-columns: 140px 1fr; }
  .tiles4 { display: grid; grid-template-columns: repeat(4, 1fr); gap: 10px; }
  .tiles4 .tile { display: flex; flex-direction: column; gap: 2px; align-items: flex-start; padding: 12px; border: 1px solid var(--border); border-radius: 10px; background: var(--bg-surface); cursor: pointer; text-align: left; }
  .tiles4 .tile b { font-size: 20px; color: var(--text-1); font-weight: 600; }
  .tiles4 .tile span { font-size: 12px; color: var(--text-3); }
  .tiles4 .tile:hover { border-color: var(--text-4); }
  .st { font: 500 11px var(--font-sans); border-radius: 999px; padding: 2px 9px; background: var(--bg-subtle); color: var(--text-3); white-space: nowrap; }
  .st.on, .st.allowed { background: color-mix(in srgb, var(--ok, #1a7158) 13%, transparent); color: var(--ok, #1a7158); }
  .st.shadow, .st.needs-approval { background: var(--warn-bg, #fbf0e0); color: var(--warn, #b45309); }
  .st.blocked { background: color-mix(in srgb, var(--err, #b42318) 13%, transparent); color: var(--err, #b42318); }
  .warn { color: var(--warn, #b45309); }
  .file { font-size: 11px; color: var(--text-4); margin-top: 2px; }
  .plain { margin: 0; padding-left: 18px; line-height: 1.7; }
  @container details (max-width: 700px) { .tiles4 { grid-template-columns: 1fr 1fr; } }
</style>
