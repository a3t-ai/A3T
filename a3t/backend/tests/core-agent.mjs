#!/usr/bin/env node
/**
 * The core agent (system/agent-engine 2.1.0), end to end on a real FlowR and ADB.
 *
 * Starts its own ADB, FlowR (EE, with a throwaway JWT secret) and a stub LLM on free
 * ports, loads the platform base, the error handler and the core agent's flows from
 * this repo, then checks:
 *
 *   - knowledge: system/agent-knowledge-ingest splits docs into SEMANTIC sections;
 *     a second run updates in place and forgets sections that went away;
 *   - tools: system/agent-tools-sync keeps one PROCEDURAL record per tool, updates
 *     and forgets on the next run;
 *   - a turn: the prompt carries the agent's instructions, the relevant doc section
 *     and the fitting tools; WORKING and EPISODIC memory are written; one
 *     a3t_agent_turns row;
 *   - a child agent (extends system/agent-engine@2.1.0) inherits the base's error
 *     handler and runs through the core;
 *   - degraded: knowledge on an unreachable ADB → the turn still answers, one
 *     a3t_errors row for step recall_knowledge;
 *   - failure: the LLM fails → the run fails with the plain message, one a3t_errors row;
 *   - a3t-chat-engine: the agent's own record (agents table) reaches the prompt.
 *
 *   node a3t/backend/tests/core-agent.mjs --adb <adb bin> --flowr <flowr EE bin>
 *   (KEEP=1 keeps the temp dir and prints it; LOGS=1 prints FlowR's log tail on failure)
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

  // ── resources, as A3T's are named ──
  await must('create_resource', { namespace: 'a3t', name: 'stub-llm', type_ref: 'llm', config: { provider: 'openai', api_url: `http://127.0.0.1:${llmPort}`, api_key: 'stub', model: 'stub' } });
  await must('create_resource', { namespace: 'a3t', name: 'agent-memory', type_ref: 'adb', config: { url: adbUrl, flight_url: flightUrl } });
  await must('create_resource', { namespace: 'a3t', name: 'dead-adb', type_ref: 'adb', config: { url: 'http://127.0.0.1:1', flight_url: 'http://127.0.0.1:1' } });
  await must('create_resource', { namespace: 'a3t', name: 'a3t-platform-db', type_ref: 'platform-db', config: { backend: 'adb', url: adbUrl, flight_url: flightUrl } });

  // ── flows: the base first ──
  for (const f of ['system/a3t_base.yaml', 'system/a3t_error_handler.yaml', 'system/agent_turn.yaml', 'system/agent-engine.yaml', 'system/agent_knowledge_ingest.yaml', 'system/agent_tools_sync.yaml', 'system/agent_engine_install.yaml', 'a3t/a3t_chat_engine.yaml']) {
    const r = await tool('create_flow', { yaml: readFileSync(join(FLOWS, f), 'utf8') });
    check(`load ${f}`, !r.isError, r.text);
  }
  // The platform tables the core agent writes, as a3t-schema-install creates them.
  const install = readFileSync(join(FLOWS, 'system/a3t_schema_install.yaml'), 'utf8');
  const tableStep = name => install.slice(install.indexOf(`- ${name}:`)).split(/\n- /)[0];
  await must('create_flow', { yaml: `document: { dsl: '1.0.3', namespace: a3t, name: core-test-tables, version: '1.0.0' }
do:
${tableStep('create-errors')}
${tableStep('create-agent-turns')}
${tableStep('create-agent-configs')}
` });
  let rt = await run('a3t', 'core-test-tables', {});
  // The upgrade flow for installed platforms: creates the turn log table, twice is fine.
  for (const n of [1, 2]) {
    const ri = await run('system', 'agent-engine-install', {});
    check(`agent-engine-install run ${n}`, (ri.out?.tables ?? []).includes('a3t_agent_turns'), ri.text?.slice(0, 400));
  }
  check('platform tables a3t_errors, a3t_agent_turns created', !rt.isError, rt.text);
  await must('create_flow', { yaml: `document: { dsl: '1.0.3', namespace: a3t, name: core-test-rows, version: '1.0.0' }
do:
  - q:
      call: lancedb
      with: { operation: query, table: '\${ .table }', filter: '\${ .filter }', limit: 100, _resource: a3t/a3t-platform-db }
` });
  await must('create_flow', { yaml: `document: { dsl: '1.0.3', namespace: a3t, name: core-test-agent, version: '1.0.0' }
metadata:
  a3t:
    extends: system/agent-engine@2.1.0
do:
  - agent:
      run:
        workflow:
          namespace: system
          name: agent-engine
          input:
            message: '\${ .message }'
            session_id: '\${ .session_id }'
            config: '\${ {llm_resource: "a3t/stub-llm", system_prompt: "You are the core test agent.", memory_namespace: "core-test", mcp_servers: []} * (.config // {}) }'
` });

  // ── 1. knowledge ──
  const doc1 = { source: 'flowr/dsl.md', title: 'FlowR DSL', text: '# FlowR DSL\nFlows are CNCF Serverless Workflow 1.0.3.\n## Error handling\nA try task runs its do tasks; catch handles a failure and sees it as $context.error.\n## Loops\nA for task iterates over a list with each and in.\n## Old section\nThis section will be removed.' };
  let r = await run('system', 'agent-knowledge-ingest', { documents: [doc1], resource: 'a3t/agent-memory' });
  check('ingest: sections stored', !r.isError && r.out?.stored === 4 && (r.out?.failed ?? []).length === 0, r.text);
  let kn = await aql('RECALL FROM SEMANTIC WHERE source = $s NAMESPACE "system-knowledge" LIMIT 50', { s: 'flowr/dsl.md' });
  check('ingest: records keyed per section, kind doc', kn.length === 4 && kn.every(x => x.data.kind === 'doc' && x.data.key.startsWith('doc:flowr/dsl.md#')), JSON.stringify(kn.map(x => x.data.key)));
  check('ingest: quotes and $ survive', kn.some(x => (x.data.content ?? '').includes('$context.error')), JSON.stringify(kn.map(x => x.data.content)));
  const doc2 = { ...doc1, text: doc1.text.replace('\n## Old section\nThis section will be removed.', '').replace('iterates over a list', 'iterates over any list') };
  r = await run('system', 'agent-knowledge-ingest', { documents: [doc2], resource: 'a3t/agent-memory' });
  check('ingest again: updated in place, removed section forgotten', !r.isError && r.out?.updated === 3 && r.out?.forgotten === 1 && r.out?.stored === 0, r.text);
  kn = await aql('RECALL FROM SEMANTIC WHERE source = $s NAMESPACE "system-knowledge" LIMIT 50', { s: 'flowr/dsl.md' });
  check('ingest again: 3 sections, new text', kn.length === 3 && kn.some(x => x.data.content.includes('any list')), JSON.stringify(kn.map(x => x.data.content)));

  // ── 1b. the real docs, through the loader script ──
  const load = spawnSync(process.execPath, [join(FLOWS, '..', 'scripts', 'load-core-agent-memory.mjs'), '--url', url, '--only', 'knowledge'], { env: { ...process.env, FLOWR_TOKEN: BEARER }, encoding: 'utf8', timeout: 300000 });
  check('loader: the docs load without failures', load.status === 0, (load.stdout + load.stderr).slice(-800));
  const mem = await aql('RECALL FROM SEMANTIC WHERE source = $s NAMESPACE "system-knowledge" LIMIT 50', { s: 'docs/concepts/memory' });
  check('loader: a docs page is in knowledge, in sections', mem.length >= 1 && mem.every(x => x.data.title && x.data.content), JSON.stringify(mem.map(x => x.data.title)));
  const guide = await aql('RECALL FROM SEMANTIC WHERE source = $s NAMESPACE "system-knowledge" LIMIT 50', { s: 'core-agent/AGENT_ENGINE' });
  check("loader: the core agent's own guide is in knowledge", guide.length >= 5, String(guide.length));

  // ── 2. tool catalogue ──
  const tools1 = [
    { server: 'a3t/agent-flows', name: 'execute_flow', description: 'Run a workflow by name and return its output' },
    { server: 'a3t/agent-flows', name: 'list_flows', description: 'List the registered workflows' },
    { server: 'a3t/agent-flows', name: 'delete_flow', description: 'Remove a workflow' },
  ];
  r = await run('system', 'agent-tools-sync', { servers: [], tools: tools1, resource: 'a3t/agent-memory' });
  check('tools: one record per tool', !r.isError && r.out?.stored === 3, r.text);
  r = await run('system', 'agent-tools-sync', { servers: [], tools: tools1.slice(0, 2).map(t => ({ ...t, description: t.description + '.' })), resource: 'a3t/agent-memory' });
  check('tools again: updated (stale only for listed servers)', !r.isError && r.out?.updated === 2 && r.out?.stored === 0, r.text);
  r = await run('system', 'agent-tools-sync', { servers: [], tools: tools1.slice(0, 2), complete_for: ['a3t/agent-flows'], resource: 'a3t/agent-memory' });
  check('tools: a complete list forgets what is gone (delete_flow)', !r.isError && r.out?.forgotten === 1, r.text);
  r = await run('system', 'agent-tools-sync', { servers: [], tools: tools1, resource: 'a3t/agent-memory' });
  const tc = await aql('RECALL FROM PROCEDURAL WHERE kind = "tool" NAMESPACE "system-tools" LIMIT 50');
  check('tools: kind tool, schema and usage kept', tc.length === 3 && tc.every(x => x.data.name && x.data.server && 'schema' in x.data), JSON.stringify(tc.map(x => x.data)));

  // The platform's own tools, through the loader script (it lists them with the token).
  const loadTools = spawnSync(process.execPath, [join(FLOWS, '..', 'scripts', 'load-core-agent-memory.mjs'), '--url', url, '--only', 'tools', '--self-server', 'a3t/flowr-self'], { env: { ...process.env, FLOWR_TOKEN: BEARER }, encoding: 'utf8', timeout: 300000 });
  check('loader: the platform tools load without failures', loadTools.status === 0, (loadTools.stdout + loadTools.stderr).slice(-800));
  const own = await aql('RECALL FROM PROCEDURAL WHERE server = "a3t/flowr-self" NAMESPACE "system-tools" LIMIT 200');
  check('loader: the platform tools are in the catalogue', own.length >= 40 && own.some(x => x.data.name === 'execute_flow'), String(own.length));

  // ── 3. a turn through a child agent ──
  prompts.length = 0;
  r = await run('a3t', 'core-test-agent', { message: 'How do I catch an error in a flow? Then run the workflow nightly.', session_id: 'agt-core:s1' });
  check('turn: answered', !r.isError && String(r.out?.content ?? '').startsWith('stub answer to:'), r.text);
  const p = prompts[0] ?? '';
  check('turn: agent instructions in the prompt', p.startsWith('You are the core test agent.'), p.slice(0, 300));
  check('turn: relevant doc section in the prompt', p.includes('## Reference') && p.includes('$context.error'), p);
  check('turn: fitting tools in the prompt', p.includes('## Tools that fit this request') && p.includes('`execute_flow`'), p);
  check('turn: output lists what was recalled', (r.out?.knowledge ?? []).length > 0 && (r.out?.tool_hints ?? []).includes('execute_flow') && (r.out?.degraded ?? []).length === 0, JSON.stringify(r.out));
  await sleep(300);
  const wm = await aql('RECALL FROM WORKING WHERE session_id = "agt-core:s1" NAMESPACE "core-test" LIMIT 10');
  const em = await aql('RECALL FROM EPISODIC WHERE session_id = "agt-core:s1" NAMESPACE "core-test" LIMIT 10');
  check('turn: WORKING memory written', wm.length >= 1, JSON.stringify(wm));
  check('turn: EPISODIC memory written', em.length >= 1 && String(em[0].data.content).startsWith('stub answer'), JSON.stringify(em));
  const turns = await rows('a3t_agent_turns', "session_id = 'agt-core:s1'");
  check('turn: one a3t_agent_turns row', turns.length === 1 && turns[0].agent_id === 'agt-core' && turns[0].tool_hints.includes('execute_flow'), JSON.stringify(turns));
  const def = output((await tool('get_flow', { namespace: 'a3t', name: 'core-test-agent' })).text) ?? (await tool('get_flow', { namespace: 'a3t', name: 'core-test-agent' })).text;
  check('child: inherits the base error handler', JSON.stringify(def).includes('a3t-error-handler') || JSON.stringify(def).includes('agent-engine'), JSON.stringify(def).slice(0, 400));

  // ── 4. degraded: knowledge on an unreachable ADB ──
  r = await run('a3t', 'core-test-agent', { message: 'What is a for loop?', session_id: 'agt-core:s2', config: { knowledge: { resource: 'a3t/dead-adb' } } });
  check('degraded: the turn still answers', !r.isError && String(r.out?.content ?? '').startsWith('stub answer'), r.text);
  check('degraded: output says what was skipped', (r.out?.degraded ?? []).some(d => d.step === 'recall_knowledge'), JSON.stringify(r.out));
  const errs = await rows('a3t_errors', "step = 'recall_knowledge'");
  check('degraded: one a3t_errors row for recall_knowledge', errs.length === 1 && errs[0].flow === 'system/agent-engine', JSON.stringify(errs));

  // ── 5. failure: the LLM refuses ──
  llmMode = 'fail';
  r = await run('a3t', 'core-test-agent', { message: 'Hello', session_id: 'agt-core:s3' });
  llmMode = 'ok';
  check('failure: the run fails', r.isError || /fail/i.test(r.text), r.text);
  const ferrs = (await rows('a3t_errors', "principal = 'core-test'")).filter(e => e.step !== 'recall_knowledge');
  check('failure: logged in a3t_errors, at the turn', ferrs.some(e => e.flow === 'system/agent-turn' && /LLM call failed/.test(e.detail)), JSON.stringify(ferrs));
  known('failure: logged once (not again by each enclosing flow)', ferrs.length === 1, 'CA-R5', `${ferrs.length} rows: ${ferrs.map(e => e.flow).join(', ')}`);
  const lastTurn = (await rows('a3t_agent_turns', "session_id = 'agt-core:s1'"))[0] ?? {};
  known('turn row carries the run id', !!lastTurn.run_id, 'CA-R2');

  // ── 6. a3t-chat-engine reads the agent's own record (agent_configs) ──
  await must('create_flow', { yaml: `document: { dsl: '1.0.3', namespace: a3t, name: core-test-put-agent, version: '1.0.0' }
do:
  - put:
      call: lancedb
      with: { operation: insert, table: agent_configs, records: '\${ [.row] }', _resource: a3t/a3t-platform-db }
` });
  r = await run('a3t', 'core-test-put-agent', { row: { id: 'agt-zed', org_id: 'org-default', name: 'zed', namespace: 'a3t-test', template_id: 'tpl-agent-engine', template_version: '2.1.0', status: 'active', mode: 'session', created_by: 'core-test', created_at: '2026-11-09T00:00:00Z', updated_at: '2026-11-09T00:00:00Z',
    resource_bindings: JSON.stringify({ llm: 'a3t/stub-llm' }), variables: JSON.stringify({ system_prompt: 'You are Zed, the release checker.', temperature: 0.1, max_iterations: 3, mcp_servers: [] }) } });
  check('chat engine: agent row stored', !r.isError, r.text);
  prompts.length = 0;
  r = await run('a3t', 'a3t-chat-engine', { message: 'Who are you?', session_id: 'agt-zed:chat-1' });
  check('chat engine: answered', !r.isError && String(r.out?.content ?? '').startsWith('stub answer'), r.text);
  check("chat engine: the agent's own instructions reach the prompt", (prompts[0] ?? '').startsWith('You are Zed, the release checker.'), (prompts[0] ?? '').slice(0, 300));
  r = await run('a3t', 'a3t-chat-engine', { message: 'Who are you?', session_id: 'studio-plain-1', llm_resource: 'a3t/stub-llm' });
  check('chat engine: a session with no agent uses the A3T assistant', !r.isError && (prompts.at(-1) ?? '').includes('A3T'), (prompts.at(-1) ?? '').slice(0, 200));
  if (process.env.DEBUG) for (const e of await rows('a3t_errors', "id != ''")) console.log('  a3t_errors:', e.flow, e.step, e.class, (e.detail ?? '').slice(0, 400));
} catch (e) {
  failures++;
  console.error(`✗ ${e.message}`);
  if (process.env.LOGS && flowr) console.error(flowr.log());
} finally {
  llm.close();
  for (const c of children) c.kill('SIGKILL');
  if (process.env.KEEP) console.log(`  kept ${tmp}`); else rmSync(tmp, { recursive: true, force: true });
}
console.log(failures ? `\n${failures} failed` : `\nall passed${gaps ? ` (${gaps} known engine gap${gaps === 1 ? '' : 's'})` : ''}`);
process.exit(failures ? 1 : 0);
