// Step 1 on vc-dev: new names live, data and triggers moved, old flows archived.
//   node step1-roll.mjs <token-file> publish|data|triggers|deploy|archive [--dry]
// Saves every definition it changes or removes to archive-backup/step1/. Token never printed.
import { readFileSync, writeFileSync, mkdirSync } from 'node:fs';
import { execFileSync } from 'node:child_process';
const T = readFileSync(process.argv[2], 'utf8').trim();
const phase = process.argv[3];
const DRY = process.argv.includes('--dry');
const REPO = '/home/coder/sri-dev/Dev/flowr-system-agent/a3t/backend/flows';
const BK = 'archive-backup/step1';
mkdirSync(BK, { recursive: true });
const MAP = JSON.parse(execFileSync('python3', ['-c', `import sys,json;sys.path.insert(0,'.');from rename_map import FLOWS;print(json.dumps(FLOWS))`]).toString());
const tool = async (name, args) => {
  const j = await (await fetch('https://console.dev.a3t.io/message', { method: 'POST', headers: { 'content-type': 'application/json', authorization: `Bearer ${T}` }, body: JSON.stringify({ jsonrpc: '2.0', id: 1, method: 'tools/call', params: { name, arguments: args } }) })).json();
  return { err: !!(j.error || j.result?.isError), text: j.result?.content?.map(c => c.text).join('\n') ?? JSON.stringify(j.error ?? j) };
};
const out = t => { const i = t.indexOf('Output:\n'); try { return JSON.parse(i < 0 ? t.slice(t.indexOf('{')) : t.slice(i + 8)); } catch { return null; } };
const run = async (ns, name, input) => { const r = await tool('execute_flow', { namespace: ns, name, input }); return { ...r, out: out(r.text) }; };
const short = t => String(t).replace(/\s+/g, ' ').slice(0, 260);
const now = () => new Date().toISOString();
let bad = 0;
const say = (ok, msg) => { console.log(`${ok ? '✓' : '✗'} ${msg}`); if (!ok) bad++; };

if (phase === 'publish') {
  // Base first, then system, then a3t (tmpl_a3t_agent extends tmpl_system_agent_engine).
  const order = [...MAP.filter(m => m[2] === 'flow_system_base'), ...MAP.filter(m => m[2] === 'flow_system_error_handler'),
    ...MAP.filter(m => m[0] === 'system' && !['flow_system_base', 'flow_system_error_handler'].includes(m[2])),
    ...MAP.filter(m => m[0] === 'a3t')];
  const extra = [['system', 'agent-template', 'agent-template', 'agent-template.yaml']];
  for (const [ns, , neu] of [...order, ...extra]) {
    const y = readFileSync(`${REPO}/${ns}/${neu}.yaml`, 'utf8');
    const ver = (y.match(/^\s+version:\s*['"]?([\d.]+)/m) ?? [])[1];
    if (DRY) { console.log(`· ${ns}/${neu} v${ver}`); continue; }
    const c = await tool('create_flow', { yaml: y });
    const u = await tool('update_flow', { namespace: ns, name: neu, version: ver, enabled: true });
    say(!u.err, `${ns}/${neu} v${ver}${c.err && !/exist/i.test(c.text) ? ' create: ' + short(c.text) : ''}${u.err ? ' activate: ' + short(u.text) : ''}`);
  }
}

if (phase === 'data') {
  const TPL = { 'tpl-agent-engine': 'tmpl_system_agent_engine', 'tpl-a3t-agent': 'tmpl_a3t_agent', 'tpl-system-shared-agent-engine': 'tmpl_a3t_agent', 'tpl-a3t-shared-agent-engine-mvx11oax': 'tmpl_a3t_agent' };
  const VER = { tmpl_system_agent_engine: '2.1.0', tmpl_a3t_agent: '4.0.0' };
  const TARGET = { 'a3t/a3t-chat-engine': 'a3t/tmpl_a3t_agent', 'a3t/a3t-chat-agent': 'a3t/tmpl_a3t_agent', 'system/agent-engine': 'system/tmpl_system_agent_engine' };
  // 1. New template rows.
  for (const [id, file, tags] of [['tmpl_system_agent_engine', 'system/tmpl_system_agent_engine.yaml', 'core,lifecycle,memory,knowledge,tool-catalogue,turn-log'], ['tmpl_a3t_agent', 'a3t/tmpl_a3t_agent.yaml', 'a3t,core,memory,knowledge,tools,turn-log']]) {
    const data = { id, name: id, version: VER[id], source: 'system', spec_yaml: readFileSync(`${REPO}/${file}`, 'utf8'), tags, vertical: 'platform', published: 'true' };
    if (DRY) { console.log(`· template row ${id}`); continue; }
    const r = await run('a3t', 'flow_a3t_templates', { input: { action: 'create', org_id: 'org-default', user_id: 'system', timestamp: now(), data } });
    say(!r.err, `template row ${id}${r.err ? ': ' + short(r.text) : ''}`);
  }
  // 2. Agents: template and target to the new names.
  const l = await run('a3t', 'flow_a3t_agents', { input: { action: 'list', org_id: 'org-default' } });
  const rows = Array.isArray(l.out) ? l.out : (l.out?.rows ?? []);
  writeFileSync(`${BK}/agents.before.json`, JSON.stringify(rows, null, 2));
  for (const a of rows) {
    let v = {}; try { v = JSON.parse(a.variables || '{}'); } catch {}
    const tpl = TPL[a.template_id] ?? a.template_id;
    const target = TARGET[v.target] ?? v.target;
    if (tpl === a.template_id && target === v.target) { console.log(`· ${a.id} ${a.name}: nothing to change`); continue; }
    if (target) v.target = target;
    const data = { name: a.name, template_id: tpl, template_version: VER[tpl] ?? a.template_version, mode: a.mode, variables: JSON.stringify(v), resource_bindings: typeof a.resource_bindings === 'string' ? a.resource_bindings : JSON.stringify(a.resource_bindings ?? {}), namespace: a.namespace, status: a.status };
    if (DRY) { console.log(`· ${a.id} ${a.name}: ${a.template_id} → ${tpl}, target → ${target}`); continue; }
    const r = await run('a3t', 'flow_a3t_agents', { input: { action: 'update', id: a.id, org_id: a.org_id, timestamp: now(), data } });
    say(!r.err, `agent ${a.id} ${a.name}: template ${tpl}, target ${target}${r.err ? ': ' + short(r.text) : ''}`);
  }
  // 3. Relations that point at a renamed template.
  if (!DRY) {
    const r = await run('a3t', 'flow_a3t_agent_control', { action: 'relate', org_id: 'org-default', user_id: 'system', id: 'agt-system-agent', parent: { kind: 'template', id: 'tmpl_system_agent_engine', version: '2.1.0' }, relation: 'template' });
    say(!r.err, `relation agt-system-agent → tmpl_system_agent_engine${r.err ? ': ' + short(r.text) : ''}`);
  }
  // 4. Old template rows: saved, then removed.
  for (const id of Object.keys(TPL)) {
    const g = await run('a3t', 'flow_a3t_templates', { input: { action: 'get', id, org_id: 'org-default' } });
    writeFileSync(`${BK}/template.${id}.json`, g.text);
    if (DRY) { console.log(`· old template row ${id}: would remove`); continue; }
    const d = await run('a3t', 'flow_a3t_templates', { input: { action: 'delete', id, org_id: 'org-default' } });
    say(!d.err, `old template row ${id} saved and removed${d.err ? ': ' + short(d.text) : ''}`);
  }
}

if (phase === 'triggers') {
  const MOVE = { 'a3t-chat-engine': 'tmpl_a3t_agent', 'a3t-early-access': 'flow_a3t_early_access' };
  const l = await tool('list_triggers', { namespace: 'a3t' });
  const list = (out(l.text)?.triggers) ?? [];
  for (const t0 of list) {
    const g = await tool('get_trigger', { id: t0.id });
    const t = out(g.text);
    if (!t) { say(false, `trigger ${t0.name}: not read`); continue; }
    const targets = (t.targets ?? []).map(x => (x.namespace === 'a3t' && MOVE[x.name] ? { ...x, name: MOVE[x.name] } : x));
    if (JSON.stringify(targets) === JSON.stringify(t.targets)) continue;
    writeFileSync(`${BK}/trigger.${t.id}.json`, g.text);
    if (DRY) { console.log(`· trigger ${t.name} → ${targets.map(x => x.name).join(', ')}`); continue; }
    const u = await tool('update_trigger', { id: t.id, name: t.name, namespace: t.namespace, description: t.description ?? undefined, config: t.config, targets, enabled: t.enabled });
    say(!u.err, `trigger ${t.name} → ${targets.map(x => x.name).join(', ')}${u.err ? ': ' + short(u.text) : ''}`);
  }
}

if (phase === 'deploy') {
  if (!DRY) {
    const d = await tool('deploy_flow', { namespace: 'a3t', name: 'flow_a3t_profile', runtime: 'rt-default' });
    say(!d.err, `deploy a3t/flow_a3t_profile${d.err ? ': ' + short(d.text) : ''}`);
    const u = await tool('undeploy', { deployment: 'flow.a3t.a3t-profile' });
    say(!u.err, `undeploy flow.a3t.a3t-profile${u.err ? ': ' + short(u.text) : ''}`);
  }
}

if (phase === 'archive') {
  // Dependents first (a3t), then system, the bases last. agent-template stays (FlowR uses it until its next build rolls).
  const order = [...MAP.filter(m => m[0] === 'a3t'), ...MAP.filter(m => m[0] === 'system' && !['flow_system_base', 'flow_system_error_handler'].includes(m[2])),
    ...MAP.filter(m => m[2] === 'flow_system_error_handler'), ...MAP.filter(m => m[2] === 'flow_system_base')];
  const extra = [['a3t', 'a3t-chat-agent']];
  const renameRefs = y => execFileSync('python3', ['-c', `import sys;sys.path.insert(0,'.');from rename_map import rename_text;sys.stdout.write(rename_text(sys.stdin.read()))`], { input: y }).toString();
  for (const [ns, old] of [...extra, ...order.map(m => [m[0], m[1]])]) {
    const g = await tool('get_flow', { namespace: ns, name: old });
    const y = (g.text.match(/```yaml\n([\s\S]*?)```/) ?? [])[1];
    if (g.err || !y) { console.log(`· ${ns}/${old}: not on vc-dev`); continue; }
    writeFileSync(`${BK}/${ns}.${old}.yaml`, y);
    // The archive copy keeps its old name; its references point at the new names, so nothing old stays referenced.
    let copy = renameRefs(y).replace(/^(\s+namespace:\s*)["']?(a3t|system)["']?\s*$/m, '$1system-archive').replace(/namespace: (a3t|system),/, 'namespace: system-archive,');
    copy = copy.replace(new RegExp(`^(\\s+name:\\s*)["']?[\\w-]+["']?\\s*$`, 'm'), `$1${old}`).replace(/(document: \{[^}]*name: )[\w-]+/, `$1${old}`);
    if (DRY) { console.log(`· ${ns}/${old} → system-archive`); continue; }
    const c = await tool('create_flow', { yaml: copy });
    const chk = await tool('get_flow', { namespace: 'system-archive', name: old });
    if (chk.err) { say(false, `${ns}/${old}: archive copy failed: ${short(c.text)}`); continue; }
    const d = await tool('delete_flow', { namespace: ns, name: old });
    say(!d.err, `${ns}/${old} → system-archive${d.err ? ': original kept: ' + short(d.text) : ''}`);
  }
}
console.log(bad ? `${bad} problem(s)` : 'done');
