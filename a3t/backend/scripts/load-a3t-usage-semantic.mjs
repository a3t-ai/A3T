#!/usr/bin/env node
/**
 * Sample A3T usage facts into a SEMANTIC space on a FlowR's ADB, as a knowledge graph:
 * each fact is subject → predicate → object, with its sentence (content), a confidence
 * and where it came from. An agent sees them once the space is attached to it
 * (Build › Memory › Semantic › Attach, or tool_a3t_agent_memory_attach).
 *
 *   FLOWR_TOKEN_FILE=<file with a console token> node a3t/backend/scripts/load-a3t-usage-semantic.mjs \
 *     --url https://console.dev.a3t.io [--namespace a3t-usage] [--resource a3t/adb] [--dry-run]
 *
 * Safe to run again: an existing fact (same key) is updated in place. The token is read
 * from the file and never printed. --dry-run prints the AQL and calls nothing.
 */
import { readFileSync } from 'node:fs';

const args = Object.fromEntries(process.argv.slice(2).reduce((a, v, i, all) => (v.startsWith('--') ? [...a, [v.slice(2), all[i + 1]?.startsWith('--') || all[i + 1] === undefined ? 'true' : all[i + 1]]] : a), []));
const DRY = args['dry-run'] === 'true';
const NS = args.namespace ?? 'a3t-usage';
const [RNS, RNAME] = (args.resource ?? 'a3t/adb').split('/');
const URL_ = (args.url ?? 'https://console.dev.a3t.io').replace(/\/$/, '');

// [subject, predicate, object, confidence, source]
const FACTS = [
  // Agents and what they run on
  ['agent_a3t_assistant', 'runs template', 'tmpl_a3t_agent', 1, 'agent_configs'],
  ['agent_a3t_assistant', 'uses model', 'claude-cli-max', 1, 'agent_configs'],
  ['agent_a3t_assistant', 'answers in', 'A3T console chat', 0.95, 'turn log'],
  ['agent_a3t_assistant', 'reads', 'system-knowledge', 1, 'engine config'],
  ['agent_a3t_assistant', 'calls tool', 'tool_a3t_agent_list', 0.9, 'turn log'],
  ['agent_a3t_assistant', 'calls tool', 'tool_a3t_resource_list', 0.85, 'turn log'],
  ['agent_a3t_agents', 'runs template', 'tmpl_a3t_service', 1, 'agent_configs'],
  ['agent_a3t_agents', 'manages', 'user agents', 1, 'agents install'],
  ['agent_a3t_agents', 'calls flow', 'flow_a3t_agent_control', 1, 'tool_flows'],
  ['agent_a3t_knowledge', 'manages', 'knowledge graph', 1, 'agents install'],
  ['agent_a3t_governance', 'reads', 'audit log', 1, 'agents install'],
  ['agent_a3t_resources', 'manages', 'resources', 1, 'agents install'],
  ['agent_system_core', 'is root of', 'every A3T agent', 1, 'AGENT_ENGINE.md'],
  ['support-bot', 'runs template', 'tmpl_a3t_agent', 1, 'agent_configs'],
  ['support-bot', 'uses model', 'claude-sonnet', 0.9, 'agent_configs'],
  ['support-bot', 'handled', '1,240 conversations last week', 0.8, 'a3t_agent_turns'],
  ['support-bot', 'escalates to', 'human support', 0.7, 'turn log'],
  ['support-bot', 'calls tool', 'tool_zendesk_ticket_create', 0.85, 'turn log'],
  ['sales-researcher', 'forked from', 'support-bot', 1, 'lineage'],
  ['sales-researcher', 'uses model', 'claude-opus', 0.9, 'agent_configs'],
  ['sales-researcher', 'calls tool', 'tool_web_search', 0.9, 'turn log'],
  ['sales-researcher', 'writes', 'CRM notes', 0.75, 'turn log'],
  ['invoice-checker', 'runs template', 'tmpl_a3t_agent', 1, 'agent_configs'],
  ['invoice-checker', 'uses model', 'claude-haiku', 0.9, 'agent_configs'],
  ['invoice-checker', 'reads', 'finance-docs', 0.85, 'semantic spaces'],
  ['invoice-checker', 'blocked by', 'policy: no external email', 0.8, 'audit log'],
  // Templates and the engine
  ['tmpl_a3t_agent', 'extends', 'tmpl_system_agent_engine', 1, 'flow metadata'],
  ['tmpl_a3t_service', 'extends', 'tmpl_system_agent_engine', 1, 'flow metadata'],
  ['tmpl_system_agent_engine', 'recalls from', 'SEMANTIC memory', 1, 'AGENT_ENGINE.md'],
  ['tmpl_system_agent_engine', 'recalls from', 'PROCEDURAL memory', 1, 'AGENT_ENGINE.md'],
  ['tmpl_system_agent_engine', 'writes', 'a3t_agent_turns', 1, 'AGENT_ENGINE.md'],
  ['tmpl_system_agent_engine', 'reports errors to', 'flow_system_error_handler', 1, 'AGENT_ENGINE.md'],
  // Memory and storage
  ['ADB', 'stores', 'agent memory', 1, 'architecture'],
  ['ADB', 'replicates with', 'Raft', 1, 'architecture'],
  ['ADB', 'stores data in', 'Lance', 1, 'architecture'],
  ['ADB', 'is queried with', 'AQL', 1, 'architecture'],
  ['SEMANTIC memory', 'holds', 'facts', 1, 'AGENT_ENGINE.md'],
  ['EPISODIC memory', 'holds', 'conversation turns', 1, 'AGENT_ENGINE.md'],
  ['WORKING memory', 'holds', 'the conversation in hand', 1, 'AGENT_ENGINE.md'],
  ['PROCEDURAL memory', 'holds', 'learned steps and tools', 1, 'AGENT_ENGINE.md'],
  ['system-knowledge', 'is a', 'SEMANTIC space', 1, 'loader'],
  ['finance-docs', 'is a', 'SEMANTIC space', 1, 'loader'],
  ['a3t-usage', 'is a', 'SEMANTIC space', 1, 'loader'],
  // Platform
  ['FlowR', 'runs', 'tmpl_system_agent_engine', 1, 'architecture'],
  ['FlowR', 'reads memory from', 'ADB', 1, 'architecture'],
  ['A3T console', 'calls', 'agent_a3t_agents', 1, 'UI'],
  ['A3T console', 'is used by', 'security and governance teams', 0.8, 'product notes'],
  ['VS Code + Claude', 'builds agents over', 'MCP', 0.85, 'product notes'],
  ['MCP', 'exposes', 'agent tools', 1, 'architecture'],
  ['org-default', 'owns', 'support-bot', 1, 'projects'],
  ['org-default', 'owns', 'invoice-checker', 1, 'projects'],
  ['org-default', 'owns', 'sales-researcher', 1, 'projects'],
  ['org-default', 'stores memory on', 'a3t/adb', 1, 'resources'],
  ['claude-cli-max', 'is a', 'model resource', 1, 'resources'],
  ['claude-sonnet', 'is a', 'model resource', 1, 'resources'],
  ['claude-opus', 'is a', 'model resource', 1, 'resources'],
  ['claude-haiku', 'is a', 'model resource', 1, 'resources'],
];

const q = s => `"${String(s).replace(/\\/g, '\\\\').replace(/"/g, '\\"').replace(/\n/g, '\\n')}"`;
const fact = ([subject, predicate, object, confidence, source], i) => {
  const key = `usage-${String(i + 1).padStart(3, '0')}`;
  const content = `${subject} ${predicate} ${object}.`;
  const f = `subject = ${q(subject)}, predicate = ${q(predicate)}, object = ${q(object)}, content = ${q(content)}, confidence = ${confidence}, source = ${q(source)}, kind = "fact"`;
  return { key, store: `STORE INTO SEMANTIC (key = ${q(key)}, ${f}) NAMESPACE ${q(NS)}`, update: `UPDATE INTO SEMANTIC WHERE key = ${q(key)} (${f}) NAMESPACE ${q(NS)}` };
};

const T = DRY ? '' : readFileSync(process.env.FLOWR_TOKEN_FILE ?? '', 'utf8').trim();
async function aql(query) {
  const j = await (await fetch(`${URL_}/message`, { method: 'POST', headers: { 'content-type': 'application/json', authorization: `Bearer ${T}` },
    body: JSON.stringify({ jsonrpc: '2.0', id: 1, method: 'tools/call', params: { name: 'query_resource', arguments: { query, resource: RNAME, namespace: RNS } } }) })).json();
  const text = j.result?.content?.map(c => c.text).join('\n') ?? JSON.stringify(j.error ?? j);
  return { err: !!(j.error || j.result?.isError) || /error|invalid/i.test(text.slice(0, 120)), text };
}

let stored = 0, updated = 0, failed = 0;
for (const [i, f] of FACTS.entries()) {
  const x = fact(f, i);
  if (DRY) { console.log(x.store); continue; }
  let r = await aql(x.store);
  if (!r.err) { stored++; continue; }
  if (/exist/i.test(r.text)) { r = await aql(x.update); if (!r.err) { updated++; continue; } }
  failed++; console.log(`✗ ${x.key}: ${r.text.replace(/\s+/g, ' ').slice(0, 200)}`);
}
if (!DRY) {
  const r = await aql(`RECALL FROM SEMANTIC ALL NAMESPACE ${q(NS)} LIMIT 500`);
  const n = (r.text.match(/"subject"/g) ?? []).length;
  console.log(`${NS}: ${stored} stored, ${updated} updated, ${failed} failed; ${n} facts read back`);
  process.exit(failed ? 1 : 0);
}
