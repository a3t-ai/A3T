#!/usr/bin/env node
/**
 * Fill the core agent's shared memory on a FlowR:
 *
 *   knowledge: the FlowR docs (a3t/wwwdocs pages, plus the core agent's own guide)
 *              → system/agent-knowledge-ingest → SEMANTIC, namespace system-knowledge
 *   tools:     the platform's own MCP tools (listed here with FLOWR_TOKEN, as
 *              --self-server, default a3t/agent-flows) and any --servers FlowR lists
 *              itself → system/agent-tools-sync → PROCEDURAL, namespace system-tools
 *
 * Safe to run again: unchanged sections and tools are updated in place; removed ones
 * are forgotten.
 *
 *   FLOWR_TOKEN=<bearer> node a3t/backend/scripts/load-core-agent-memory.mjs \
 *     --url https://<flowr> [--resource a3t/agent-memory] [--self-server a3t/agent-flows] [--servers <ns/name>,...] \
 *     [--only knowledge|tools] [--dry-run]
 *
 * FLOWR_TOKEN is a user's access token (the console's), never printed. --dry-run
 * shows what would be sent and calls nothing.
 */
import { readFileSync, readdirSync, statSync } from 'node:fs';
import { dirname, join, relative } from 'node:path';
import { fileURLToPath } from 'node:url';

const args = Object.fromEntries(process.argv.slice(2).reduce((a, v, i, all) => (v.startsWith('--') ? [...a, [v.slice(2), all[i + 1]?.startsWith('--') || all[i + 1] === undefined ? 'true' : all[i + 1]]] : a), []));
const ROOT = join(dirname(fileURLToPath(import.meta.url)), '..', '..');
const DRY = args['dry-run'] === 'true';
if (!args.url && !DRY) {
  console.error('usage: FLOWR_TOKEN=… load-core-agent-memory.mjs --url <flowr> [--resource a3t/agent-memory] [--servers a,b] [--only knowledge|tools] [--dry-run]');
  process.exit(2);
}
const resource = args.resource ?? 'a3t/agent-memory';
const servers = (args.servers ?? '').split(',').filter(Boolean);
const selfServer = args['self-server'] ?? 'a3t/agent-flows';

/// The docs: each page's title (frontmatter) and its text without frontmatter,
/// imports and JSX component tags (their inner text is kept).
export function documents() {
  const dir = join(ROOT, 'wwwdocs', 'src', 'content', 'docs');
  const walk = d => readdirSync(d).flatMap(f => { const p = join(d, f); return statSync(p).isDirectory() ? walk(p) : /\.mdx?$/.test(f) ? [p] : []; });
  const pages = walk(dir).sort().map(p => {
    const raw = readFileSync(p, 'utf8');
    const fm = raw.match(/^---\n([\s\S]*?)\n---\n?/);
    const title = fm?.[1].match(/^title:\s*["']?(.+?)["']?\s*$/m)?.[1] ?? relative(dir, p);
    const text = (fm ? raw.slice(fm[0].length) : raw)
      .replace(/^import .*$/gm, '')
      .replace(/<\/?[A-Z][A-Za-z.]*(\s[^>]*)?\/?>/g, '')
      .replace(/\n{3,}/g, '\n\n')
      .trim();
    return { source: `docs/${relative(dir, p).replace(/\.mdx?$/, '')}`, title, text: `# ${title}\n${text}` };
  });
  const guide = join(ROOT, 'backend', 'flows', 'system', 'AGENT_ENGINE.md');
  try { pages.push({ source: 'core-agent/AGENT_ENGINE', title: 'The core agent', text: readFileSync(guide, 'utf8') }); } catch {}
  return pages;
}

async function rpc(method, params) {
  const r = await fetch(`${args.url.replace(/\/$/, '')}/message`, {
    method: 'POST',
    headers: { 'content-type': 'application/json', ...(process.env.FLOWR_TOKEN ? { authorization: `Bearer ${process.env.FLOWR_TOKEN}` } : {}) },
    body: JSON.stringify({ jsonrpc: '2.0', id: 1, method, params }),
  });
  if (!r.ok) throw new Error(`${method}: HTTP ${r.status}`);
  return r.json();
}
async function call(name, a) {
  const j = await rpc('tools/call', { name, arguments: a });
  const text = j.result?.content?.[0]?.text ?? JSON.stringify(j.error ?? j);
  if (j.error || j.result?.isError || /execution failed/i.test(text)) throw new Error(`${name}: ${text.slice(0, 600)}`);
  const i = text.indexOf('Output:\n');
  return i < 0 ? text : JSON.parse(text.slice(i + 8));
}

let failed = 0;
if (args.only !== 'tools') {
  const docs = documents();
  console.log(`knowledge: ${docs.length} documents → ${resource} system-knowledge`);
  if (DRY) for (const d of docs) console.log(`  ${d.source}  "${d.title}"  ${d.text.length} chars`);
  else {
    // A few documents per run, so one run stays short.
    for (let i = 0; i < docs.length; i += 4) {
      const batch = docs.slice(i, i + 4);
      try {
        const out = await call('execute_flow', { namespace: 'system', name: 'agent-knowledge-ingest', input: { documents: batch, resource } });
        console.log(`  ${batch.map(d => d.source).join(', ')}: stored ${out.stored}, updated ${out.updated}, forgotten ${out.forgotten}${out.failed?.length ? `, FAILED ${out.failed.length}` : ''}`);
        failed += out.failed?.length ?? 0;
      } catch (e) { console.error(`  ✗ ${e.message}`); failed++; }
    }
  }
}
if (args.only !== 'knowledge') {
  console.log(`tools: ${[selfServer, ...servers].filter(Boolean).join(', ')} → ${resource} system-tools`);
  if (!DRY) {
    try {
      // The platform's own tools, as the user sees them.
      const own = selfServer ? ((await rpc('tools/list', {})).result?.tools ?? []).map(t => ({ server: selfServer, name: t.name, description: t.description ?? '', inputSchema: t.inputSchema ?? {} })) : [];
      if (selfServer && !own.length) throw new Error(`${selfServer}: tools/list returned no tools`);
      const out = await call('execute_flow', { namespace: 'system', name: 'agent-tools-sync', input: { servers, tools: own, complete_for: selfServer ? [selfServer] : [], resource } });
      console.log(`  ${out.tools} tools: stored ${out.stored}, updated ${out.updated}, forgotten ${out.forgotten}${out.failed?.length ? `, FAILED: ${JSON.stringify(out.failed)}` : ''}`);
      failed += out.failed?.length ?? 0;
    } catch (e) { console.error(`  ✗ ${e.message}`); failed++; }
  }
}
process.exit(failed ? 1 : 0);
