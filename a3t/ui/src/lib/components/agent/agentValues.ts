// An agent's values, as a coder edits them: a small YAML document over the
// agent record's bindings. The template is the code; these are what this agent
// fills in. Keys are named for what they do, and map one-to-one onto the
// record (`resource_bindings.llm`, `variables.*`, `mode`).
import * as YAML from 'yaml';

export type Bindings = {
  name: string; description: string; environment: string; mode: string; llm: string; target: string; ep_path: string;
  mcp_servers: string[]; enabled_tools: string[] | null; system_prompt: string; max_iterations: number; temperature: number;
  /// Its own hook flows by phase (on_turn_start … on_turn_end), "<project>/<flow>"; the engine runs them.
  hooks?: Record<string, string[]>;
};

export const MODES = ['session', 'persistent', 'ephemeral', 'reactive'] as const;

/// key in the YAML ↔ field of the bindings, with what it means.
export const ENV_VALUES = ['dev', 'test', 'prod'] as const;

export const KEYS: { key: string; field: keyof Bindings; info: string }[] = [
  { key: 'environment', field: 'environment', info: 'dev | test | prod — where its owner put it' },
  { key: 'model', field: 'llm', info: 'LLM resource it thinks with (<project>/<name>)' },
  { key: 'instructions', field: 'system_prompt', info: 'System prompt, sent at the start of every conversation' },
  { key: 'tools', field: 'mcp_servers', info: 'MCP servers it may call (<project>/<name>)' },
  { key: 'tool_allowlist', field: 'enabled_tools', info: 'null = every tool of those servers; a list = only these tool names' },
  { key: 'memory', field: 'mode', info: 'session: per conversation · persistent: across conversations · ephemeral · reactive' },
  { key: 'temperature', field: 'temperature', info: '0 – 2' },
  { key: 'max_steps', field: 'max_iterations', info: 'Most tool-call rounds per message' },
  { key: 'served_by', field: 'target', info: 'Flow that runs each call (<project>/<name>)' },
  { key: 'endpoint', field: 'ep_path', info: 'Its webhook path: POST /webhook<endpoint>' },
];

export function toYaml(b: Bindings, header: string): string {
  const doc = new YAML.Document({
    environment: b.environment || null,
    model: b.llm || null,
    instructions: b.system_prompt ?? '',
    tools: b.mcp_servers ?? [],
    tool_allowlist: b.enabled_tools ?? null,
    memory: b.mode,
    temperature: Number(b.temperature),
    max_steps: Number(b.max_iterations),
    served_by: b.target || null,
    endpoint: b.ep_path || null,
  });
  const ins = doc.get('instructions', true) as any;
  // Prompts are prose: a block, wrapped by the editor, unless very short.
  const prompt = String(b.system_prompt ?? '');
  // Multi-line: kept as written. One long line: folded at 80 columns, which
  // reads back as the same single line.
  if (ins && prompt.includes('\n')) ins.type = 'BLOCK_LITERAL';
  else if (ins && prompt.length > 60) ins.type = 'BLOCK_FOLDED';
  return `# ${header}\n` + doc.toString({ lineWidth: 80, minContentWidth: 40 });
}

export type Issue = { line: number; level: 'error' | 'warning'; message: string };

/// The values in `text`, applied over `b`; and what is wrong with them.
export function fromYaml(text: string, b: Bindings): { values: Bindings | null; issues: Issue[] } {
  const lineOf = (offset: number) => text.slice(0, Math.max(0, offset)).split('\n').length;
  const doc = YAML.parseDocument(text);
  if (doc.errors.length) return { values: null, issues: doc.errors.map((e) => ({ line: lineOf(e.pos[0]), level: 'error' as const, message: e.message.split('\n')[0] })) };
  const o: any = doc.toJS() ?? {};
  if (typeof o !== 'object' || Array.isArray(o)) return { values: null, issues: [{ line: 1, level: 'error', message: 'Expected keys: ' + KEYS.map((k) => k.key).join(', ') }] };
  const issues: Issue[] = [];
  const at = (key: string) => { const n: any = (doc.contents as any)?.items?.find((p: any) => p.key?.value === key); return n ? lineOf(n.key.range?.[0] ?? 0) : 1; };
  for (const k of Object.keys(o)) if (!KEYS.some((x) => x.key === k)) issues.push({ line: at(k), level: 'warning', message: `Unknown key "${k}" — ignored` });
  const v: Bindings = { ...b };
  const str = (k: string) => (o[k] == null ? '' : String(o[k]));
  if ('environment' in o) {
    if (o.environment != null && !(ENV_VALUES as readonly string[]).includes(str('environment'))) issues.push({ line: at('environment'), level: 'error', message: `environment: one of ${ENV_VALUES.join(', ')}` });
    else v.environment = o.environment == null ? '' : str('environment');
  }
  if ('model' in o) v.llm = str('model');
  if ('instructions' in o) v.system_prompt = str('instructions');
  if ('tools' in o) {
    if (o.tools != null && !Array.isArray(o.tools)) issues.push({ line: at('tools'), level: 'error', message: 'tools: a list of <project>/<name>' });
    else v.mcp_servers = (o.tools ?? []).map(String);
  }
  if ('tool_allowlist' in o) {
    if (o.tool_allowlist != null && !Array.isArray(o.tool_allowlist)) issues.push({ line: at('tool_allowlist'), level: 'error', message: 'tool_allowlist: null, or a list of tool names' });
    else v.enabled_tools = o.tool_allowlist == null ? null : o.tool_allowlist.map(String);
  }
  if ('memory' in o) {
    if (!(MODES as readonly string[]).includes(str('memory'))) issues.push({ line: at('memory'), level: 'error', message: `memory: one of ${MODES.join(', ')}` });
    else v.mode = str('memory');
  }
  for (const [k, f, lo, hi] of [['temperature', 'temperature', 0, 2], ['max_steps', 'max_iterations', 1, 100]] as const) {
    if (!(k in o)) continue;
    const n = Number(o[k]);
    if (!Number.isFinite(n) || n < lo || n > hi) issues.push({ line: at(k), level: 'error', message: `${k}: a number from ${lo} to ${hi}` });
    else (v as any)[f] = n;
  }
  if ('served_by' in o) v.target = str('served_by');
  if ('endpoint' in o) {
    if (o.endpoint && !str('endpoint').startsWith('/')) issues.push({ line: at('endpoint'), level: 'error', message: 'endpoint: a path starting with /' });
    else v.ep_path = str('endpoint');
  }
  return { values: issues.some((i) => i.level === 'error') ? null : v, issues };
}

/// Completion for the values editor: keys at the start of a line, choices after `key: `.
export function completer(choices: { model: string[]; tools: string[]; flows: string[] }) {
  return (text: string, pos: number) => {
    const lineStart = text.lastIndexOf('\n', pos - 1) + 1;
    const line = text.slice(lineStart, pos);
    const m = line.match(/^(\w+):\s*(\S*)$/);
    if (m) {
      const list = m[1] === 'model' ? choices.model : m[1] === 'served_by' ? choices.flows : m[1] === 'memory' ? [...MODES] : m[1] === 'environment' ? [...ENV_VALUES] : [];
      if (!list.length) return null;
      return { from: pos - m[2].length, options: list.map((x) => ({ label: x, apply: x })) };
    }
    const item = line.match(/^\s+-\s*(\S*)$/);
    if (item && /\ntools:\s*\n(?:\s+-.*\n)*$/.test(text.slice(0, lineStart))) {
      return { from: pos - item[1].length, options: choices.tools.map((x) => ({ label: x, apply: x })) };
    }
    const key = line.match(/^(\w*)$/);
    if (key) return { from: lineStart, options: KEYS.map((k) => ({ label: k.key, detail: k.field, info: k.info, apply: `${k.key}: ` })) };
    return null;
  };
}

/// What the template's code connects to, by name: the resources and flows its
/// steps call. Their values differ per environment (dev, prod).
export function usesOf(spec: any): { name: string; kind: string }[] {
  const out = new Map<string, string>();
  const walk = (v: any, key = '') => {
    if (!v || typeof v !== 'object') return;
    if (Array.isArray(v)) { v.forEach((x) => walk(x)); return; }
    for (const [k, x] of Object.entries(v)) {
      if (k === '_resource' && typeof x === 'string') out.set(x, 'resource');
      else if (k === 'llm_resource' && typeof x === 'string' && !x.includes('${')) out.set(x, 'model');
      else if (k === 'mcp_servers' && Array.isArray(x)) x.forEach((s) => typeof s === 'string' && out.set(s, 'tool server'));
      else if (k === 'memory_namespace' && typeof x === 'string') out.set(x, 'memory');
      else if (k === 'call' && typeof x === 'string' && !/^(http|grpc|openapi|asyncapi|a3t:agent)$/.test(x)) out.set(x, 'call');
      else if (k === 'workflow' && x && typeof x === 'object' && (x as any).name) out.set(`${(x as any).namespace ?? ''}/${(x as any).name}`.replace(/^\//, ''), 'flow');
      else walk(x, k);
    }
    void key;
  };
  walk(spec?.do); walk(spec?.use);
  return [...out].map(([name, kind]) => ({ name, kind }));
}
