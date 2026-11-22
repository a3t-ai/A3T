<script lang="ts">
  /// One agent seen by security (not by its builder): who it is and who it acts
  /// for, what it can reach after the rules are applied, which rules apply to
  /// it, what it was refused, and what changed — and the security actions:
  /// stop it, revoke its keys, review its access, add a rule for it.
  import { untrack } from 'svelte';
  import * as YAML from 'yaml';
  import PropertiesTree from '$lib/components/shared/PropertiesTree.svelte';
  import WizardHead from '$lib/components/shared/WizardHead.svelte';
  import '$lib/components/agent/agent.css';
  import { audit, orgPolicy, orgsApi, securityCatalogue, securityPolicies, agentSecurity, agentSpaceApi, accessReviews,
    type InventoryRow, type AgentSecurityView, type OrgPolicyFiles, type OrgInfo } from '$lib/services/api';
  import { navigateToDetail } from '$lib/stores/panels.svelte.ts';
  import { rulesOf, sentence, type Rule } from './rules/model';
  import RuleEditor from './rules/RuleEditor.svelte';

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

  let view = $state<AgentSecurityView | null>(null);
  let viewErr = $state('');
  let recs = $state<any[] | null>(null);
  let shadow = $state<any[]>([]);
  let recErr = $state('');
  let orgInfo = $state<OrgInfo | null>(null);
  let files = $state<OrgPolicyFiles | null>(null);
  let perms = $state<{ id: string; label: string }[]>([]);
  let platformRules = $state<string[]>([]);
  async function load(r: InventoryRow) {
    recs = null; recErr = ''; viewErr = '';
    const since = Date.now() - 7 * 864e5;
    const [v, a, sh, o, c, b] = await Promise.all([
      agentSecurity.get(r.id),
      audit.query({ org: r.org, agent: r.id, since_ms: since, limit: 1000 }),
      audit.query({ org: r.org, agent: r.id, action: 'policy:shadow', since_ms: since, limit: 500 }),
      orgsApi.list(), securityCatalogue(), securityPolicies.baseline(),
    ]);
    if (r.id !== row.id) return;
    if (v.ok) view = v.data ?? null; else viewErr = v.error ?? `HTTP ${v.status}`;
    if (a.ok) recs = (a.data?.records ?? []).filter((d: any) => d.action !== 'policy:shadow');
    else { recs = []; recErr = a.status === 403 ? 'You can\'t read this org\'s decisions.' : (a.error ?? 'Decisions could not be read.'); }
    shadow = sh.ok ? sh.data?.records ?? [] : [];
    if (c.ok) perms = (c.data?.permissions ?? []).map((p: any) => ({ id: p.id, label: p.label }));
    orgInfo = (o.ok ? o.data?.orgs ?? [] : []).find((x: OrgInfo) => x.id === r.org) ?? null;
    if (orgInfo) { const f = await orgPolicy.files(orgInfo.handle); files = f.ok ? f.data ?? null : null; }
    // The platform's built-in rules that bind agents, as their reasons say.
    const cedar = b.ok ? b.data?.cedar ?? '' : '';
    const out = new Set<string>();
    for (const m of cedar.matchAll(/@reason\("((?:[^"\\]|\\.)*)"\)[\s\S]*?;/g)) if (/Agent/.test(m[0])) out.add(m[1].replace(/\\"/g, '"'));
    platformRules = [...out];
  }
  $effect(() => { const r = row; untrack(() => void load(r)); });

  const label = (id: string) => perms.find((p) => p.id === id)?.label ?? (id.startsWith('tools.') ? `call ${id.slice(6)}` : id);
  const parse = (t: string): any => { try { return YAML.parse(t) ?? {}; } catch { return {}; } };
  const handle = $derived(orgInfo?.handle ?? '');

  // The org rules that bind this agent: agents in general, agents acting for a
  // role, or this agent by name — in its project or the whole org.
  type Applies = { say: string; file: string; n: number; state: string; id: string; why: string };
  const applies = $derived.by(() => {
    const out: Applies[] = [];
    for (const f of files?.files ?? []) {
      if (f.kind !== 'policy') continue;
      const y = parse(f.content);
      const named = Array.isArray(y?.applies_to?.agents) && y.applies_to.agents.includes(row.id);
      rulesOf(y).forEach((r: Rule | null, i: number) => {
        if (!r || r.callers === 'people') return;
        if (r.agents.length && !r.agents.includes(row.id)) return;
        if (!named && r.projects.length && !r.projects.includes(row.project)) return;
        const id = `${handle}/${y.name}/r${i + 1}`;
        const n = (recs ?? []).filter((d) => (d.policy_ids ?? []).some((p: string) => p.startsWith(id))).length + shadow.filter((d) => (d.policy_ids ?? []).some((p: string) => p.startsWith(id))).length;
        const why = r.agents.includes(row.id) || named ? 'names this agent' : `${r.projects.length ? `project ${row.project}` : 'whole org'} · ${r.callers === 'agents' ? (r.roles.length ? `agents acting for ${r.roles.join(', ')}` : 'any agent') : r.roles.length ? `${r.roles.join(', ')} and their agents` : 'everyone'}`;
        out.push({ say: sentence(r, label), file: f.path, n, id, state: f.enforced ? 'on' : (f as any).shadow ? 'shadow' : 'off', why });
      });
    }
    return out;
  });

  const refused = $derived((recs ?? []).filter((d) => d.decision === 'deny'));
  const asked = $derived((recs ?? []).filter((d) => d.decision === 'ask' || d.action === 'approval:request'));
  const group = (rows: any[]) => {
    const m = new Map<string, { n: number; rule: string; last: number }>();
    for (const d of rows) {
      const k = `${d.action === 'policy:shadow' ? (d.reason?.match(/would refuse (\S+)/)?.[1] ?? 'call') : d.action} ${d.resource_id ?? ''}`.trim();
      const e = m.get(k) ?? { n: 0, rule: (d.policy_ids ?? [])[0] ?? '—', last: 0 };
      e.n++; e.last = Math.max(e.last, d.ts_ms ?? 0); m.set(k, e);
    }
    return [...m.entries()].sort((a, b) => b[1].n - a[1].n);
  };
  const byAction = $derived(group(refused));
  const byShadow = $derived(group(shadow));
  // Delegated identities: the people (and apps) it acted for, from the log.
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

  const when = (ms?: number | string | null) => { const t = typeof ms === 'number' ? ms : ms ? Date.parse(ms) : NaN; return Number.isFinite(t) ? new Date(t).toLocaleString(undefined, { month: 'short', day: 'numeric', hour: '2-digit', minute: '2-digit' }) : '—'; };
  const day = (s?: string | null) => { const t = s ? Date.parse(s) : NaN; return Number.isFinite(t) ? new Date(t).toLocaleDateString(undefined, { month: 'short', day: 'numeric', year: 'numeric' }) : '—'; };
  const keys = $derived(view?.keys ?? []);
  const liveKeys = $derived(keys.filter((k) => !k.revoked_at && (!k.expires_at || Date.parse(k.expires_at) > Date.now())));

  // ── Actions ──
  let busy = $state('');
  let note = $state(''); let noteOk = $state(false);
  const say = (t: string, ok = false) => { note = t; noteOk = ok; };
  async function stop() {
    const why = prompt(`Stop ${row.name}? Its runs in flight are cancelled and calls to it are refused until an admin starts it again. Why?`);
    if (!why) return;
    busy = 'stop'; const r = await agentSpaceApi.kill(row.id, why); busy = '';
    if (!r.ok) { say(`Not stopped: ${r.error ?? `HTTP ${r.status}`}`); return; }
    say(`Stopped${r.data?.cancelled_runs?.length ? `; ${r.data.cancelled_runs.length} run(s) cancelled` : ''}.`, true);
    row = { ...row, status: 'blocked' };
  }
  async function revokeKeys() {
    if (!confirm(`Revoke all ${liveKeys.length} live key(s) of ${row.name}? Apps using them stop working at once.`)) return;
    busy = 'keys'; const r = await agentSecurity.revokeKeys(row.id); busy = '';
    if (!r.ok) { say(`Not revoked: ${r.error ?? `HTTP ${r.status}`}`); return; }
    say(`${r.data?.revoked ?? 0} key(s) revoked.`, true);
    await load(row);
  }
  async function review() {
    if (!handle) return;
    busy = 'review'; const r = await accessReviews.create(handle, `Access of ${row.name} (${new Date().toLocaleDateString()})`); busy = '';
    if (!r.ok) { say(`Not started: ${r.error ?? `HTTP ${r.status}`}`); return; }
    say(`Access review started for ${handle}: open ${handle} › Identity › Reviews to decide its keys and roles.`, true);
  }
  let addingRule = $state(false);
  const canEditRules = $derived(!!orgInfo && (orgInfo.my_role === 'admin' || orgInfo.my_role === 'securityadmin' || (orgInfo as any).can_audit));
</script>

{#if addingRule && files}
  <div class="panel-layout">
    <PropertiesTree icon="AG" color="#5530a8" name={row.name} meta={`agent · ${row.project} · new rule for it`} nodes={[]} steps={STEPS} activeNode="rules" onselect={(id) => { addingRule = false; tab = id; }} />
    <div class="panel-col"><div class="panel-content"><div class="pf"><section class="agt">
      <RuleEditor org={handle} {files} projects={files.projects ?? []} roles={['Admin', 'Operator', 'Viewer']} {perms} canEdit={canEditRules} target={null}
        seed={{ callers: 'agents', agents: [row.id], projects: [row.project] }} closeLabel={`← ${row.name}`}
        onclose={() => (addingRule = false)} onsaved={async (m) => { addingRule = false; tab = 'rules'; say(m, true); await load(row); }} />
    </section></div></div></div>
  </div>
{:else}
<div class="panel-layout">
  <PropertiesTree icon="AG" color="#5530a8" name={row.name} meta={`agent · ${row.project} · ${handle || row.org}`} nodes={[]} steps={STEPS} activeNode={tab} onselect={(id) => (tab = id)} />
  <div class="panel-col"><div class="panel-content"><div class="pf">
    <WizardHead {step} />
    <section class="agt">
      <div class="row"><button class="link" onclick={onback}>← All agents</button><span class="grow"></span>
        <button class="link" onclick={() => navigateToDetail('agents', 'Agents', row.id)}>Open in Agents app ↗</button></div>
      {#if note}<div class="note" class:ok={noteOk} class:bad={!noteOk}>{note}</div>{/if}
      {#if viewErr}<div class="note bad">This agent's security view could not be read: {viewErr}</div>{/if}

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
          <button class="tile" onclick={() => (tab = 'rules')}><b>{applies.length + (view?.policy.rules.length ?? 0)}</b><span>rules apply</span></button>
          <button class="tile" onclick={() => (tab = 'identity')}><b>{actedFor.length}</b><span>people it acted for</span></button>
        </div>
        <div class="card">
          <div class="card-head"><h3>Act on it</h3></div>
          <div class="acts">
            <button class="btn2 danger" disabled={!!busy || row.status === 'blocked'} onclick={stop}>{busy === 'stop' ? 'Stopping…' : row.status === 'blocked' ? 'Stopped' : 'Stop agent'}</button>
            <button class="btn2" disabled={!!busy || !liveKeys.length} onclick={revokeKeys}>{busy === 'keys' ? 'Revoking…' : `Revoke all keys (${liveKeys.length})`}</button>
            <button class="btn2" disabled={!!busy || !handle} onclick={review}>{busy === 'review' ? 'Starting…' : 'Start an access review'}</button>
            <button class="btn2" disabled={!files} onclick={() => (addingRule = true)}>+ Rule for this agent</button>
          </div>
          <p class="dim">Stopping keeps its config and history; calls to it are refused until an admin starts it again.</p>
        </div>

      {:else if tab === 'identity'}
        <div class="card">
          <div class="card-head"><h3>Its own identity</h3></div>
          <dl class="kv">
            <dt>Agent id</dt><dd class="mono">{row.id}</dd>
            <dt>Keys</dt><dd>{liveKeys.length} active of {keys.length}</dd>
            <dt>Role ceiling</dt><dd>{row.role_ceiling?.length ? `at most ${row.role_ceiling.join(', ')}, whoever calls it` : 'none: it can do whatever its caller can'}</dd>
            <dt>Own rules</dt><dd>{row.own_policy ? `${row.rules} rule${row.rules === 1 ? '' : 's'} in its own policy` : 'none'}</dd>
          </dl>
        </div>
        <div class="card">
          <div class="card-head"><h3>Keys</h3><span>tokens apps call it with; never shown again after they are made</span></div>
          {#if !view}<p class="dim">Reading…</p>
          {:else if !keys.length}<p class="empty">No keys: only signed-in people (and their agents) call it.</p>
          {:else}
            <table class="tbl"><thead><tr><th>Key</th><th>Made by</th><th>Created</th><th>Last used</th><th>Expires</th><th></th></tr></thead>
              <tbody>{#each keys as k (k.id)}<tr class:gone={!!k.revoked_at}><td class="mono">{k.name} · {k.prefix}…</td><td class="mono">{k.created_by}</td><td>{day(k.created_at)}</td><td>{k.last_used_at ? day(k.last_used_at) : 'never'}</td>
                <td class:warn={!k.expires_at}>{k.expires_at ? day(k.expires_at) : 'never'}</td><td>{k.revoked_at ? `revoked ${day(k.revoked_at)}` : ''}</td></tr>{/each}</tbody></table>
          {/if}
        </div>
        <div class="card">
          <div class="card-head"><h3>Acts for (delegated identities)</h3><span>last 7 days, from the decision log</span></div>
          {#if !recs}<p class="dim">Reading…</p>
          {:else if !actedFor.length}<p class="empty">No calls on someone's behalf in the last 7 days.</p>
          {:else}
            <table class="tbl"><thead><tr><th>Person or app</th><th>Calls</th><th>Refused</th><th></th></tr></thead>
              <tbody>{#each actedFor as [who, e] (who)}<tr><td class="mono">{who}</td><td>{e.n}</td><td>{e.deny || '—'}</td><td class="dim">{[...e.via].join(', ')}</td></tr>{/each}</tbody></table>
          {/if}
        </div>
        <div class="card">
          <div class="card-head"><h3>Passing the identity on</h3></div>
          <dl class="kv">
            <dt>To A3T tools</dt><dd>as the person it acts for, capped by its role ceiling; the rules see that person's sign-in (address, second factor, groups)</dd>
            <dt>Through its keys</dt><dd>{view?.acts_as.own_key ?? '—'}</dd>
            <dt>To outside services</dt><dd>{view?.acts_as.outside ?? '—'}</dd>
          </dl>
        </div>

      {:else if tab === 'access'}
        <div class="card">
          <div class="card-head"><h3>What it can reach</h3><span>decided by {handle || 'the org'}'s rules, as the agent acting for its owner</span></div>
          {#if !view}<p class="dim">Reading…</p>
          {:else if !view.access.length}<p class="empty">It is bound to no tools, servers or models.</p>
          {:else}
            <table class="tbl"><thead><tr><th>Kind</th><th>What</th><th>State</th><th>Why</th></tr></thead>
              <tbody>{#each view.access as a, i (i)}<tr><td>{a.kind}</td><td class="mono">{a.what}</td>
                <td><span class="st {a.decision.state.replace(' ', '-')}">{a.decision.state}</span></td>
                <td class="dim small">{a.decision.by?.length ? a.decision.by.join(', ') : ''}{a.decision.approver ? ` · asks ${a.decision.approver}` : ''}{!a.decision.by?.length && a.decision.why ? a.decision.why : ''}</td></tr>{/each}</tbody></table>
          {/if}
          <p class="dim">Servers, tools and its model are checked by the policy engine now (a what-if: nothing is logged). "Reached" lists what its flows call; each call is checked when made.</p>
        </div>

      {:else if tab === 'rules'}
        <div class="card">
          <div class="card-head"><h3>Org rules that apply</h3><span>{handle} rules for this agent</span><span class="grow"></span><button class="btn2" disabled={!files} onclick={() => (addingRule = true)}>+ Rule for this agent</button></div>
          {#if !files}<p class="dim">Reading…</p>
          {:else if !applies.length}<p class="empty">No org rule binds agents in {row.project}, or this agent by name.</p>
          {:else}
            <table class="tbl"><thead><tr><th>Rule</th><th>Why it applies</th><th>State</th><th>Fired, 7 days</th></tr></thead>
              <tbody>{#each applies as a (a.id)}<tr><td><div>{a.say}</div><div class="file mono">{a.file}</div></td><td>{a.why}</td><td><span class="st {a.state}">{a.state}</span></td><td>{a.n || '—'}</td></tr>{/each}</tbody></table>
          {/if}
          <p class="dim">Rules for people by role bind this agent too when it acts for someone with that role.</p>
        </div>
        <div class="card">
          <div class="card-head"><h3>Its own policy</h3></div>
          {#if view?.policy.set && view.policy.rules.length}<ul class="plain">{#each view.policy.rules as x (x)}<li>{x}</li>{/each}</ul>
          {:else}<p>{view?.policy.set ? 'A ceiling or limits, no rules of its own.' : 'None: only the org and platform rules apply.'}</p>{/if}
          <p class="dim">Written by its developer, inside the org's rules: it can only narrow them.</p>
        </div>
        <div class="card">
          <div class="card-head"><h3>Platform rules</h3><span>built in; every agent</span></div>
          {#if platformRules.length}<ul class="plain">{#each platformRules as x (x)}<li>{x}</li>{/each}</ul>{:else}<p class="dim">Reading…</p>{/if}
        </div>

      {:else if tab === 'decisions'}
        {#if recErr}<div class="note bad">{recErr}</div>{/if}
        <div class="card">
          <div class="card-head"><h3>Blocked by policy</h3><span>last 7 days</span></div>
          {#if !recs}<p class="dim">Reading…</p>
          {:else if !byAction.length}<p class="empty">Nothing refused in the last 7 days.</p>
          {:else}
            <table class="tbl"><thead><tr><th>Action</th><th>Times</th><th>Rule</th><th>Last</th></tr></thead>
              <tbody>{#each byAction as [k, e] (k)}<tr><td class="mono">{k}</td><td>{e.n}</td><td class="mono">{e.rule}</td><td>{when(e.last)}</td></tr>{/each}</tbody></table>
          {/if}
        </div>
        <div class="card">
          <div class="card-head"><h3>Waited for approval</h3></div>
          {#if !recs}<p class="dim">Reading…</p>
          {:else if !asked.length}<p class="empty">No call waited for approval.</p>
          {:else}
            <ul class="tl">{#each asked.slice(0, 50) as d, i (i)}<li class="ev"><span class="ic">?</span><div><div class="t"><span class="mono">{d.action}</span> {d.resource_id ?? ''}{d.on_behalf_of ? ` for ${d.on_behalf_of}` : ''}</div>{#if d.reason}<div class="m">{d.reason}</div>{/if}</div><time>{when(d.ts_ms)}</time></li>{/each}</ul>
          {/if}
        </div>
        <div class="card">
          <div class="card-head"><h3>Would be blocked</h3><span>shadow rules and rules that only log</span></div>
          {#if !byShadow.length}<p class="empty">Nothing in the last 7 days.</p>
          {:else}
            <table class="tbl"><thead><tr><th>Call</th><th>Times</th><th>Rule</th><th>Last</th></tr></thead>
              <tbody>{#each byShadow as [k, e] (k)}<tr><td class="mono">{k}</td><td>{e.n}</td><td class="mono">{e.rule}</td><td>{when(e.last)}</td></tr>{/each}</tbody></table>
          {/if}
        </div>

      {:else if tab === 'changes'}
        <div class="card">
          <div class="card-head"><h3>Changes to this agent</h3><span>its record, keys and own policy</span></div>
          {#if !view}<p class="dim">Reading…</p>
          {:else if !view.changes.length}<p class="empty">No changes recorded.</p>
          {:else}
            <ul class="tl">{#each view.changes.slice(0, 100) as c, i (i)}<li class="ev"><span class="ic">✎</span><div>
              <div class="t">{c.what}{c.key ? ` · ${c.key}` : ''}{c.version ? ` · ${c.version}` : ''}{c.changed?.length ? `: ${c.changed.join(', ')}` : ''}</div>
              {#if c.by || c.approved_by}<div class="m mono">{c.by ?? ''}{c.approved_by ? ` · approved by ${c.approved_by}` : ''}</div>{/if}</div><time>{when(c.at)}</time></li>{/each}</ul>
          {/if}
        </div>
      {/if}
    </section>
  </div></div></div>
</div>
{/if}

<style>
  .risk { padding: 1px 8px; border-radius: 10px; font-size: 11.5px; font-weight: 600; text-transform: capitalize; white-space: nowrap; }
  .risk.high { background: color-mix(in srgb, var(--err, #b42318) 14%, transparent); color: var(--err, #b42318); }
  .risk.medium { background: color-mix(in srgb, #b45309 14%, transparent); color: #b45309; }
  .risk.low { background: color-mix(in srgb, var(--ok, #1a7158) 14%, transparent); color: var(--ok, #1a7158); }
  .why { margin: 0; color: var(--text-2); }
  .kv { grid-template-columns: 140px 1fr; }
  .small { font-size: 11.5px; }
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
  tr.gone td { color: var(--text-4); text-decoration: line-through; }
  .file { font-size: 11px; color: var(--text-4); margin-top: 2px; }
  .plain { margin: 0; padding-left: 18px; line-height: 1.7; }
  .note.ok { border-color: color-mix(in srgb, var(--ok, #1a7158) 40%, var(--border)); }
  @container details (max-width: 700px) { .tiles4 { grid-template-columns: 1fr 1fr; } }
</style>
