// How another app calls an agent. Its data plane is POST /chat: the message and
// a session id that starts with the agent's id; the answer streams back as server-
// sent events (the AI SDK UI message stream: text-delta events, then [DONE]). The
// server reads the agent's own record (model, instructions, tools, memory) from
// that id, so the caller sends nothing else. An agent with a webhook can also be
// started there: the call starts a run, polled until it completes.
import { API_BASE } from '$lib/config';

/// The API's origin as an app outside A3T would use it.
export function apiBase(): string {
  const b = API_BASE || (typeof location !== 'undefined' ? location.origin : '');
  return b.startsWith('/') && typeof location !== 'undefined' ? `${location.origin}${b}` : b.replace(/\/$/, '');
}
export const endpointUrl = (ep: string) => `${apiBase()}/webhook${ep.startsWith('/') ? '' : '/'}${ep}`;

export async function copy(text: string): Promise<boolean> {
  try { await navigator.clipboard.writeText(text); return true; } catch { return false; }
}

export type Lang = 'curl' | 'ts' | 'python';

/// Code that talks to the agent's data plane, streaming its answer.
export function chatSnippet(lang: Lang, agentId: string): string {
  const url = `${apiBase()}/chat`;
  if (lang === 'curl') return `curl -N -X POST "${url}" \\
  -H "Authorization: Bearer $A3T_TOKEN" \\
  -H "Content-Type: application/json" \\
  -H "Accept: text/event-stream" \\
  -d '{"session_id":"${agentId}:conv-1","message":"Hello"}'
# data: {"type":"text-delta","id":"msg_…","delta":"Hello! …"}
# …
# data: [DONE]`;
  if (lang === 'ts') return `// Streams the agent's answer; onText gets each piece as it is written.
export async function ask(token: string, message: string, conversationId: string, onText: (t: string) => void): Promise<string> {
  const res = await fetch('${url}', {
    method: 'POST',
    headers: { Authorization: \`Bearer \${token}\`, 'Content-Type': 'application/json', Accept: 'text/event-stream' },
    body: JSON.stringify({ session_id: \`${agentId}:\${conversationId}\`, message }),
  });
  if (!res.ok || !res.body) throw new Error(\`\${res.status} \${await res.text()}\`);
  const reader = res.body.pipeThrough(new TextDecoderStream()).getReader();
  let buf = '', answer = '';
  for (;;) {
    const { value, done } = await reader.read();
    if (done) return answer;
    buf += value;
    const lines = buf.split('\\n'); buf = lines.pop() ?? '';
    for (const line of lines) {
      if (!line.startsWith('data: ') || line === 'data: [DONE]') continue;
      const ev = JSON.parse(line.slice(6));
      if (ev.type === 'text-delta') { answer += ev.delta; onText(ev.delta); }
      if (ev.type === 'error') throw new Error(ev.errorText ?? 'the agent failed');
    }
  }
}`;
  return `import json, requests

def ask(token: str, message: str, conversation_id: str) -> str:
    """Streams the agent's answer, printing each piece as it is written."""
    res = requests.post("${url}", stream=True,
        headers={"Authorization": f"Bearer {token}", "Accept": "text/event-stream"},
        json={"session_id": f"${agentId}:{conversation_id}", "message": message})
    res.raise_for_status()
    answer = ""
    for line in res.iter_lines(decode_unicode=True):
        if not line or not line.startswith("data: ") or line == "data: [DONE]":
            continue
        ev = json.loads(line[6:])
        if ev["type"] == "text-delta":
            answer += ev["delta"]
            print(ev["delta"], end="", flush=True)
        elif ev["type"] == "error":
            raise RuntimeError(ev.get("errorText", "the agent failed"))
    return answer`;
}

/// A webhook call: it starts a run of the agent's flow, polled until it completes.
export function webhookSnippet(url: string, agentId: string): string {
  const base = apiBase();
  return `# 1. Send a message: the answer is the run it started
curl -s -X POST "${url}" \\
  -H "Authorization: Bearer $A3T_TOKEN" \\
  -H "Content-Type: application/json" \\
  -d '{"session_id":"${agentId}:conv-1","message":"Hello"}'
# → {"targets":["<flow> (<run-id>)"]}

# 2. Poll the run until status is "completed"; output.content is the answer
curl -s -X POST "${base}/message" \\
  -H "Authorization: Bearer $A3T_TOKEN" \\
  -H "Content-Type: application/json" \\
  -d '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"get_execution_status","arguments":{"instance_id":"<run-id>"}}}'`;
}
