// One implementation of "what are this app's items", shared by the context list
// and the app home page.
//
// These lived inside ContextList. The home page needs exactly the same rows, and
// a second copy would drift — which is the failure mode that already produced two
// disagreeing resource-category counts.

import { mcpCall, fetchChatSessions, opsApi, canvasApi } from '$lib/services/api';
import { RESOURCE_CATEGORIES, categorize } from '$lib/core/resources/categories';
import { namespacesFor, inContext, canvasKind, projectOf, itemProject, getProjects, getShowPlatform, getProfile, loadProjects, canSee, roleIn, type ListKind } from '$lib/stores/context.svelte.ts';

export type AppItem = {
  /// What navigation addresses. For chat this is the *client* conversation id,
  /// which is what `/chat/<id>` uses.
  id: string;
  label: string;
  sub?: string;
  /// The full server key, kept for chat so delete hits the right transcript.
  serverId?: string;
  /// Single-letter badge + its colours, for the fixed resource categories.
  badge?: string;
  badgeStyle?: string;
  /// A heading row in a tree: picking it opens or closes its branch.
  group?: boolean;
  /// The Items group it sits in, where that isn't its namespace (Projects: the
  /// namespace that owns it, since each project is its own namespace).
  section?: string;
  /// A conversation that exists only in this browser so far.
  pending?: boolean;
  /// Registry namespace. The list GROUPS by this, so it is kept as data rather
  /// than baked into `sub` — repeating it on every row under its own heading
  /// was most of the visual noise.
  namespace?: string;
  /// Short state shown as a coloured dot (execution status, agent status).
  status?: string;
  /// Tree position for hierarchical lists (host → runtime → deployment). Items
  /// arrive in depth-first order; a row's children are the following rows with a
  /// greater depth. Absent for flat lists.
  depth?: number;
};

// Which namespaces a list loads comes from the working context (project + "show
// platform"), so every list agrees — querying without one returns only `default`.

/// Server session ids are "<agent>:<clientId>"; the agent half is often empty.
/// Never drop an entry over a missing prefix — that bug hid every conversation.
export function splitServerId(sid: string): { agent: string; id: string } {
  const i = sid.indexOf(':');
  if (i < 0) return { agent: '', id: sid };
  const id = sid.slice(i + 1);
  if (!id) return { agent: '', id: sid };
  return { agent: sid.slice(0, i), id };
}

export function relTime(iso: string): string {
  const t = Date.parse(iso);
  if (!Number.isFinite(t)) return '';
  const mins = Math.floor((Date.now() - t) / 60000);
  if (mins < 1) return 'now';
  if (mins < 60) return `${mins}m ago`;
  const hrs = Math.floor(mins / 60);
  if (hrs < 24) return `${hrs}h ago`;
  const days = Math.floor(hrs / 24);
  return days < 30 ? `${days}d ago` : new Date(t).toLocaleDateString();
}

/// Pull an array out of whatever shape a tool returns, without assuming one.
function rows(data: any, ...keys: string[]): any[] {
  if (Array.isArray(data)) return data;
  for (const k of keys) if (Array.isArray(data?.[k])) return data[k];
  for (const v of Object.values(data ?? {})) if (Array.isArray(v)) return v as any[];
  return [];
}

const named = (r: any): AppItem => ({
  id: r.name ?? r.id ?? String(r),
  label: r.name ?? r.id ?? String(r),
  namespace: r.namespace || undefined,
  // Namespace is the group heading now, so the subtitle carries what differs
  // between siblings: version, then type.
  sub: [r.version ? `v${r.version}` : '', r.type_ref ?? r.type ?? ''].filter(Boolean).join(' · '),
  status: r.status || undefined,
});

/// Load one app's items. Throws with a readable message so callers can surface a
/// failure rather than rendering an empty list that looks like "nothing here".
/// `document.namespace` from a CNCF workflow's YAML, without a YAML parser: the
/// first `namespace:` inside the top-level `document:` block.
function specNamespace(yaml: unknown): string | undefined {
  if (typeof yaml !== 'string') return undefined;
  const doc = yaml.split(/\n(?=\S)/).find((block) => block.startsWith('document:'));
  return doc?.match(/^\s+namespace:\s*['"]?([\w.-]+)/m)?.[1];
}

/// Group items by namespace for display, keeping each group's original order.
///
/// Namespaces in `PREFERRED` come first in that order (the platform's own
/// namespace before user ones), the rest alphabetically; items without a
/// namespace collect under `''`, rendered last. Returns `null` when nothing in
/// the list has a namespace — grouping a flat list would only add a heading.
const PREFERRED = ['a3t', 'default', 'system'];
/// `keepOrder`: groups in the order their items come (Settings' own order),
/// not projects-first and alphabetical.
export function groupByNamespace(items: AppItem[], keepOrder = false): { ns: string; items: AppItem[] }[] | null {
  if (!items.some((i) => i.section ?? i.namespace)) return null;
  const map = new Map<string, AppItem[]>();
  for (const it of items) {
    const k = it.section ?? it.namespace ?? '';
    (map.get(k) ?? map.set(k, []).get(k)!).push(it);
  }
  const rank = (ns: string) => {
    if (!ns) return [2, ''];
    const i = PREFERRED.indexOf(ns);
    return i >= 0 ? [0, String(i)] : [1, ns];
  };
  if (keepOrder) return [...map.entries()].map(([ns, list]) => ({ ns, items: list }));
  return [...map.entries()]
    .sort(([a], [b]) => {
      const [ra, ka] = rank(a), [rb, kb] = rank(b);
      return ra !== rb ? (ra as number) - (rb as number) : (ka as string).localeCompare(kb as string);
    })
    .map(([ns, items]) => ({ ns, items }));
}

/// Lists that come back whole (not per namespace) and are filtered to the context.
const FILTERED: Record<string, ListKind> = {
  // Agents were stored in `a3t` by default, so they aren't platform internals:
  // they stay visible with the platform hidden, split My/Shared by creator.
  agents: 'shared', 'memory-agents': 'shared', storage: 'shared', 'kg-dbs': 'shared', templates: 'shared', triggers: 'owned', executions: 'owned',
};

export async function loadAppItems(source: string): Promise<AppItem[]> {
  const items = await loadAll(source);
  const kind = FILTERED[source];
  return kind ? items.filter((i) => inContext(i.namespace, kind)) : items;
}

async function loadAll(source: string): Promise<AppItem[]> {
  if (source === 'sessions') {
    const remote = await fetchChatSessions();
    return remote.map((s) => ({
      id: splitServerId(s.session_id).id,
      label: s.title || 'Conversation',
      sub: `${relTime(s.updated_at)}${s.message_count ? ` · ${s.message_count} msgs` : ''}`,
      serverId: s.session_id,
    }));
  }

  if (source === 'projects') {
    await loadProjects();
    // Grouped by the namespace that owns them — your handle, then your orgs,
    // then the platform's — with each owner's projects folded under it.
    const me = getProfile()?.handle ?? '';
    const owner = (p: { name: string; info?: { owner?: string } }) =>
      p.info?.owner || (p.name.includes('-') ? p.name.slice(0, p.name.indexOf('-')) : p.name);
    const rank = (p: { name: string; platform?: boolean; info?: { owner?: string } }) =>
      p.platform ? 2 : owner(p) === me ? 0 : 1;
    return getProjects()
      .filter((p) => (p.platform ? getShowPlatform() : canSee(p)))
      .sort((a, b) => rank(a) - rank(b) || owner(a).localeCompare(owner(b)) || a.name.localeCompare(b.name))
      .map((p) => ({
        id: p.name,
        label: p.info?.display_name || p.name,
        namespace: p.name,
        section: owner(p),
        // Its namespace too, when the display name hides it ("Crawler" is adgn-crawler).
        sub: [(p.info?.display_name && p.info.display_name !== p.name) ? p.name : '', roleIn(p) ?? (p.platform ? 'platform' : 'open to everyone'),
              `${p.flows ?? 0} flows · ${p.resources ?? 0} resources`].filter(Boolean).join(' · '),
      }));
  }
  if (source === 'flows') {
    // Platform namespaces too: users' flows saved there before projects are
    // `default`'s (itemProject); the filter below keeps the selected project's.
    const out: AppItem[] = [];
    for (const ns of namespacesFor('picker')) {
      const res = await mcpCall('list_flows', { namespace: ns });
      if (!res.ok) continue;
      for (const r of rows(res.data, 'workflows', 'flows')) {
        const project = itemProject('flow', r.namespace ?? ns, { name: r.name });
        if (!inContext(project, 'owned')) continue;
        out.push({
          // "~" not "/": the id becomes a single URL path segment.
          id: `${r.namespace ?? ns}~${r.name}`,
          label: r.name,
          namespace: project,
          sub: [r.version ? `v${r.version}` : '',
                r.step_count != null ? `${r.step_count} step${r.step_count === 1 ? '' : 's'}` : '']
            .filter(Boolean).join(' · '),
          status: r.enabled === false ? 'disabled' : undefined,
        });
      }
    }
    return out;
  }

  if (source === 'resources') {
    // A tree like Runtimes: each kind, then its resources. Every namespace is
    // asked at once — one after another left the list blank for as long as the
    // slowest project took — and a namespace that doesn't answer is skipped.
    // Id `<category>:<namespace>/<name>`: the Resources page opens that one.
    const nss = namespacesFor('picker');
    const [resLists, flowLists] = await Promise.all([
      Promise.all(nss.map((ns) => within(mcpCall('list_resources', { namespace: ns })).then((r) => [ns, r] as const))),
      Promise.all(nss.map((ns) => within(mcpCall('list_flows', { namespace: ns })).then((r) => [ns, r] as const))),
    ]);
    const byCat: Record<string, AppItem[]> = {};
    for (const [ns, res] of resLists) {
      if (!res?.ok) continue;
      for (const r of rows(res.data, 'resources')) {
        if (r.type === 'project') continue; // the Projects app's records
        const project = itemProject('resource', r.namespace ?? ns, r);
        if (!inContext(project, 'shared')) continue;
        const c = categorize(r.type ?? '');
        (byCat[c] ??= []).push({ id: `${c}:${r.namespace ?? ns}/${r.name}`, label: r.name, depth: 1,
          sub: [project, r.type].filter(Boolean).join(' · ') });
      }
    }
    for (const [ns, fl] of flowLists) {
      if (!fl?.ok) continue;
      for (const f of rows(fl.data, 'workflows', 'flows')) {
        const project = itemProject('flow', f.namespace ?? ns, f);
        if (!inContext(project, 'owned')) continue;
        (byCat.flows ??= []).push({ id: `flows:${f.namespace ?? ns}/${f.name}`, label: f.name, depth: 1,
          sub: [project, f.version ? `v${f.version}` : ''].filter(Boolean).join(' · '), status: f.enabled === false ? 'disabled' : undefined });
      }
    }
    const known = new Set<string>(RESOURCE_CATEGORIES.map((c) => c.key));
    const kinds = [
      ...RESOURCE_CATEGORIES.filter((c) => c.key !== 'all').map((c) => ({ key: c.key, label: c.label, icon: c.icon, style: c.iconStyle })),
      // Kinds the categories don't name yet, so nothing goes missing.
      ...Object.keys(byCat).filter((k) => !known.has(k)).map((k) => ({ key: k, label: k === 'other' ? 'Other' : k, icon: '?', style: 'background:var(--bg-subtle);color:var(--text-4)' })),
    ];
    const out: AppItem[] = [];
    for (const k of kinds) {
      const ids = new Set<string>();
      const list = (byCat[k.key] ?? []).filter((i) => !ids.has(i.id) && !!ids.add(i.id))
        .sort((a, b) => a.label.localeCompare(b.label));
      if (!list.length) continue;
      out.push({ id: `kind:${k.key}`, label: k.label, badge: k.icon, badgeStyle: k.style, depth: 0, group: true,
        sub: `${list.length} ${list.length === 1 ? 'item' : 'items'}` });
      out.push(...list);
    }
    return out;
  }
  if (source === 'resource-categories') {
    // Fixed categories, always all shown — a zero is information. Counts come from
    // the same two sources the Resources page merges.
    const counts: Record<string, number> = {};
    for (const ns of namespacesFor('picker')) {
      const res = await mcpCall('list_resources', { namespace: ns });
      if (!res.ok) continue;
      for (const r of rows(res.data, 'resources')) {
        if (r.type === 'project') continue; // the Projects app's records
        if (!inContext(itemProject('resource', r.namespace ?? ns, r), 'shared')) continue;
        const c = categorize(r.type ?? '');
        counts[c] = (counts[c] ?? 0) + 1;
      }
    }
    let flowCount = 0;
    for (const ns of namespacesFor('picker')) {
      const fl = await mcpCall('list_flows', { namespace: ns });
      if (fl.ok) flowCount += rows(fl.data, 'workflows', 'flows')
        .filter((f: any) => inContext(itemProject('flow', f.namespace ?? ns, f), 'owned')).length;
    }
    counts.flows = flowCount;
    counts.all = Object.entries(counts).filter(([k]) => k !== 'all')
      .reduce((n, [, v]) => n + v, 0);
    return RESOURCE_CATEGORIES.map((c) => ({
      id: c.key, label: c.label, sub: `${counts[c.key] ?? 0}`,
      badge: c.icon, badgeStyle: c.iconStyle,
    }));
  }

  if (source === 'storage') {
    // Where files live: storage resources, by driver.
    const nss = namespacesFor('picker');
    const lists = await Promise.all(nss.map((ns) => within(mcpCall('list_resources', { namespace: ns })).then((r) => [ns, r] as const)));
    const out: AppItem[] = [];
    for (const [ns, res] of lists) {
      if (!res?.ok) continue;
      for (const r of rows(res.data, 'resources')) {
        const t = String(r.type ?? '').toLowerCase();
        if (!['s3', 'minio', 'gcs', 'azure', 'ftp'].includes(t)) continue;
        const project = itemProject('resource', r.namespace ?? ns, r);
        if (!inContext(project, 'shared')) continue;
        const c = r.config ?? {};
        out.push({ id: `${r.namespace ?? ns}~${r.name}`, label: r.name, namespace: project,
          sub: [t === 'ftp' ? 'FTP' : t === 'gcs' ? 'Google Cloud' : t === 'azure' ? 'Azure' : t === 'minio' ? 'MinIO' : 'S3',
                `${c.bucket ?? c.container ?? c.host ?? ''}${c.prefix ? '/' + c.prefix : ''}`].filter(Boolean).join(' · ') });
      }
    }
    return out;
  }
  if (source === 'kg-dbs') {
    // The knowledge-graph databases: LanceDB resources that are KGs (kgApi.isKg).
    const { isKg } = await import('$lib/components/knowledge/kgApi');
    const nss = namespacesFor('picker');
    const lists = await Promise.all(nss.map((ns) => within(mcpCall('list_resources', { namespace: ns })).then((r) => [ns, r] as const)));
    const out: AppItem[] = [];
    for (const [ns, res] of lists) {
      if (!res?.ok) continue;
      for (const r of rows(res.data, 'resources')) {
        if (!isKg({ type: r.type, name: r.name, config: r.config })) continue;
        const project = itemProject('resource', r.namespace ?? ns, r);
        if (!inContext(project, 'shared')) continue;
        out.push({ id: `${r.namespace ?? ns}~${r.name}`, label: r.name, namespace: project, sub: r.config?.source_space ? `from ${r.config.source_space}` : 'knowledge graph' });
      }
    }
    return out;
  }
  if (source === 'memory-agents') {
    // The agents, for their memory: each one's mode says how long it remembers.
    const res = await mcpCall('list_agents', {});
    if (!res.ok) throw new Error(res.error ?? 'list_agents failed');
    return rows(res.data, 'agents').map((r: any) => ({ ...named(r), id: r.id ?? r.name, namespace: projectOf(r.namespace || undefined, r.created_by),
      sub: (r.mode ?? r.agent_mode) === 'persistent' ? 'remembers across conversations' : 'remembers one conversation' }));
  }
  if (source === 'agents') {
    const res = await mcpCall('list_agents', {});
    if (!res.ok) throw new Error(res.error ?? 'list_agents failed');
    // Keyed by the agent's id: two projects may each have an agent of the same name.
    return rows(res.data, 'agents').map((r: any) => ({ ...named(r), id: r.id ?? r.name, namespace: projectOf(r.namespace || undefined, r.created_by) }));
  }
  if (source === 'templates') {
    const res = await mcpCall('list_agent_templates', {});
    if (!res.ok) throw new Error(res.error ?? 'list_agent_templates failed');
    // Template rows have no namespace column — it lives in the definition
    // (`document.namespace` in spec_yaml). Read it from there so templates group
    // like everything else, and show where the template came from.
    return rows(res.data, 'templates').map((r: any) => ({
      ...named(r),
      namespace: projectOf(r.namespace || specNamespace(r.spec_yaml), r.created_by),
      sub: [r.version ? `v${r.version}` : '', r.source === 'system' ? 'built-in' : r.source === 'my' ? 'yours' : r.source ?? '']
        .filter(Boolean).join(' · '),
    }));
  }
  if (source === 'settings-sections') {
    // Every Settings page, under its group's heading.
    const { NAV } = await import('$lib/components/settings/nav');
    return NAV.flatMap((g) => g.items.map((it) => ({
      id: it.key, label: it.label, namespace: g.label, sub: it.ee ? 'Enterprise' : '',
    })));
  }
  if (source === 'decision-views') {
    // SuperAdmin and SecurityAdmin only (the server refuses anyone else): say so
    // here, as Security does, rather than list views that can only fail.
    const { isInfosec } = await import('$lib/stores/auth');
    if (!isInfosec()) throw new Error('The decision log is for SuperAdmin and SecurityAdmin.');
    return [
      { id: 'all', label: 'All decisions', sub: 'every allow, deny and approval asked' },
      { id: 'deny', label: 'Denials', sub: 'what was refused, and why' },
      { id: 'ask', label: 'Approvals asked', sub: 'what waited for a person' },
    ];
  }
  if (source === 'security-sections') {
    // Fixed sections, like Resources' categories.
    // Identity (the SSO realm, platform-wide), then each project's Cedar policies
    // — only the projects in view — and the compiled baseline. InfoSec only: the
    // server answers 403 to anyone else, and the menu does not offer it.
    const { securityPolicies } = await import('$lib/services/api');
    const out: AppItem[] = [{ id: 'identity', label: 'Identity', sub: 'sign-in provider and users · platform-wide', namespace: 'platform' }];
    const r = await securityPolicies.list();
    if (r.status === 403) throw new Error('Security is for SuperAdmin and SecurityAdmin.');
    for (const p of (r.ok ? r.data?.policies ?? [] : []) as any[]) {
      if (!inContext(p.namespace, 'shared')) continue;
      out.push({ id: `policy:${p.namespace}/${p.name}`, label: p.name, namespace: p.namespace, sub: p.description || 'Cedar policy' });
    }
    out.push({ id: 'baseline', label: 'baseline', namespace: 'platform', sub: 'compiled deny-by-default guardrails · read only' });
    return out;
  }
  if (source === 'canvases') {
    const res = await canvasApi.list();
    if (!res.ok) throw new Error(res.error || 'Could not load canvases');
    return rows(res.data, 'resources')
      .map((r: any) => ({ ...r, project: itemProject('resource', r.namespace || undefined, { ...r, type: 'canvas' }) }))
      .filter((r: any) => inContext(r.project, canvasKind(r))).map((r: any) => {
      const c = r.config ?? {};
      return {
        id: r.name, label: r.name, namespace: r.project,
        sub: c.flowRef ? `deploys ${c.flowRef}` : (c.shared === false ? 'your copy · unbound' : 'preset'),
        status: c.flowRef ? 'active' : undefined,
      };
    });
  }
  if (source === 'runtimes') {
    return runtimeTree();
  }
  if (source === 'triggers') {
    const res = await mcpCall('list_triggers', {});
    if (!res.ok) throw new Error(res.error ?? 'list_triggers failed');
    return rows(res.data, 'triggers').map((r: any) => ({
      id: r.id ?? r.name, label: r.name ?? r.id, sub: r.config?.type ?? r.type ?? '',
      namespace: itemProject('trigger', r.namespace || undefined, r), status: r.status || undefined,
    }));
  }
  if (source === 'executions') {
    // The run store, newest first; the shell narrows it to the top bar's project.
    const res = await mcpCall('list_runs', { limit: 200 });
    if (!res.ok) throw new Error(res.error ?? 'list_runs failed');
    const PH: Record<string, string> = { faulted: 'failed', pending: 'queued', suspended: 'paused' };
    return rows(res.data, 'runs').map((r: any) => ({
      id: r.run_id, label: r.flow ?? r.run_id,
      namespace: itemProject('execution', r.namespace || undefined, r),
      sub: [r.trigger, r.started_at || r.created_at ? relTime(r.started_at ?? r.created_at) : '', r.duration_us ? dur(r.duration_us) : '', r.error ? String(r.error).slice(0, 80) : ''].filter(Boolean).join(' · '),
      status: PH[r.phase] ?? r.phase,
    }));
  }
  return [];
}

/// The Runtime console's drill-down: host → runtime → deployed agents and flows.
///
/// Ids are typed (`host:` / `rt:` / `dep:`) because the page shows a different
/// view for each level. In CE every runtime lives in the one flowr pod, so there is
/// a single host; with the EE controller each runtime gets its own pods, and the
/// same tree will show them under their own hosts.
const DEP_STATUS: Record<string, string> = {
  serving: 'active', paused: 'paused', 'runtime-paused': 'paused', hibernated: 'hibernated', unplaced: 'degraded',
};
const BADGE = {
  host: 'background:#e8eef6;color:#3a5a85',
  runtime: 'background:var(--runtime-bg,#e6f4f1);color:var(--runtime,#0f766e)',
  flow: 'background:var(--spec-bg,#ede7ff);color:var(--spec,#5530a8)',
  agent: 'background:var(--ephemeral-bg,#fff4e5);color:var(--ephemeral,#b45309)',
};
/// A list call that hasn't answered in time counts as failed, so one slow
/// namespace can't hold the whole list back.
function within<T>(p: Promise<T>, ms = 12000): Promise<T | null> {
  return Promise.race([p, new Promise<null>((r) => setTimeout(() => r(null), ms))]);
}

function dur(us: number): string {
  const ms = us / 1000;
  return ms < 1000 ? `${Math.round(ms)} ms` : ms < 60000 ? `${(ms / 1000).toFixed(1)} s` : `${Math.round(ms / 60000)} min`;
}

async function runtimeTree(): Promise<AppItem[]> {
  const res = await opsApi.overview();
  if (!res.ok) throw new Error(res.error || 'Could not load runtimes');
  const d = res.data ?? {};
  const host = d.host?.host || 'flowr';
  const out: AppItem[] = [{
    id: `host:${host}`, label: host, badge: 'H', badgeStyle: BADGE.host, depth: 0,
    sub: `shared host · ${(d.runtimes ?? []).length} runtimes`,
  }];
  const dep = (row: any, depth: number): AppItem => ({
    id: `dep:${row.deployment.id}`,
    label: row.deployment.target,
    badge: row.deployment.kind === 'agent' ? 'A' : 'F',
    badgeStyle: row.deployment.kind === 'agent' ? BADGE.agent : BADGE.flow,
    sub: `${row.deployment.kind} · ${row.serving.replace('-', ' ')}`,
    status: DEP_STATUS[row.serving],
    depth,
  });
  for (const r of d.runtimes ?? []) {
    const labels = Object.entries(r.runtime.labels ?? {}).map(([k, v]) => `${k}=${v}`).join(' ');
    out.push({
      id: `rt:${r.runtime.name}`, label: r.runtime.name, badge: 'R', badgeStyle: BADGE.runtime, depth: 1,
      sub: [`${r.deployments.length} deployed`, labels].filter(Boolean).join(' · '),
      status: r.state,
    });
    for (const row of r.deployments) out.push(dep(row, 2));
  }
  if ((d.unplaced ?? []).length) {
    out.push({ id: 'rt:__unplaced', label: 'Unplaced', badge: '?', badgeStyle: BADGE.host, depth: 1, sub: 'runtime was removed', status: 'degraded' });
    for (const row of d.unplaced) out.push(dep(row, 2));
  }
  return out;
}
