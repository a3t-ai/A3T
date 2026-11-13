// What a template's spec says, in plain terms, for the template guide (About,
// How to use, Try it, Production). A template is a CNCF workflow document whose
// agent step (`call: a3t:agent`) carries its settings. Templates differ in how
// much they declare, so everything here is optional: what is not in the spec
// is left out, never guessed.
import * as YAML from 'yaml';

export interface Field {
  name: string;
  type: string;
  required: boolean;
  description?: string;
}

export interface TemplateFacts {
  title?: string;
  summary?: string;
  description?: string;
  version?: string;
  namespace?: string;
  /// A copy: the template and version it was made from (`<project>/<name>@<version>`).
  copiedFrom?: string;
  /// What a caller passes in, and what comes back.
  inputs: Field[];
  outputs: Field[];
  /// The agent step's settings.
  agent: {
    present: boolean;
    model?: string;
    systemPrompt?: string;
    tools: string[];
    mcpServers: string[];
    memory?: string;
    maxIterations?: number;
    temperature?: number;
  };
  /// Lifecycle hooks with something in them (use.extensions).
  hooks: string[];
  /// It runs on the core agent (system/tmpl_system_agent_engine): it is the core,
  /// extends it, or runs it or an A3T agent template. Its memory is then the
  /// core's — each agent's own, on the org's ADB.
  onCore: boolean;
  /// What it extends (metadata.a3t.extends).
  extends?: string;
  parseError?: string;
}

const empty = (): TemplateFacts => ({ inputs: [], outputs: [], agent: { present: false, tools: [], mcpServers: [] }, hooks: [], onCore: false });

const CORE = 'tmpl_system_agent_engine';
/// Whether any task in the `do` tree runs one of these workflows.
function runsWorkflow(tasks: any, names: string[]): boolean {
  const list = Array.isArray(tasks) ? tasks : [];
  return list.some((entry) => entry && typeof entry === 'object' && (Object.values(entry) as any[]).some((t) =>
    names.includes(t?.run?.workflow?.name) || runsWorkflow(t?.do, names) || runsWorkflow(t?.try, names)));
}

/// JSON-schema properties (input.schema.document) or a contextSchema field list.
function fieldsOf(schema: any): Field[] {
  const doc = schema?.document ?? schema;
  if (Array.isArray(schema?.fields)) {
    return schema.fields.filter((f: any) => !f.injected).map((f: any) => ({
      name: String(f.name), type: String(f.type ?? 'any'), required: !!f.required, description: f.description,
    }));
  }
  const props = doc?.properties;
  if (!props || typeof props !== 'object') return [];
  const req: string[] = Array.isArray(doc.required) ? doc.required : [];
  return Object.entries(props).map(([name, p]: [string, any]) => ({
    name, type: String(p?.type ?? 'any'), required: req.includes(name), description: p?.description,
  }));
}

/// The first `call: a3t:agent` task, anywhere in the `do` tree.
function agentTask(tasks: any): any {
  const list = Array.isArray(tasks) ? tasks : [];
  for (const entry of list) {
    if (!entry || typeof entry !== 'object') continue;
    for (const t of Object.values(entry) as any[]) {
      if (t?.call === 'a3t:agent') return t;
      const inner = agentTask(t?.do) ?? agentTask(t?.try);
      if (inner) return inner;
    }
  }
  return null;
}

export function readTemplate(specYaml: string | undefined): TemplateFacts {
  const out = empty();
  if (!specYaml?.trim()) return out;
  let d: any;
  try { d = YAML.parse(specYaml); } catch (e) { out.parseError = String((e as Error).message ?? e); return out; }
  if (!d || typeof d !== 'object') return out;
  const doc = d.document ?? {};
  out.title = doc.title;
  out.summary = doc.summary;
  out.description = doc.metadata?.description ?? doc.description ?? d.metadata?.description;
  out.version = doc.version != null ? String(doc.version) : undefined;
  out.namespace = doc.namespace;
  out.copiedFrom = doc.metadata?.copiedFrom ?? undefined;
  const declared = fieldsOf(d.input?.schema);
  out.inputs = declared.length ? declared : fieldsOf(d.contextSchema);
  out.outputs = fieldsOf(d.output?.schema);
  const t = agentTask(d.do);
  if (t) {
    const c = t.with?.config ?? {};
    out.agent = {
      present: true,
      model: c.llm_resource ?? c.llm ?? undefined,
      systemPrompt: typeof c.system_prompt === 'string' ? c.system_prompt : undefined,
      tools: Array.isArray(c.tools) ? c.tools.map(String) : [],
      mcpServers: Array.isArray(c.mcp_servers) ? c.mcp_servers.map(String) : [],
      memory: c.memory_namespace ?? undefined,
      maxIterations: typeof c.max_iterations === 'number' ? c.max_iterations : undefined,
      temperature: typeof c.temperature === 'number' ? c.temperature : undefined,
    };
    // The agent step's own inputs, when the workflow declares none.
    if (!out.inputs.length && t.with) {
      for (const k of ['message', 'session_id']) if (k in t.with) out.inputs.push({ name: k, type: 'string', required: k === 'message' });
    }
  }
  out.extends = d.metadata?.a3t?.extends ?? undefined;
  out.onCore = doc.name === CORE || String(out.extends ?? '').includes(CORE) || runsWorkflow(d.do, [CORE, 'tmpl_a3t_agent']);
  if (out.onCore && !out.inputs.length) {
    out.inputs = [{ name: 'message', type: 'string', required: true, description: 'What the caller says' },
      { name: 'session_id', type: 'string', required: false, description: '<agent-id>:<conversation>' }];
  }
  const ext = Array.isArray(d.use?.extensions) ? d.use.extensions : [];
  for (const e of ext) for (const [name, h] of Object.entries(e ?? {}) as [string, any][]) {
    if ((Array.isArray(h?.before) && h.before.length) || (Array.isArray(h?.after) && h.after.length)) out.hooks.push(name);
  }
  return out;
}

/// A template's spec made into a copy's: its own name and project, version
/// 1.0.0, and where it came from. Edits the `document:` block line by line so
/// the rest of the spec — comments included — is kept as written.
export function specForCopy(spec: string, to: { name: string; namespace: string; from: string }): string {
  const lines = spec.split('\n');
  const start = lines.findIndex((l) => /^document:\s*$/.test(l));
  if (start < 0) return `document:\n  name: ${to.name}\n  namespace: ${to.namespace}\n  version: '1.0.0'\n  metadata:\n    copiedFrom: ${to.from}\n` + spec;
  let end = lines.findIndex((l, i) => i > start && /^\S/.test(l));
  if (end < 0) end = lines.length;
  const block = lines.slice(start + 1, end);
  const set = (key: string, value: string) => {
    const i = block.findIndex((l) => new RegExp(`^  ${key}:`).test(l));
    if (i >= 0) block[i] = `  ${key}: ${value}`; else block.push(`  ${key}: ${value}`);
  };
  set('name', to.name);
  set('namespace', to.namespace);
  set('version', "'1.0.0'");
  const m = block.findIndex((l) => /^  metadata:\s*$/.test(l));
  if (m >= 0) {
    const c = block.findIndex((l, i) => i > m && /^    copiedFrom:/.test(l));
    if (c >= 0) block[c] = `    copiedFrom: ${to.from}`; else block.splice(m + 1, 0, `    copiedFrom: ${to.from}`);
  } else block.push('  metadata:', `    copiedFrom: ${to.from}`);
  return [...lines.slice(0, start + 1), ...block, ...lines.slice(end)].join('\n');
}
