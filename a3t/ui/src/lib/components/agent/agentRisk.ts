// What a security or governance reviewer should notice about an agent, each
// with the tab that shows the evidence. Computed from what the console can
// read today; a flag is only raised on evidence, never guessed.
import { factsOf, envOf, type AgentRec, type TemplateRec } from './agentData';
import type { AgentLive } from './agentLive.svelte';

export type Flag = { level: 'high' | 'medium' | 'info'; title: string; why: string; tab: string };

export function flagsOf(agent: AgentRec & { created_by?: string }, templates: TemplateRec[], live: AgentLive, llmLabel?: string): Flag[] {
  const f = factsOf(agent, templates);
  const out: Flag[] = [];
  if (f.endpoint && live.trigger?.enabled !== false) {
    out.push({ level: 'high', title: 'Callable without a token', why: `POST /webhook${f.endpoint} accepts calls with no user or app key (SRV-16).`, tab: 'exposure' });
  }
  if (!agent.created_by) out.push({ level: 'high', title: 'No owner', why: 'Nobody is accountable for it; what it may be granted has no bound.', tab: 'settings' });
  if (live.destructive.length) {
    out.push({ level: 'high', title: `${live.destructive.length} destructive tool${live.destructive.length === 1 ? '' : 's'} allowed`, why: live.destructive.slice(0, 4).join(', ') + (live.destructive.length > 4 ? '…' : ''), tab: 'access' });
  }
  if (f.mcp.length && !f.tools) out.push({ level: 'medium', title: 'Tools not restricted', why: `Every tool of ${f.mcp.length} server${f.mcp.length === 1 ? '' : 's'} may run; only policies stand in the way.`, tab: 'access' });
  if (live.denied.length) out.push({ level: 'medium', title: `${live.denied.length} blocked action${live.denied.length === 1 ? '' : 's'}`, why: 'Policies refused what it tried — worth a look.', tab: 'audit' });
  if (envOf(agent) === 'prod' && !live.placement) out.push({ level: 'medium', title: 'Prod, but not deployed', why: 'It runs on the shared runtime, beside other agents.', tab: 'releases' });
  if (!envOf(agent)) out.push({ level: 'info', title: 'No environment set', why: 'Its owner has not said whether it is dev, test or prod.', tab: 'settings' });
  if (agent.mode === 'persistent') out.push({ level: 'info', title: 'Remembers across conversations', why: 'What callers tell it is kept and can come back later.', tab: 'access' });
  if (llmLabel && /openai|anthropic|groq/i.test(llmLabel)) out.push({ level: 'info', title: 'Data goes to an external model', why: `${f.llm} — ${llmLabel.replace(/^.*\(|\)$/g, '')} sees what it is sent.`, tab: 'access' });
  if (!f.hooks.some((h) => /redact|pii|guard|filter/i.test(h.name + h.flows.join(' ')))) out.push({ level: 'info', title: 'No redaction or guard hook', why: 'Its template has no hook that filters what goes in or out.', tab: 'controls' });
  return out;
}
