#!/usr/bin/env node
/**
 * The agents control plane (a3t/a3t-agent-control), end to end on a real FlowR and ADB.
 *
 * Starts its own ADB and FlowR EE (throwaway JWT secret), loads the platform base,
 * the error handler and the control flows, creates the platform tables, then checks:
 *   - create from a template and from an agent (inherit, clone); bad input refused;
 *   - get: record, relation, endpoints, visibility;
 *   - lineage: ancestors (agent, then template) and descendants;
 *   - relate (a fork made by FlowR), compare, merge (procedures into the source,
 *     through ADB) and the refusal for a fork whose config changed, evolve;
 *   - share, shares, shared_with (namespace and public), unshare.
 *
 *   node a3t/backend/tests/agent-control.mjs --adb <adb bin> --flowr <flowr EE bin>
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
  for (const f of ['system/a3t_base.yaml', 'system/a3t_error_handler.yaml', 'system/agent_control_install.yaml', 'a3t/a3t_agent_control.yaml']) {
    const r = await tool('create_flow', { yaml: readFileSync(join(FLOWS, f), 'utf8') });
    check(`load ${f}`, !r.isError, r.text);
  }
  const install = readFileSync(join(FLOWS, 'system/a3t_schema_install.yaml'), 'utf8');
  const tableStep = name => install.slice(install.indexOf(`- ${name}:`)).split(/\n- /)[0];
  await must('create_flow', { yaml: `document: { dsl: '1.0.3', namespace: a3t, name: ctl-test-tables, version: '1.0.0' }
do:
${tableStep('create-errors')}
${tableStep('create-template-registry')}
${tableStep('create-agent-configs')}
${tableStep('create-agent-events')}
` });
  check('platform tables', !(await run('a3t', 'ctl-test-tables', {})).isError);
  for (const n of [1, 2]) {
    const ri = await run('system', 'agent-control-install', {});
    check(`agent-control-install run ${n}`, ['agent_relations', 'agent_shares'].every(t => (ri.out?.tables ?? []).includes(t)), ri.text?.slice(0, 400));
  }
  await must('create_flow', { yaml: `document: { dsl: '1.0.3', namespace: a3t, name: core-test-rows, version: '1.0.0' }
do:
  - q:
      call: lancedb
      with: { operation: query, table: '\${ .table }', filter: '\${ .filter }', limit: 100, _resource: a3t/a3t-platform-db }
` });
  await must('create_flow', { yaml: `document: { dsl: '1.0.3', namespace: a3t, name: ctl-test-seed, version: '1.0.0' }
do:
  - tpl:
      call: lancedb
      with:
        operation: insert
        table: template_registry
        records:
          - { id: tpl-support, org_id: org-default, name: support-agent, version: '2.1.0', source: system, spec_yaml: 'system/agent-engine', digest: '', signed_by: '', tags: '', vertical: '', published: 'true', created_by: test, created_at: '2026-11-12', updated_at: '2026-11-12' }
        _resource: a3t/a3t-platform-db
` });
  check('seed template', !(await run('a3t', 'ctl-test-seed', {})).isError);
  const ctl = input => run('a3t', 'a3t-agent-control', { org_id: 'org-default', user_id: 'tester', ...input });

  // ── create ──
  let r = await ctl({ action: 'create', id: 'agt-parent', name: 'support-main', project: 'acme', from: { kind: 'template', id: 'tpl-support' }, values: { system_prompt: 'You help Acme customers.', llm_resource: 'a3t/x' } });
  check('create from template', r.out?.ok === true && r.out?.agent?.id === 'agt-parent' && r.out?.relation?.kind === 'template', r.text);
  check('  endpoints: control + data', r.out?.endpoints?.control === '/agents/agt-parent' && r.out?.endpoints?.data?.session_id === 'agt-parent:<session>', JSON.stringify(r.out?.endpoints));
  r = await ctl({ action: 'create', id: 'agt-child', name: 'support-eu', from: { kind: 'agent', id: 'agt-parent' }, mode: 'inherit', values: { temperature: 0.2 } });
  check('create inherit from agent', r.out?.ok === true && r.out?.relation?.kind === 'inherit', r.text);
  r = await ctl({ action: 'get', id: 'agt-child' });
  check('get: values inherited and overridden', r.out?.agent?.variables?.system_prompt === 'You help Acme customers.' && r.out?.agent?.variables?.temperature === 0.2 && r.out?.agent?.namespace === 'acme', JSON.stringify(r.out?.agent));
  check('get: relation and private by default', r.out?.relation?.parent?.id === 'agt-parent' && r.out?.visibility === 'private', JSON.stringify([r.out?.relation, r.out?.visibility]));
  r = await ctl({ action: 'create', name: 'x', from: { kind: 'agent', id: 'agt-nope' } });
  check('create: unknown source refused', r.isError && /agt-nope/.test(r.text), r.text);
  r = await ctl({ action: 'create', name: 'x', from: { kind: 'agent', id: 'agt-parent' }, mode: 'fork' });
  check('create: fork goes through FlowR, refused here', r.isError && /fork/i.test(r.text), r.text);

  // ── relate replaces ──
  await ctl({ action: 'create', id: 'agt-root', name: 'root', from: { kind: 'template', id: 'tpl-support' } });
  await ctl({ action: 'create', id: 'agt-moved', name: 'moved', from: { kind: 'template', id: 'tpl-support' } });
  r = await ctl({ action: 'relate', id: 'agt-moved', parent: { kind: 'agent', id: 'agt-root' }, relation: 'inherit' });
  r = await ctl({ action: 'lineage', id: 'agt-moved' });
  check('relate replaces the relation', JSON.stringify((r.out?.ancestors ?? []).map(a => a.id)) === JSON.stringify(['agt-root', 'tpl-support']), JSON.stringify(r.out));
  r = await ctl({ action: 'lineage', id: 'agt-root' });
  check('  one row per agent', (r.out?.descendants ?? []).filter(d => d.id === 'agt-moved').length === 1, JSON.stringify(r.out));

  // ── lineage ──
  r = await ctl({ action: 'create', id: 'agt-grand', name: 'support-eu-de', from: { kind: 'agent', id: 'agt-child' }, mode: 'clone' });
  r = await ctl({ action: 'lineage', id: 'agt-grand' });
  check('lineage: ancestors up to the template', JSON.stringify((r.out?.ancestors ?? []).map(a => a.id)) === JSON.stringify(['agt-child', 'agt-parent', 'tpl-support']), JSON.stringify(r.out));
  r = await ctl({ action: 'lineage', id: 'agt-parent' });
  check('lineage: descendants', (r.out?.descendants ?? []).map(d => d.id).includes('agt-child'), JSON.stringify(r.out));

  // ── fork made by FlowR, then compare / merge / evolve ──
  await must('create_flow', { yaml: `document: { dsl: '1.0.3', namespace: a3t, name: ctl-test-fork-row, version: '1.0.0' }
do:
  - row:
      call: lancedb
      with:
        operation: insert
        table: agent_configs
        records:
          - { id: '\${ .id }', org_id: org-default, name: '\${ .id }', template_id: tpl-support, template_version: '2.1.0', mode: persistent, variables: '\${ .variables }', resource_bindings: '{}', namespace: acme, status: draft, created_by: test, created_at: x, updated_at: x }
        _resource: a3t/a3t-platform-db
` });
  const parentVars = JSON.stringify({ system_prompt: 'You help Acme customers.', llm_resource: 'a3t/x' });
  await run('a3t', 'ctl-test-fork-row', { id: 'agt-fork-ok', variables: parentVars });
  await run('a3t', 'ctl-test-fork-row', { id: 'agt-fork-changed', variables: JSON.stringify({ system_prompt: 'Different.', llm_resource: 'a3t/x' }) });
  for (const id of ['agt-fork-ok', 'agt-fork-changed']) {
    r = await ctl({ action: 'relate', id, parent: { kind: 'agent', id: 'agt-parent' }, relation: 'fork' });
    check(`relate ${id}`, r.out?.ok === true, r.text);
  }
  await aql('STORE INTO PROCEDURAL (key = "proc-refund", name = "refund", pattern = "refund order", steps = "get_order then refund") NAMESPACE "acme--agt-fork-ok"');
  await aql('STORE INTO PROCEDURAL (key = "proc-track", name = "track", pattern = "where is my order", steps = "get_order") NAMESPACE "acme--agt-fork-ok"');
  await aql('STORE INTO PROCEDURAL (key = "proc-refund", name = "refund", pattern = "refund", steps = "old") NAMESPACE "acme--agt-parent"');
  r = await ctl({ action: 'compare', id: 'agt-fork-ok' });
  check('compare: procedures only → mergeable', r.out?.mergeable === true && r.out?.next === 'merge', JSON.stringify(r.out));
  r = await ctl({ action: 'compare', id: 'agt-fork-changed' });
  check('compare: config changed → evolve', r.out?.mergeable === false && (r.out?.changed ?? []).includes('values.system_prompt'), JSON.stringify(r.out));
  r = await ctl({ action: 'merge', id: 'agt-fork-ok' });
  check('merge: procedures into the source', r.out?.ok === true && r.out?.procedures === 2, r.text);
  const merged = await aql('RECALL FROM PROCEDURAL ALL NAMESPACE "acme--agt-parent" LIMIT 10');
  check('  source has the new and the updated procedure', merged.length === 2 && merged.some(m => m.data?.key === 'proc-refund' && JSON.stringify(m.data?.steps) === JSON.stringify(['get_order then refund'])), JSON.stringify(merged.map(m => m.data)));
  r = await ctl({ action: 'merge', id: 'agt-fork-changed' });
  check('merge: refused when more than procedures changed', r.isError && /evolve/i.test(r.text), r.text);
  r = await ctl({ action: 'evolve', id: 'agt-fork-changed' });
  r = await ctl({ action: 'get', id: 'agt-fork-changed' });
  check('evolve: relation evolved, ancestor kept', r.out?.relation?.kind === 'evolved' && r.out?.relation?.parent?.id === 'agt-parent', JSON.stringify(r.out?.relation));

  // ── sharing ──
  r = await ctl({ action: 'share', id: 'agt-parent', target: 'globex', rights: { call: true, fork: true } });
  check('share with a namespace', r.out?.ok === true, r.text);
  r = await ctl({ action: 'share', id: 'agt-parent', target: 'globex', rights: { call: true, inherit: true } });
  r = await ctl({ action: 'shares', id: 'agt-parent' });
  check('  sharing again replaces the rights', (r.out?.shares ?? []).length === 1 && r.out.shares[0].can_inherit === 'true' && r.out.shares[0].can_fork === 'false', JSON.stringify(r.out));
  await ctl({ action: 'share', id: 'agt-child', target: '*' });
  r = await ctl({ action: 'get', id: 'agt-child' });
  check('public visibility', r.out?.visibility === 'public', JSON.stringify(r.out?.visibility));
  r = await ctl({ action: 'shared_with', namespace: 'globex' });
  check('shared_with: the namespace’s and public ones', JSON.stringify((r.out?.shares ?? []).map(s => s.agent_id).sort()) === JSON.stringify(['agt-child', 'agt-parent']), JSON.stringify(r.out));
  await ctl({ action: 'unshare', id: 'agt-parent', target: 'globex' });
  r = await ctl({ action: 'shares', id: 'agt-parent' });
  check('unshare', (r.out?.shares ?? []).length === 0, JSON.stringify(r.out));
  r = await ctl({ action: 'nope' });
  check('unknown action refused', r.isError && /unknown action/i.test(r.text), r.text);
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
