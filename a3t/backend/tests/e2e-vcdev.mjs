#!/usr/bin/env node
/**
 * The plan's end-to-end outcomes against a live A3T (vc-dev by default), through
 * the API only:
 *   1. the A3T agents answer on their control plane (AT-1);
 *   2. a user agent is created by agent_a3t_agents (tool_a3t_agent_create);
 *   3. its data plane: POST /chat streams an answer (SSE, AI SDK UI message stream);
 *   4. its config changes through the control-plane agent (tool_a3t_agent_update);
 *   5. its figures and memory read back (tool_a3t_agent_turns, tool_a3t_agent_memory).
 *
 *   A3T_TOKEN_FILE=<file with a console token> node a3t/backend/tests/e2e-vcdev.mjs [--url https://console.dev.a3t.io] [--llm a3t/claude-cli-max] [--keep]
 * The token is read from the file and never printed. --keep leaves the agent for a look in the UI.
 */
import { readFileSync } from 'node:fs';
const arg = k => { const i = process.argv.indexOf(`--${k}`); return i < 0 ? undefined : process.argv[i + 1]; };
const URL_ = (arg('url') ?? 'https://console.dev.a3t.io').replace(/\/$/, '');
const LLM = arg('llm') ?? 'a3t/claude-cli-max';
const KEEP = process.argv.includes('--keep');
const T = readFileSync(process.env.A3T_TOKEN_FILE, 'utf8').trim();
let failures = 0;
const check = (label, ok, detail = '') => { console.log(`${ok ? '✓' : '✗'} ${label}${ok || !detail ? '' : `  — ${String(detail).slice(0, 400)}`}`); if (!ok) failures++; };
const rest = async (method, path, body) => {
  const r = await fetch(`${URL_}${path}`, { method, headers: { 'content-type': 'application/json', authorization: `Bearer ${T}` }, body: body ? JSON.stringify(body) : undefined });
  let j = null; try { j = await r.json(); } catch {}
  return { status: r.status, body: j };
};
const tool = (agent, name, args) => rest('POST', `/agents/${agent}/tools/${name}`, args);
const id = `agt-e2e-${Date.now().toString(36)}`;

// 1. The A3T agents.
for (const a of ['agent_a3t_agents', 'agent_a3t_templates', 'agent_a3t_projects', 'agent_a3t_resources', 'agent_a3t_runtime', 'agent_a3t_governance', 'agent_a3t_knowledge']) {
  const t = await rest('GET', `/agents/${a}/tools`);
  check(`${a}: ${(t.body?.tools ?? []).length} tools on its control plane`, t.status === 200 && (t.body?.tools ?? []).length > 0, JSON.stringify(t));
}
let t = await tool('agent_a3t_templates', 'tool_a3t_template_list', { org_id: 'org-default' });
check('A3T UI data: templates through agent_a3t_templates', t.status === 200 && Array.isArray(t.body?.output) && t.body.output.some(x => x.id === 'tmpl_a3t_agent'), JSON.stringify(t.body).slice(0, 300));

// 2. Create.
t = await tool('agent_a3t_agents', 'tool_a3t_agent_create', { org_id: 'org-default', id, name: id, project: 'default',
  from: { kind: 'template', id: 'tmpl_a3t_agent' }, agent_mode: 'persistent',
  values: { system_prompt: 'You are the e2e test agent. Answer in one short sentence.', llm_resource: LLM, mcp_servers: [], target: 'a3t/tmpl_a3t_agent' },
  resource_bindings: { llm: LLM } });
check(`create: ${id} made by tool_a3t_agent_create`, t.status === 200 && t.body?.output?.agent?.id === id, JSON.stringify(t.body).slice(0, 300));
console.log(`  control plane: ${URL_}/agents/${id}   data plane: ${URL_}/chat  (session ${id}:<session>)`);

// 3. Data plane, SSE.
const chat = async (message, session) => {
  const r = await fetch(`${URL_}/chat`, { method: 'POST', headers: { 'content-type': 'application/json', authorization: `Bearer ${T}`, accept: 'text/event-stream' }, body: JSON.stringify({ session_id: `${id}:${session}`, message, namespace: 'a3t', name: 'tmpl_a3t_agent' }) });
  const text = await r.text();
  const deltas = [...text.matchAll(/"delta":"((?:[^"\\]|\\.)*)"/g)].map(m => JSON.parse(`"${m[1]}"`)).join('');
  return { status: r.status, type: r.headers.get('content-type'), text, reply: deltas };
};
let c = await chat('Say hello and tell me what you are.', 's1');
check('data plane: /chat streams (SSE) an answer', c.status === 200 && /event-stream/.test(c.type ?? '') && c.reply.length > 0, `${c.status} ${c.type} ${c.text.slice(0, 300)}`);
console.log(`  reply: ${c.reply.slice(0, 160)}`);

// 4. Change it through the control-plane agent.
const g = await tool('agent_a3t_agents', 'tool_a3t_agent_get', { org_id: 'org-default', id });
const row = Array.isArray(g.body?.output) ? g.body.output[0] : g.body?.output;
const vars = JSON.parse(row?.variables || '{}');
vars.system_prompt = 'You are the e2e test agent, changed. Always start your answer with the word CHANGED.';
vars.temperature = 0;
t = await tool('agent_a3t_agents', 'tool_a3t_agent_update', { org_id: 'org-default', id, timestamp: new Date().toISOString(),
  data: { name: row?.name ?? id, template_id: row?.template_id, template_version: row?.template_version, mode: row?.mode, variables: JSON.stringify(vars),
          resource_bindings: typeof row?.resource_bindings === 'string' ? row.resource_bindings : JSON.stringify(row?.resource_bindings ?? {}), namespace: row?.namespace, status: 'live' } });
check('control plane: tool_a3t_agent_update changed its config', t.status === 200, JSON.stringify(t.body).slice(0, 300));
const after = await tool('agent_a3t_agents', 'tool_a3t_agent_describe', { org_id: 'org-default', id });
check('  the record says so', after.body?.output?.agent?.variables?.system_prompt?.includes('CHANGED') && after.body?.output?.agent?.status === 'live', JSON.stringify(after.body?.output?.agent).slice(0, 300));
c = await chat('Who are you?', 's2');
check('  and the agent answers with it', c.status === 200 && /CHANGED/i.test(c.reply), c.reply.slice(0, 200));

// 4b. Its history names the change, and its controls go through the control plane.
const evs = async () => { const e = await tool('agent_a3t_agents', 'tool_a3t_agent_events', { id }); return (Array.isArray(e.body?.output) ? e.body.output : []).map(x => ({ ...x, p: JSON.parse(x.payload || '{}') })); };
let ev = await evs();
const upd = ev.find(x => x.event_type === 'updated');
check(`history: created + updated (${(upd?.p?.changed ?? []).join(', ')})`, ev.some(x => x.event_type === 'created') && upd?.p?.changed?.includes('values.system_prompt') && !!upd?.p?.by, JSON.stringify(ev).slice(0, 300));
t = await tool('agent_a3t_agents', 'tool_a3t_agent_block', { org_id: 'org-default', id, reason: 'e2e' });
check('controls: tool_a3t_agent_block', t.status === 200 && t.body?.output?.status === 'blocked', JSON.stringify(t.body).slice(0, 300));
c = await chat('Are you there?', 's3');
check('  a blocked agent refuses its data plane', !/CHANGED/i.test(c.reply) && /blocked/i.test(c.text), `${c.status} ${c.text.slice(0, 300)}`);
const g2 = await tool('agent_a3t_agents', 'tool_a3t_agent_get', { org_id: 'org-default', id });
const row2 = Array.isArray(g2.body?.output) ? g2.body.output[0] : g2.body?.output;
await tool('agent_a3t_agents', 'tool_a3t_agent_update', { org_id: 'org-default', id, timestamp: new Date().toISOString(),
  data: { name: row2?.name, template_id: row2?.template_id, template_version: row2?.template_version, mode: row2?.mode, variables: row2?.variables,
          resource_bindings: row2?.resource_bindings, namespace: row2?.namespace, status: 'live' } });
const after2 = await tool('agent_a3t_agents', 'tool_a3t_agent_describe', { org_id: 'org-default', id });
check('  a plain update (status live) does not unblock it', after2.body?.output?.agent?.status === 'blocked', JSON.stringify(after2.body?.output?.agent?.status));
t = await tool('agent_a3t_agents', 'tool_a3t_agent_unblock', { org_id: 'org-default', id });
check(`  tool_a3t_agent_unblock gives back "${t.body?.output?.status}"`, t.status === 200 && t.body?.output?.status === 'live', JSON.stringify(t.body).slice(0, 300));
ev = await evs();
check('  both are in its history, with who', ['blocked', 'unblocked'].every(k => ev.some(x => x.event_type === k && x.p.by)), JSON.stringify(ev.map(x => x.event_type)));

// 5. Figures and memory.
t = await tool('agent_a3t_agents', 'tool_a3t_agent_turns', { id });
check(`metrics: ${t.body?.output?.summary?.turns ?? 0} turns, avg ${t.body?.output?.summary?.avg_ms ?? '-'} ms`, t.status === 200 && (t.body?.output?.summary?.turns ?? 0) >= 2, JSON.stringify(t.body).slice(0, 300));
t = await tool('agent_a3t_agents', 'tool_a3t_agent_memory', { id });
check(`memory: ${JSON.stringify(t.body?.output?.counts)}`, t.status === 200 && (t.body?.output?.counts?.working ?? 0) >= 1 && (t.body?.output?.counts?.episodic ?? 0) >= 1, JSON.stringify(t.body).slice(0, 300));

if (!KEEP) {
  await tool('agent_a3t_agents', 'tool_a3t_agent_delete', { org_id: 'org-default', id });
  await tool('agent_a3t_agents', 'tool_a3t_agent_forget', { org_id: 'org-default', id });
} else console.log(`  kept ${id}: open it in the UI (Agents, project default)`);
console.log(failures ? `\n${failures} failed` : '\nall passed');
process.exit(failures ? 1 : 0);
