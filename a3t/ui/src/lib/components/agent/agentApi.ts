// How another app calls an agent, as the server works today (see SRV-14/15 in
// .ai/A3T_UI_LIMITATIONS.md):
//
//   1. POST /webhook<ep_path> { message, session_id, …its settings } — starts a
//      run of its serving flow and answers { targets: ["<flow> (<run id>)"] }.
//   2. Poll get_execution_status (MCP JSON-RPC on POST /message) until the run
//      is completed; its output carries the answer.
//
// The server does not read the agent's record when it is called: the caller
// sends its model and tools (as A3T's own chat does). The snippets say so.
import { API_BASE } from '$lib/config';
import type { Field } from '$lib/components/templates/templateSpec';

/// The API's origin as an app outside A3T would use it.
export function apiBase(): string {
  const b = API_BASE || (typeof location !== 'undefined' ? location.origin : '');
  return b.startsWith('/') && typeof location !== 'undefined' ? `${location.origin}${b}` : b.replace(/\/$/, '');
}
export const endpointUrl = (ep: string) => `${apiBase()}/webhook${ep.startsWith('/') ? '' : '/'}${ep}`;

export async function copy(text: string): Promise<boolean> {
  try { await navigator.clipboard.writeText(text); return true; } catch { return false; }
}

/// The body an app sends: the template's declared inputs, the session, and the
/// settings the server does not yet look up itself.
export function requestBody(agentId: string, inputs: Field[], settings: { llm?: string; mcp: string[]; tools: string[] | null }): Record<string, unknown> {
  const body: Record<string, unknown> = {};
  const declared = inputs.filter((f) => f.name !== 'session_id');
  if (!declared.some((f) => f.name === 'message')) body.message = 'Hello';
  for (const f of declared) body[f.name] = f.name === 'message' ? 'Hello' : f.type === 'number' || f.type === 'integer' ? 0 : f.type === 'boolean' ? false : f.type === 'array' ? [] : f.type === 'object' ? {} : `<${f.name}>`;
  body.session_id = `${agentId}:<conversation-id>`;
  if (settings.llm) body.llm_resource = settings.llm;
  if (settings.mcp.length) body.mcp_servers = settings.mcp;
  if (settings.tools) body.enabled_tools = settings.tools;
  return body;
}

export type Lang = 'curl' | 'ts' | 'python';

/// The body as code: `message` and the conversation come from the function's
/// arguments, everything else is written out.
function bodyLiteral(body: Record<string, unknown>, lang: 'ts' | 'python', indent: string): string {
  const val = (v: unknown): string => lang === 'python'
    ? JSON.stringify(v).replace(/\bnull\b/g, 'None').replace(/\btrue\b/g, 'True').replace(/\bfalse\b/g, 'False')
    : JSON.stringify(v);
  const lines = Object.entries(body).map(([k, v]) => {
    const key = lang === 'python' ? JSON.stringify(k) : /^[a-z_]\w*$/i.test(k) ? k : JSON.stringify(k);
    let code = val(v);
    if (k === 'message') code = 'message';
    if (k === 'session_id') {
      const agent = String(v).split(':')[0];
      code = lang === 'python' ? `f"${agent}:{conversation_id}"` : `\`${agent}:\${conversationId}\``;
    }
    return `${indent}  ${key}: ${code},`;
  });
  return `{\n${lines.join('\n')}\n${indent}}`;
}

export function snippet(lang: Lang, url: string, body: Record<string, unknown>): string {
  const base = apiBase();
  if (lang === 'curl') return `# 1. Send a message — the answer is the run it started
curl -s -X POST "${url}" \\
  -H "Authorization: Bearer $A3T_TOKEN" \\
  -H "Content-Type: application/json" \\
  -d '${JSON.stringify(body)}'
# → {"targets":["<flow> (<run-id>)"]}

# 2. Poll the run until status is "completed"; output.content is the answer
curl -s -X POST "${base}/message" \\
  -H "Authorization: Bearer $A3T_TOKEN" \\
  -H "Content-Type: application/json" \\
  -d '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"get_execution_status","arguments":{"instance_id":"<run-id>"}}}'`;
  if (lang === 'ts') return `const A3T = '${base}';

export async function ask(token: string, message: string, conversationId: string): Promise<string> {
  const headers = { Authorization: \`Bearer \${token}\`, 'Content-Type': 'application/json' };
  const body = ${bodyLiteral(body, 'ts', '  ')};
  const start = await fetch('${url}', { method: 'POST', headers, body: JSON.stringify(body) }).then((r) => r.json());
  const runId = /\\(([0-9a-f-]+)\\)/.exec(start.targets?.[0] ?? '')?.[1];
  if (!runId) throw new Error('No run started');
  for (;;) {
    await new Promise((r) => setTimeout(r, 1000));
    const res = await fetch(\`\${A3T}/message\`, { method: 'POST', headers, body: JSON.stringify({
      jsonrpc: '2.0', id: 1, method: 'tools/call',
      params: { name: 'get_execution_status', arguments: { instance_id: runId } } }) }).then((r) => r.json());
    const st = JSON.parse(res.result?.content?.[0]?.text ?? '{}');
    if (/completed/i.test(st.status)) return st.output?.content ?? JSON.stringify(st.output);
    if (/failed/i.test(st.status)) throw new Error(st.error ?? 'run failed');
  }
}`;
  return `import json, re, time, requests

A3T = "${base}"

def ask(token: str, message: str, conversation_id: str) -> str:
    headers = {"Authorization": f"Bearer {token}"}
    body = ${bodyLiteral(body, 'python', '    ')}
    start = requests.post("${url}", json=body, headers=headers).json()
    run_id = re.search(r"\\(([0-9a-f-]+)\\)", start["targets"][0]).group(1)
    while True:
        time.sleep(1)
        res = requests.post(f"{A3T}/message", headers=headers, json={
            "jsonrpc": "2.0", "id": 1, "method": "tools/call",
            "params": {"name": "get_execution_status", "arguments": {"instance_id": run_id}}}).json()
        st = json.loads(res["result"]["content"][0]["text"])
        if re.match("completed", st["status"], re.I):
            return st["output"].get("content", st["output"])
        if re.match("failed", st["status"], re.I):
            raise RuntimeError(st.get("error", "run failed"))`;
}
