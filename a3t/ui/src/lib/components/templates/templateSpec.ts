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
  parseError?: string;
}

const empty = (): TemplateFacts => ({ inputs: [], outputs: [], agent: { present: false, tools: [], mcpServers: [] }, hooks: [] });

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
  const ext = Array.isArray(d.use?.extensions) ? d.use.extensions : [];
  for (const e of ext) for (const [name, h] of Object.entries(e ?? {}) as [string, any][]) {
    if ((Array.isArray(h?.before) && h.before.length) || (Array.isArray(h?.after) && h.after.length)) out.hooks.push(name);
  }
  return out;
}
