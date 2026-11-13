#!/usr/bin/env node
/**
 * The A3T agents end to end (plan "A3T control plane as A3T agents"): on a real
 * FlowR EE (with TF-1/AT-1) and ADB, loads the platform flows, installs the A3T
 * agents with system/flow_system_agents_install (twice), then checks:
 *   - 9 agents, their relation to agent_system_core, A3T agents shared for calls;
 *   - AT-1: GET /agents/{id}/tools and POST /agents/{id}/tools/{tool}, by the
 *     owner and, through the public call share, by an ordinary user; a tool the
 *     agent doesn't have is 404;
 *   - TF-1: the fixed action wins over the caller's;
 *   - a user agent made by tool_a3t_agent_create answers on the data plane (/chat).
 *
 *   node a3t/backend/tests/a3t-agents.mjs --adb <adb bin> --flowr <flowr EE bin>
 */
import { spawn, spawnSync } from 'node:child_process';
import { createHmac, randomBytes } from 'node:crypto';
import { mkdtempSync, readFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { createServer as netServer } from 'node:net';
import { createServer as httpServer } from 'node:http';

const args = Object.fromEntries(process.argv.slice(2).reduce((a, v, i, all) => (v.startsWith('--') ? [...a, [v.slice(2), all[i + 1]]] : a), []));
if (!args.adb || !args.flowr) {
  console.error('usage: core-agent.mjs --adb <adb bin> --flowr <flowr EE bin>');
  process.exit(2);
}
const FLOWS = join(dirname(fileURLToPath(import.meta.url)), '..', 'flows');
const sleep = ms => new Promise(r => setTimeout(r, ms));
const freePort = () => new Promise(res => { const s = netServer().listen(0, '127.0.0.1', () => { const p = s.address().port; s.close(() => res(p)); }); });
let failures = 0;
const check = (label, ok, detail = '') => { console.log(`${ok ? '✓' : '✗'} ${label}${ok || !detail ? '' : `  — ${String(detail).slice(0, 600)}`}`); if (!ok) failures++; };
// What the engine doesn't do yet: shown, not counted (.ai/CORE_AGENT_ENGINE_REQUESTS.md).
let gaps = 0;
const known = (label, ok, request, detail = '') => { console.log(`${ok ? '✓' : '!'} ${label}${ok ? '' : `  — known engine gap, ${request}${detail ? `: ${String(detail).slice(0, 300)}` : ''}`}`); if (!ok) gaps++; };

const tmp = mkdtempSync(join(tmpdir(), 'core-agent-'));
const children = new Set();
function start(bin, argv, env) {
  const c = spawn(bin, argv, { env: { ...process.env, RUST_LOG: 'warn', ...env }, stdio: ['ignore', 'ignore', 'pipe'] });
  let log = '';
  c.stderr.on('data', d => { log = (log + d).slice(-20000); });
  c.log = () => log;
  c.on('exit', () => children.delete(c));
  children.add(c);
  return c;
}
async function healthy(url, who, c) {
  for (let i = 0; i < 400; i++) {
    try { if ((await fetch(`${url}/health`)).ok) return; } catch {}
    await sleep(100);
  }
  throw new Error(`${who} not healthy: ${c.log()}`);
}

// ── stub LLM: OpenAI-compatible; remembers every system prompt it was sent ──
const prompts = [];
let llmMode = 'ok';
const llm = httpServer((req, res) => {
  let body = '';
  req.on('data', d => { body += d; });
  req.on('end', () => {
    const b = JSON.parse(body || '{}');
    const system = (b.messages ?? []).find(m => m.role === 'system')?.content ?? '';
    prompts.push(system);
    if (llmMode === 'fail') { res.writeHead(400, { 'content-type': 'application/json' }); res.end(JSON.stringify({ error: { message: 'stub: model refused the request' } })); return; }
    const user = [...(b.messages ?? [])].reverse().find(m => m.role === 'user')?.content ?? '';
    const content = `stub answer to: ${user}`;
    if (b.stream) {
      res.writeHead(200, { 'content-type': 'text/event-stream' });
      res.write(`data: ${JSON.stringify({ choices: [{ delta: { content } }] })}\n\n`);
      res.end('data: [DONE]\n\n');
    } else {
      res.writeHead(200, { 'content-type': 'application/json' });
      res.end(JSON.stringify({ choices: [{ message: { role: 'assistant', content } }] }));
    }
  });
});

let flowr;
try {
  const [adbPort, flightPort, flowrPort, llmPort] = [await freePort(), await freePort(), await freePort(), await freePort()];
  const adbUrl = `http://127.0.0.1:${adbPort}`, flightUrl = `http://127.0.0.1:${flightPort}`;
  const adb = start(args.adb, ['--data-dir', join(tmp, 'adb'), 'serve-http', '--host', '127.0.0.1', '--port', String(adbPort), '--flight-port', String(flightPort)], {});
  await healthy(adbUrl, 'ADB', adb);
  await new Promise(r => llm.listen(llmPort, '127.0.0.1', r));

  const url = `http://127.0.0.1:${flowrPort}`;
  const secret = randomBytes(32).toString('hex');
  flowr = start(args.flowr, ['--mode', 'http', '--port', String(flowrPort)], {
    HOSTNAME: 'flowr-console', CLAWDFLOW_DATA_DIR: join(tmp, 'flowr'),
    FLOWR_JWT_SECRET: secret, MCP_RESOURCE_URI: url,
    RUST_LOG: process.env.RUST_LOG ?? 'warn',
  });
  await healthy(url, 'FlowR', flowr);
  const b64u = b => Buffer.from(b).toString('base64url');
  const now = Math.floor(Date.now() / 1000);
  const jwt = c => { const h = b64u(JSON.stringify({ alg: 'HS256', typ: 'JWT' })), p = b64u(JSON.stringify(c)); return `${h}.${p}.${createHmac('sha256', secret).update(`${h}.${p}`).digest('base64url')}`; };
  const BEARER = jwt({ sub: 'core-test', org: 'org-default', role: 'superadmin', email: 'core-test@localhost', name: 'core-test', aud: url, iat: now, exp: now + 7200 });
  console.log(`  ADB ${adbUrl}, FlowR ${url}, stub LLM :${llmPort}`);

  let rpc = 1;
  const tool = async (name, a) => {
    const r = await (await fetch(`${url}/message`, { method: 'POST', headers: { 'content-type': 'application/json', authorization: `Bearer ${BEARER}` }, body: JSON.stringify({ jsonrpc: '2.0', id: rpc++, method: 'tools/call', params: { name, arguments: a } }) })).json();
    const text = r.result?.content?.[0]?.text ?? '';
    return { isError: !!(r.error || r.result?.isError), text: text || JSON.stringify(r.error ?? r) };
  };
  const must = async (name, a) => { const r = await tool(name, a); if (r.isError) throw new Error(`${name}: ${r.text}`); return r.text; };
  const output = text => {
    const i = text.indexOf('Output:\n');
    if (i < 0) return undefined;
    let o = JSON.parse(text.slice(i + 8));
    if (typeof o === 'string') { try { o = JSON.parse(o); } catch {} }
    return o;
  };
  const run = async (namespace, name, input) => { const r = await tool('execute_flow', { namespace, name, input }); return { ...r, out: output(r.text) }; };
  const aql = async (query, variables) => {
    const r = await (await fetch(`${adbUrl}/query`, { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify({ query, variables }) })).json();
    if (!r.success) throw new Error(`${query}: ${r.error}`);
    return (r.data?.data ?? r.data)?.records ?? [];
  };
  const rows = async (table, filter) => {
    const r = await run('a3t', 'core-test-rows', { table, filter });
    if (r.isError) throw new Error(`rows ${table}: ${r.text}`);
    return Array.isArray(r.out) ? r.out : (r.out?.rows ?? r.out?.data ?? []);
  };
  await must('create_resource', { namespace: 'a3t', name: 'adb', type_ref: 'adb', config: { url: adbUrl, flight_url: flightUrl } });
  await must('create_resource', { namespace: 'a3t', name: 'a3t-platform-db', type_ref: 'platform-db', config: { backend: 'adb', url: adbUrl, flight_url: flightUrl } });
  await must('create_resource', { namespace: 'a3t', name: 'claude-cli-max', type_ref: 'llm', config: { provider: 'openai', api_url: `http://127.0.0.1:${llmPort}`, api_key: 'stub', model: 'stub' } });
  for (const f of ['system/flow_system_base.yaml', 'system/flow_system_error_handler.yaml', 'system/flow_system_agent_turn.yaml', 'system/tmpl_system_agent_engine.yaml',
                   'system/flow_system_agent_control_install.yaml', 'a3t/flow_a3t_agent_control.yaml', 'a3t/flow_a3t_agents.yaml', 'a3t/flow_a3t_templates.yaml',
                   'a3t/tmpl_a3t_agent.yaml', 'a3t/tmpl_a3t_service.yaml', 'system/flow_system_agents_install.yaml']) {
    const r = await tool('create_flow', { yaml: readFileSync(join(FLOWS, f), 'utf8') });
    check(`load ${f}`, !r.isError, r.text);
  }
  const install = readFileSync(join(FLOWS, 'system/flow_system_schema_install.yaml'), 'utf8');
  const tableStep = name => install.slice(install.indexOf(`- ${name}:`)).split(/\n- /)[0];
  await must('create_flow', { yaml: `document: { dsl: '1.0.3', namespace: a3t, name: ag_test_tables, version: '1.0.0' }
do:
${tableStep('create-errors')}
${tableStep('create-agent-turns')}
${tableStep('create-template-registry')}
${tableStep('create-agent-configs')}
${tableStep('create-agent-events')}
${tableStep('create-agent-relations')}
${tableStep('create-agent-shares')}
` });
  check('platform tables', !(await run('a3t', 'ag_test_tables', {})).isError);
  for (const [id, file] of [['tmpl_system_agent_engine', 'system/tmpl_system_agent_engine.yaml'], ['tmpl_a3t_agent', 'a3t/tmpl_a3t_agent.yaml'], ['tmpl_a3t_service', 'a3t/tmpl_a3t_service.yaml']]) {
    const r = await run('a3t', 'flow_a3t_templates', { input: { action: 'create', org_id: 'org-default', user_id: 'test', timestamp: 'x', data: { id, name: id, version: '1.0.0', source: 'system', spec_yaml: readFileSync(join(FLOWS, file), 'utf8'), tags: '', vertical: 'platform', published: 'true' } } });
    check(`template row ${id}`, !r.isError, r.text);
  }

  // ── install, twice ──
  let r = await run('system', 'flow_system_agents_install', {});
  check('install: 9 agents created', (r.out?.created ?? []).length === 9 && (r.out?.failed ?? []).length === 0, JSON.stringify(r.out ?? r.text).slice(0, 600));
  r = await run('system', 'flow_system_agents_install', {});
  check('install again: 9 updated, none created', (r.out?.updated ?? []).length === 9 && (r.out?.created ?? []).length === 0 && (r.out?.failed ?? []).length === 0, JSON.stringify(r.out ?? r.text).slice(0, 600));
  const ctl = input => run('a3t', 'flow_a3t_agent_control', { org_id: 'org-default', user_id: 'test', ...input });
  r = await ctl({ action: 'lineage', id: 'agent_a3t_agents' });
  check('A3T agent inherits agent_system_core', (r.out?.ancestors ?? [])[0]?.id === 'agent_system_core', JSON.stringify(r.out));
  r = await ctl({ action: 'get', id: 'agent_a3t_agents' });
  check('A3T agent shared for calls', r.out?.visibility === 'public' && r.out.shares[0].can_call === 'true' && r.out.shares[0].can_fork === 'false', JSON.stringify(r.out?.shares));

  // ── AT-1 + TF-1 ──
  const rest = async (method, path, body, bearer = BEARER) => {
    const res = await fetch(`${url}${path}`, { method, headers: { 'content-type': 'application/json', authorization: `Bearer ${bearer}` }, body: body ? JSON.stringify(body) : undefined });
    let j = null; try { j = await res.json(); } catch {}
    return { status: res.status, body: j };
  };
  let t = await rest('GET', '/agents/agent_a3t_agents/tools');
  check('AT-1: the agent lists its tools', t.status === 200 && (t.body?.tools ?? []).some(x => x.name === 'tool_a3t_agent_list'), JSON.stringify(t).slice(0, 300));
  t = await rest('POST', '/agents/agent_a3t_agents/tools/tool_a3t_agent_list', { org_id: 'org-default', action: 'delete' });
  const ids = (Array.isArray(t.body?.output) ? t.body.output : []).map(a => a.id);
  check('AT-1 + TF-1: list runs (the fixed action wins over the caller\'s)', t.status === 200 && ids.includes('agent_a3t_agents'), JSON.stringify(t).slice(0, 400));
  t = await rest('POST', '/agents/agent_a3t_agents/tools/tool_nope', {});
  check('AT-1: a tool the agent lacks is 404', t.status === 404, JSON.stringify(t));
  const userBearer = jwt({ sub: 'plain-user', org: 'org-default', role: 'viewer', email: 'plain@localhost', name: 'plain', aud: url, iat: now, exp: now + 7200 });
  t = await rest('GET', '/agents/agent_a3t_agents/tools', null, userBearer);
  check('AT-1: an ordinary user reaches a shared A3T agent', t.status === 200, JSON.stringify(t).slice(0, 300));
  t = await rest('GET', '/agents/agent_system_core/tools', null, userBearer);
  check('AT-1: but not the system agent', t.status === 404, JSON.stringify(t).slice(0, 300));

  // ── a user agent through the control-plane agent, then its data plane ──
  t = await rest('POST', '/agents/agent_a3t_agents/tools/tool_a3t_agent_create', { org_id: 'org-default', user_id: 'test', id: 'agt-e2e', name: 'e2e-agent', project: 'default',
    from: { kind: 'template', id: 'tmpl_a3t_agent' }, values: { system_prompt: 'You are e2e.', llm_resource: 'a3t/claude-cli-max', mcp_servers: [] }, resource_bindings: { llm: 'a3t/claude-cli-max' } });
  check('control plane: tool_a3t_agent_create makes a user agent', t.status === 200 && t.body?.output?.agent?.id === 'agt-e2e', JSON.stringify(t).slice(0, 400));
  t = await rest('POST', '/agents/agent_a3t_agents/tools/tool_a3t_agent_update', { org_id: 'org-default', id: 'agt-e2e', timestamp: 'x', data: { name: 'e2e-agent', template_id: 'tmpl_a3t_agent', template_version: '1.0.0', mode: 'session', variables: JSON.stringify({ system_prompt: 'You are e2e, changed.', llm_resource: 'a3t/claude-cli-max', mcp_servers: [] }), resource_bindings: '{"llm":"a3t/claude-cli-max"}', namespace: 'default', status: 'draft' } });
  check('control plane: tool_a3t_agent_update changes its config', t.status === 200, JSON.stringify(t).slice(0, 300));
  const chat = await fetch(`${url}/chat`, { method: 'POST', headers: { 'content-type': 'application/json', authorization: `Bearer ${BEARER}` }, body: JSON.stringify({ session_id: 'agt-e2e:s1', message: 'Who are you?', namespace: 'a3t', name: 'tmpl_a3t_agent' }) });
  const stream = await chat.text();
  check('data plane: /chat streams an answer', chat.status === 200 && /stub answer/.test(stream), stream.slice(0, 300));
  check('  with the changed instructions', (prompts.at(-1) ?? '').startsWith('You are e2e, changed.'), (prompts.at(-1) ?? '').slice(0, 200));
  t = await rest('POST', '/agents/agent_a3t_agents/tools/tool_a3t_agent_turns', { id: 'agt-e2e' });
  check('metrics: tool_a3t_agent_turns counts the turn', t.status === 200 && t.body?.output?.summary?.turns >= 1, JSON.stringify(t.body).slice(0, 300));
  t = await rest('POST', '/agents/agent_a3t_agents/tools/tool_a3t_agent_memory', { id: 'agt-e2e' });
  check('memory: tool_a3t_agent_memory reads its working and episodic memory', t.status === 200 && (t.body?.output?.counts?.working ?? 0) >= 1 && (t.body?.output?.counts?.episodic ?? 0) >= 1, JSON.stringify(t.body?.output?.counts ?? t.body).slice(0, 300));
} catch (e) {
  failures++;
  console.error(`✗ ${e.message}`);
  if (process.env.LOGS && flowr) console.error(flowr.log());
} finally {
  llm.close();
  for (const c of children) c.kill('SIGKILL');
  if (process.env.KEEP) console.log(`  kept ${tmp}`); else rmSync(tmp, { recursive: true, force: true });
}
console.log(failures ? `\n${failures} failed` : '\nall passed');
process.exit(failures ? 1 : 0);
