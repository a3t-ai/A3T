// What is happening with one agent — its serving flow's runs, decisions about
// it, your test conversations with it, and its deployment — loaded once and
// shared by every tab. The page makes one per agent (and again after it
// changes the deployment).
import {
  factsOf, servingRuns, decisionsFor, conversationsWith, placementOf,
  type AgentRec, type TemplateRec, type Run, type Decision, type Placement,
} from './agentData';
import { mcpCall, type ChatSessionSummary } from '$lib/services/api';
import { discoverServerTools, isDestructive, type ServerTools } from '$lib/services/mcpTools';

/// Its endpoint's trigger: what serves its webhook path, and whether it is on.
export type Trigger = { id: string; name: string; path: string; enabled: boolean };

const DAY = 86_400_000;

export class AgentLive {
  runs = $state<Run[]>([]);
  runsErr = $state('');
  decisions = $state<Decision[]>([]);
  decErr = $state('');
  scanned = $state(0);
  convs = $state<ChatSessionSummary[]>([]);
  placement = $state<Placement | null>(null);
  placeErr = $state('');
  loading = $state(true);
  /// Its tool servers' tools (discovered), and its endpoint's trigger.
  tools = $state<ServerTools[]>([]);
  toolsLoading = $state(true);
  trigger = $state<Trigger | null>(null);

  /// Its serving flow's runs are shared by every agent on that flow (UI-2), so
  /// they describe this agent only once it is deployed on its own.
  ownRuns = $derived(!!this.placement);
  failed = $derived(this.runs.filter((r) => ['failed', 'error', 'cancelled'].includes(r.status)));
  denied = $derived(this.decisions.filter((d) => d.decision === 'deny'));
  runsDay = $derived(this.runs.filter((r) => Date.now() - Date.parse(r.started) < DAY));
  lastSeen = $derived.by(() => {
    const ts = [...(this.ownRuns ? this.runs.map((r) => Date.parse(r.started)) : []), ...this.convs.map((c) => Date.parse(c.updated_at)), ...this.decisions.map((d) => d.ts)].filter((x) => x > 0);
    return ts.length ? Math.max(...ts) : 0;
  });
  health = $derived.by((): { tone: 'ok' | 'warn' | 'err' | 'idle'; word: string } => {
    if (this.loading) return { tone: 'idle', word: 'Checking…' };
    const p = this.placement;
    if (!p) return { tone: 'idle', word: 'Not deployed' };
    if (p.deploymentState === 'paused') return { tone: 'warn', word: 'Paused' };
    const er = p.stats?.errorRate;
    if (er != null && er > 0.2) return { tone: 'err', word: 'Failing' };
    if (er != null && er > 0.02) return { tone: 'warn', word: 'Some errors' };
    return { tone: 'ok', word: 'Serving' };
  });

  /// Tools it may call: every tool of its servers, or its allow-list.
  allowedTools = $derived.by(() => {
    const all = this.tools.flatMap((t) => t.tools);
    const allow = this.allow;
    return allow ? all.filter((t) => allow.includes(t)) : all;
  });
  destructive = $derived(this.allowedTools.filter(isDestructive));
  private allow = $state<string[] | null>(null);

  constructor(private agent: AgentRec, private templates: TemplateRec[]) {
    // After the caller's reaction: it is made inside a derived value, where
    // writing state is not allowed.
    queueMicrotask(() => void this.load());
  }

  async load() {
    this.loading = true;
    const f = factsOf(this.agent, this.templates);
    this.allow = f.tools;
    void this.loadTools(f.mcp);
    void this.loadTrigger(f.endpoint);
    const [r, d, c, p] = await Promise.all([
      servingRuns(f), decisionsFor(this.agent), conversationsWith(this.agent).catch(() => []), placementOf(this.agent, f),
    ]);
    this.runs = r.runs; this.runsErr = r.error;
    this.decisions = d.list; this.decErr = d.error; this.scanned = d.scanned;
    this.convs = c;
    this.placement = p.placement; this.placeErr = p.error;
    this.loading = false;
  }

  async loadTools(servers: string[]) {
    this.toolsLoading = true;
    this.tools = servers.length ? await discoverServerTools(servers, this.agent.namespace || 'a3t').catch(() => []) : [];
    this.toolsLoading = false;
  }

  async loadTrigger(ep: string) {
    const r = await mcpCall('list_triggers', {});
    let d: any = r.data; if (typeof d === 'string') { try { d = JSON.parse(d); } catch { d = []; } }
    const list: any[] = Array.isArray(d) ? d : d?.triggers ?? [];
    const t = list.find((x) => x?.name === `agent-${this.agent.id}-chat`) ?? (ep ? list.find((x) => x?.config?.path === ep) : null);
    this.trigger = t ? { id: t.id, name: t.name, path: t.config?.path ?? '', enabled: t.enabled !== false && t.status !== 'paused' } : null;
  }
}

/// One loader per agent and deployment revision; asking again returns the same.
const cache = new Map<string, AgentLive>();
export function liveFor(agent: AgentRec, templates: TemplateRec[], rev: number): AgentLive {
  // The record's values are in the key: a saved change (tools revoked, the
  // endpoint moved) reloads.
  const key = `${agent.id}|${templates.length}|${rev}|${agent.variables ?? ''}|${agent.resource_bindings ?? ''}|${agent.mode}`;
  let l = cache.get(key);
  if (!l) {
    for (const k of cache.keys()) if (k.startsWith(`${agent.id}|`)) cache.delete(k);
    l = new AgentLive(agent, templates);
    cache.set(key, l);
  }
  return l;
}
