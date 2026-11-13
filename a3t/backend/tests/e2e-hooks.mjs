#!/usr/bin/env node
/**
 * The agent's own hooks (AH-1), used for real on a live A3T (vc-dev by default):
 * hook flows in project `default`, an agent that names them in values.hooks, and
 * what each phase does to its turns.
 *
 *   A3T_TOKEN_FILE=<file with a console token> node a3t/backend/tests/e2e-hooks.mjs [--url …] [--keep]
 *
 * Checks, by phase:
 *   on_turn_start  replaces the message (the model gets the hook's)       → reply
 *                  refuses a message ("forbidden")                       → refused
 *   before_llm     add_to_prompt ("end with ZEBRA")                      → reply
 *   after_llm, on_turn_end   observe: each writes a row to zz_hook_log   → rows
 *   on_tool_call   refuses list_flows                                   → log row, reply
 *   before_store   skip: nothing of the turn is remembered              → memory
 */
import { readFileSync } from 'node:fs';
const arg = (k) => { const i = process.argv.indexOf(`--${k}`); return i < 0 ? undefined : process.argv[i + 1]; };
const URL_ = (arg('url') ?? 'https://console.dev.a3t.io').replace(/\/$/, '');
const KEEP = process.argv.includes('--keep');
const T = readFileSync(process.env.A3T_TOKEN_FILE, 'utf8').trim();
let failures = 0;
const check = (label, ok, detail = '') => { console.log(`${ok ? '✓' : '✗'} ${label}${ok || !detail ? '' : `  — ${String(detail).slice(0, 300)}`}`); if (!ok) failures++; };
const rest = async (method, path, body) => { const r = await fetch(`${URL_}${path}`, { method, headers: { 'content-type': 'application/json', authorization: `Bearer ${T}` }, body: body ? JSON.stringify(body) : undefined }); let j = null; try { j = await r.json(); } catch {} return { status: r.status, body: j }; };
const mcp = async (name, args) => { const j = await (await fetch(`${URL_}/message`, { method: 'POST', headers: { 'content-type': 'application/json', authorization: `Bearer ${T}` }, body: JSON.stringify({ jsonrpc: '2.0', id: 1, method: 'tools/call', params: { name, arguments: args } }) })).json(); return { err: !!(j.error || j.result?.isError), text: j.result?.content?.map((c) => c.text).join('\n') ?? JSON.stringify(j.error ?? j) }; };
const tool = (agent, name, args) => rest('POST', `/agents/${agent}/tools/${name}`, args);
const run = Date.now().toString(36);
const id = `agt-hooks-${run}`;

// Hook flows. One log flow, shared by the observing phases: a row per call.
const flow = (name, body) => `document:\n  dsl: '1.0.3'\n  namespace: default\n  name: ${name}\n  version: '1.0.0'\ndo:\n${body}`;
const logRow = `  - log:\n      call: lancedb\n      with:\n        operation: insert\n        table: zz_hook_log\n        records:\n          - id: '\${ .phase + "-" + ((now * 1000000) | floor | tostring) }'\n            agent_id: '\${ .agent_id }'\n            phase: '\${ .phase }'\n            what: '\${ (.tool // .outcome // .iteration // "") | tostring }'\n        _resource: a3t/a3t-platform-db\n`;
const FLOWS = {
  flow_default_hook_start: flow('flow_default_hook_start',
    `  - decide:\n      switch:\n        - no: { when: '\${ (.message // "") | test("forbidden") }', then: refuse }\n        - rewrite: { when: '\${ (.message // "") | test("^REWRITE") }', then: rewrite }\n        - default: { then: keep }\n  - keep:\n      set: { message: '\${ .message }' }\n      then: end\n  - rewrite:\n      set: { message: 'Reply with exactly the word HOOKED and nothing else.' }\n      then: end\n  - refuse:\n      set: { refuse: 'forbidden words' }\n      then: end\n`),
  flow_default_hook_prompt: flow('flow_default_hook_prompt', `  - add:\n      set: { add_to_prompt: 'Always end your answer with the word ZEBRA.' }\n`),
  flow_default_hook_log: flow('flow_default_hook_log', logRow),
  flow_default_hook_tool: flow('flow_default_hook_tool', logRow + `  - deny:\n      set: { refuse: '\${ "the tool " + .tool + " is not allowed for this agent" }' }\n`),
  flow_default_hook_nostore: flow('flow_default_hook_nostore', `  - skip:\n      set: { skip: true }\n`),
};
for (const [name, y] of Object.entries(FLOWS)) {
  const c = await mcp('create_flow', { yaml: y });
  check(`hook flow default/${name}`, !c.err || /exist/i.test(c.text), c.text);
}
await mcp('execute_flow', { namespace: 'default', name: 'flow_default_hook_log', input: { agent_id: 'setup', phase: 'setup' } });

// The agent, with its hooks.
const hooks = {
  on_turn_start: ['default/flow_default_hook_start'],
  before_llm: ['default/flow_default_hook_prompt', 'default/flow_default_hook_log'],
  after_llm: ['default/flow_default_hook_log'],
  on_tool_call: ['default/flow_default_hook_tool'],
  on_turn_end: ['default/flow_default_hook_log'],
};
let t = await tool('agent_a3t_agents', 'tool_a3t_agent_create', { org_id: 'org-default', id, name: id, project: 'default',
  from: { kind: 'template', id: 'tmpl_a3t_agent' }, agent_mode: 'persistent',
  values: { system_prompt: 'You are a test agent. Answer in one short sentence.', llm_resource: 'a3t/claude-cli-max', mcp_servers: ['a3t/agent-flows'], target: 'a3t/tmpl_a3t_agent', hooks },
  resource_bindings: { llm: 'a3t/claude-cli-max' } });
check(`agent ${id} with hooks at ${Object.keys(hooks).join(', ')}`, t.status === 200 && !!t.body?.output?.key?.key, JSON.stringify(t.body).slice(0, 300));
const KEY = t.body?.output?.key?.key;
const chat = async (message, session) => {
  const r = await fetch(`${URL_}/chat`, { method: 'POST', headers: { 'content-type': 'application/json', authorization: `Bearer ${KEY}`, accept: 'text/event-stream' }, body: JSON.stringify({ session_id: `${id}:${session}`, message }) });
  const text = await r.text();
  return { status: r.status, text, reply: [...text.matchAll(/"delta":"((?:[^"\\]|\\.)*)"/g)].map((m) => JSON.parse(`"${m[1]}"`)).join('') };
};
const logs = async () => { const q = await mcp('execute_flow', { namespace: 'default', name: 'flow_default_hook_q', input: { id } }); try { return JSON.parse(q.text.slice(q.text.indexOf('Output:') + 8)); } catch { return []; } };
await mcp('create_flow', { yaml: flow('flow_default_hook_q', `  - q:\n      call: lancedb\n      with: { operation: query, table: zz_hook_log, filter: '\${ "agent_id = ''" + .id + "''" }', limit: 500, _resource: a3t/a3t-platform-db }\n`) });

let c = await chat('Say hello.', 'a');
check('before_llm: add_to_prompt reached the model (ends with ZEBRA)', c.status === 200 && /zebra\W*$/i.test(c.reply.trim()), c.reply || c.text.slice(0, 200));
let rows = await logs();
check(`before_llm, after_llm, on_turn_end each ran (${rows.map((r) => r.phase).join(', ')})`, ['before_llm', 'after_llm', 'on_turn_end'].every((p) => rows.some((r) => r.phase === p)), JSON.stringify(rows).slice(0, 300));
c = await chat('REWRITE this into something else', 'b');
check('on_turn_start: the model got the hook\'s message (HOOKED)', /HOOKED/.test(c.reply), c.reply || c.text.slice(0, 200));
c = await chat('this has forbidden words', 'c');
check('on_turn_start: refuse stops the turn', !/zebra/i.test(c.reply) && /refused by a hook|forbidden/i.test(c.text), c.text.slice(0, 300));
c = await chat('Use the list_flows tool to list the flows in project default, then tell me how many there are.', 'd');
rows = await logs();
check('on_tool_call: the hook saw the tool call', rows.some((r) => r.phase === 'on_tool_call' && /list_flows/.test(r.what)), JSON.stringify(rows.filter((r) => r.phase === 'on_tool_call')).slice(0, 300));
check('on_tool_call: the tool was refused, and the model was told', /not allowed|refused|can't|cannot|unable/i.test(c.reply), c.reply.slice(0, 300));

// before_store: skip → this turn adds nothing to memory.
const mem = async () => { const m = await tool('agent_a3t_agents', 'tool_a3t_agent_memory', { id }); return m.body?.output?.counts?.episodic ?? -1; };
const g = await tool('agent_a3t_agents', 'tool_a3t_agent_get', { org_id: 'org-default', id });
const row = Array.isArray(g.body?.output) ? g.body.output[0] : g.body?.output;
const vars = JSON.parse(row?.variables || '{}');
vars.hooks = { before_store: ['default/flow_default_hook_nostore'] };
await tool('agent_a3t_agents', 'tool_a3t_agent_update', { org_id: 'org-default', id, timestamp: new Date().toISOString(),
  data: { name: row.name, template_id: row.template_id, template_version: row.template_version, mode: row.mode, variables: JSON.stringify(vars), resource_bindings: row.resource_bindings, namespace: row.namespace, status: row.status } });
const before = await mem();
c = await chat('Remember that my favourite colour is teal.', 'e');
const after = await mem();
check(`before_store: skip remembered nothing (episodic ${before} → ${after})`, c.status === 200 && before >= 0 && after === before, `${before} → ${after}`);

if (!KEEP) {
  await tool('agent_a3t_agents', 'tool_a3t_agent_delete', { org_id: 'org-default', id });
  await tool('agent_a3t_agents', 'tool_a3t_agent_forget', { org_id: 'org-default', id });
} else console.log(`  kept ${id}`);
console.log(failures ? `\n${failures} failed` : '\nall passed');
process.exit(failures ? 1 : 0);
