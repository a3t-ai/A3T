#!/usr/bin/env node
/**
 * Triggers used for real on a live A3T (vc-dev by default):
 *   - a webhook's POST body mapped into a flow's input (input_mapping, jq);
 *   - a webhook delivered to an A3T agent: its message and conversation mapped;
 *   - a schedule (cron) that fires on its own, once per minute however many
 *     FlowRs run the scheduler (one run per tick);
 *   - a mapping that fails is reported, and nothing runs;
 *   - dev time: a mapping checked on samples against the flow's input schema;
 *   - a webhook takes a token or a body signed with its secret, nothing else;
 *     its secret is never shown back.
 *
 *   A3T_TOKEN_FILE=<file with a console token> node a3t/backend/tests/e2e-triggers.mjs [--url …] [--keep]
 */
import { readFileSync } from 'node:fs';
import { createHmac, randomBytes } from 'node:crypto';
const arg = (k) => { const i = process.argv.indexOf(`--${k}`); return i < 0 ? undefined : process.argv[i + 1]; };
const URL_ = (arg('url') ?? 'https://console.dev.a3t.io').replace(/\/$/, '');
const KEEP = process.argv.includes('--keep');
const T = readFileSync(process.env.A3T_TOKEN_FILE, 'utf8').trim();
let failures = 0;
const check = (label, ok, detail = '') => { console.log(`${ok ? '✓' : '✗'} ${label}${ok || !detail ? '' : `  — ${String(detail).slice(0, 300)}`}`); if (!ok) failures++; };
const H = { 'content-type': 'application/json', authorization: `Bearer ${T}` };
const mcp = async (name, args) => { const j = await (await fetch(`${URL_}/message`, { method: 'POST', headers: H, body: JSON.stringify({ jsonrpc: '2.0', id: 1, method: 'tools/call', params: { name, arguments: args } }) })).json(); const text = j.result?.content?.map((c) => c.text).join('\n') ?? JSON.stringify(j.error ?? j); let data = null; try { data = JSON.parse(text.slice(text.search(/[[{]/))); } catch {} return { err: !!(j.error || j.result?.isError), text, data }; };
const tool = async (agent, name, args) => { const r = await fetch(`${URL_}/agents/${agent}/tools/${name}`, { method: 'POST', headers: H, body: JSON.stringify(args) }); return { status: r.status, body: await r.json().catch(() => null) }; };
// create_trigger answers in markdown: "- ID: `<id>`".
const idOf = (r) => r.data?.id ?? r.data?.trigger?.id ?? (r.text.match(/ID: `([^`]+)`/) ?? [])[1];
const wait = (ms) => new Promise((r) => setTimeout(r, ms));
const run = Date.now().toString(36);
const created = { triggers: [], flows: [], agents: [] };

// A flow that answers with what it was given.
const echo = `flow_default_echo_${run}`;
let r = await mcp('create_flow', { yaml: `document: { dsl: '1.0.3', namespace: default, name: ${echo}, version: '1.0.0' }\ndo:\n  - echo:\n      set:\n        got: '\${ . }'\n` });
check(`flow default/${echo}`, !r.err, r.text); created.flows.push(echo);

const status = async (id) => { for (let i = 0; i < 40; i++) { const s = await mcp('get_execution_status', { instance_id: id }); const d = s.data ?? {}; if (/completed|failed/i.test(d.status ?? '')) return d; await wait(1000); } return null; };
const runsOf = async () => { const l = await mcp('list_executions', { namespace: 'default', name: echo, limit: 50 }); return l.data?.executions ?? l.data?.runs ?? (Array.isArray(l.data) ? l.data : []); };

// 1. Webhook → flow, the body mapped.
const path = `/e2e/${run}/map`;
r = await mcp('create_trigger', { name: `e2e-map-${run}`, namespace: 'default', enabled: true,
  config: { type: 'webhook', path, method: 'POST' },
  targets: [{ type: 'workflow', namespace: 'default', name: echo, input_mapping: '${ {order: .body.order_id, amount: (.body.amount * 100), from: .trigger} }' }] });
const mapId = idOf(r); created.triggers.push(mapId);
check('webhook trigger with an input mapping', !r.err && !!mapId, r.text);
let w = await fetch(`${URL_}/webhook${path}`, { method: 'POST', headers: H, body: JSON.stringify({ order_id: 'A-17', amount: 4.5, noise: 'dropped' }) });
let wj = await w.json().catch(() => ({}));
const inst = wj.executions?.[0]?.instance_id;
check(`  POST fired it (${w.status})`, w.ok && !!inst, JSON.stringify(wj).slice(0, 300));
let d = inst ? await status(inst) : null;
const got = d?.output?.got ?? {};
check(`  the flow got the mapped input: ${JSON.stringify(got)}`, got.order === 'A-17' && got.amount === 450 && got.from === `e2e-map-${run}` && !('noise' in got), JSON.stringify(d).slice(0, 300));

// 1b. Who may fire it: a token, or the body signed with the trigger's secret.
const spath = `/e2e/${run}/signed`;
const secret = randomBytes(24).toString('hex');
r = await mcp('create_trigger', { name: `e2e-signed-${run}`, namespace: 'default', enabled: true,
  config: { type: 'webhook', path: spath, method: 'POST', secret }, targets: [{ type: 'workflow', namespace: 'default', name: echo }] });
const sId = idOf(r); created.triggers.push(sId);
check('webhook trigger with a secret', !r.err && !!sId, r.text);
const sbody = JSON.stringify({ order_id: 'S-1' });
const sign = (k, b) => 'sha256=' + createHmac('sha256', k).update(b).digest('hex');
const plain = { 'content-type': 'application/json' };
w = await fetch(`${URL_}/webhook${spath}`, { method: 'POST', headers: plain, body: sbody });
check(`  unsigned, no token: refused (${w.status})`, w.status === 401);
w = await fetch(`${URL_}/webhook${spath}`, { method: 'POST', headers: { ...plain, 'x-a3t-signature': sign('another-secret-0123456789', sbody) }, body: sbody });
check(`  wrongly signed: refused (${w.status})`, w.status === 401);
w = await fetch(`${URL_}/webhook${spath}`, { method: 'POST', headers: { ...plain, 'x-a3t-signature': sign(secret, sbody) }, body: sbody });
check(`  signed with its secret: fired (${w.status})`, w.ok, await w.text());
w = await fetch(`${URL_}/webhook${spath}`, { method: 'POST', headers: { ...plain, 'x-hub-signature-256': sign(secret, sbody) }, body: sbody });
check(`  GitHub's signature header: fired (${w.status})`, w.ok);
w = await fetch(`${URL_}/webhook${path}`, { method: 'POST', headers: plain, body: sbody });
check(`  a trigger with no secret takes signed-in callers only (${w.status})`, w.status === 401);
const listed = await mcp('get_trigger', { id: sId });
check('  its secret is never shown back', !listed.text.includes(secret) && listed.data?.config?.secret_set === true, listed.text.slice(0, 300));
r = await mcp('create_trigger', { name: `e2e-short-${run}`, namespace: 'default', config: { type: 'webhook', path: `/e2e/${run}/short`, secret: 'short' }, targets: [] });
check('  a short secret is refused', r.err, r.text);
if (!r.err) created.triggers.push(idOf(r));

// 2. A mapping that fails: reported, nothing runs.
const bad = `/e2e/${run}/bad`;
r = await mcp('create_trigger', { name: `e2e-bad-${run}`, namespace: 'default', enabled: true, config: { type: 'webhook', path: bad, method: 'POST' },
  targets: [{ type: 'workflow', namespace: 'default', name: echo, input_mapping: '${ .body.x | error("no") }' }] });
created.triggers.push(idOf(r));
w = await fetch(`${URL_}/webhook${bad}`, { method: 'POST', headers: H, body: JSON.stringify({ x: 1 }) });
wj = await w.json().catch(() => ({}));
check(`  a failing mapping is reported (${w.status}) and runs nothing`, w.status === 422 && !(wj.executions?.length) && /mapping/.test(wj.failed?.[0]?.error ?? ''), JSON.stringify(wj).slice(0, 300));

// 3. Webhook → A3T agent: message and conversation mapped.
const agentId = `agt-trg-${run}`;
r = await tool('agent_a3t_agents', 'tool_a3t_agent_create', { org_id: 'org-default', id: agentId, name: `trg${run}`, project: 'default',
  from: { kind: 'template', id: 'tmpl_a3t_agent' }, agent_mode: 'persistent',
  values: { system_prompt: 'You are a test agent. Reply with the order number you are given and nothing else.', llm_resource: 'a3t/claude-cli-max', mcp_servers: [], target: 'a3t/tmpl_a3t_agent' },
  resource_bindings: { llm: 'a3t/claude-cli-max' } });
check(`agent ${agentId}`, r.status === 200, JSON.stringify(r.body).slice(0, 200)); created.agents.push(agentId);
const apath = `/e2e/${run}/agent`;
r = await mcp('create_trigger', { name: `e2e-agent-${run}`, namespace: 'default', enabled: true, config: { type: 'webhook', path: apath, method: 'POST' },
  targets: [{ type: 'a3t_agent', agent_id: agentId, message: '${ "Order number: " + .body.order_id }', conversation: '${ "cust-" + .body.customer }' }] });
created.triggers.push(idOf(r));
w = await fetch(`${URL_}/webhook${apath}`, { method: 'POST', headers: H, body: JSON.stringify({ order_id: 'B-42', customer: 'c9' }) });
wj = await w.json().catch(() => ({}));
const ainst = wj.executions?.[0]?.instance_id;
check(`  POST reached the agent (${w.status})`, w.ok && !!ainst, JSON.stringify(wj).slice(0, 300));
d = ainst ? await status(ainst) : null;
check(`  it answered from the mapped message: "${String(d?.output?.content ?? '').slice(0, 60)}"`, /B-?42/.test(String(d?.output?.content ?? '')), JSON.stringify(d).slice(0, 300));
const turns = await tool('agent_a3t_agents', 'tool_a3t_agent_turns', { id: agentId });
check('  in the mapped conversation (cust-c9)', (turns.body?.output?.turns ?? []).some((t) => t.session_id === `${agentId}:cust-c9`), JSON.stringify(turns.body?.output?.turns?.map((t) => t.session_id)));

// 3b. Dev time: a mapping checked on samples, against the flow's input schema.
const sflow = `flow_default_schema_${run}`;
r = await mcp('create_flow', { yaml: `document: { dsl: '1.0.3', namespace: default, name: ${sflow}, version: '1.0.0' }\ninput:\n  schema:\n    type: object\n    required: [order, amount_cents]\n    properties:\n      order: { type: string }\n      amount_cents: { type: integer }\ndo:\n  - echo:\n      set:\n        got: '\${ . }'\n` });
created.flows.push(sflow);
const mcheck = async (expr, samples, extra = {}) => { const x = await fetch(`${URL_}/triggers/mapping/check`, { method: 'POST', headers: H, body: JSON.stringify({ expr, samples, field: 'input', target: `default/${sflow}`, trigger: 't', source: 'webhook', ...extra }) }); return { status: x.status, body: await x.json().catch(() => ({})) }; };
const S = [{ order_id: 'A-1', amount: 4.5 }, { order_id: 'B-2', amount: 12 }];
let c = await mcheck('${ {order: .body.order_id, amount_cents: (.body.amount * 100 | round)} }', S);
check('mapping check: a good mapping passes on every sample, against the schema', c.status === 200 && c.body.all_ok === true && c.body.results?.[0]?.value?.amount_cents === 450 && !!c.body.schema, JSON.stringify(c.body).slice(0, 300));
c = await mcheck('${ {order: .body.order_id} }', S);
check('  one missing a required field fails, naming it', c.body.all_ok === false && /amount_cents/.test(JSON.stringify(c.body.results?.[0]?.schema_errors)), JSON.stringify(c.body).slice(0, 300));
c = await mcheck('${ .body.amount | error("bad") }', S);
check('  one that errors reports the error', c.body.all_ok === false && /bad/.test(c.body.results?.[0]?.error ?? ''), JSON.stringify(c.body).slice(0, 300));
c = await mcheck('${ "Order " + .body.order_id }', S, { field: 'message', target: undefined });
check('  a message mapping gives text', c.body.all_ok === true && c.body.results?.[0]?.value === 'Order A-1', JSON.stringify(c.body).slice(0, 300));
c = await mcheck('${ . }', []);
check('  no samples: 400', c.status === 400, JSON.stringify(c));

// 4. A schedule: fires on its own, one run per minute.
const before = (await runsOf()).length;
r = await mcp('create_trigger', { name: `e2e-cron-${run}`, namespace: 'default', enabled: true, config: { type: 'cron', expression: '* * * * *', message: 'tick' },
  targets: [{ type: 'workflow', namespace: 'default', name: echo, input_mapping: '${ {tick: .body.minute} }' }] });
const cronId = idOf(r); created.triggers.push(cronId);
check('cron trigger every minute', !r.err && !!cronId, r.text);
console.log('  waiting for two ticks (up to 130 s)…');
await wait(130000);
await mcp('pause_trigger', { id: cronId });
const ticks = (await runsOf()).length - before;
check(`  it fired on its own, one run per tick: ${ticks} runs in two ticks`, ticks >= 1 && ticks <= 2, `${ticks} runs`);

if (!KEEP) {
  for (const id of created.triggers.filter(Boolean)) await mcp('delete_trigger', { id });
  for (const n of created.flows) await mcp('delete_flow', { namespace: 'default', name: n });
  for (const a of created.agents) { await tool('agent_a3t_agents', 'tool_a3t_agent_delete', { org_id: 'org-default', id: a }); await tool('agent_a3t_agents', 'tool_a3t_agent_forget', { org_id: 'org-default', id: a }); }
} else console.log('  kept', JSON.stringify(created));
console.log(failures ? `\n${failures} failed` : '\nall passed');
process.exit(failures ? 1 : 0);
