<script lang="ts">
  /// Try it here: a conversation with the agent on its data plane (POST /chat,
  /// streamed), signed in as you — the same call an app makes with a key.
  import { API_BASE } from '$lib/config';
  import { getAccessToken } from '$lib/services/api';

  let { agentId, onanswered }: { agentId: string; onanswered?: () => void } = $props();

  type Msg = { role: 'you' | 'agent'; text: string; err?: boolean };
  let msgs = $state<Msg[]>([]);
  let input = $state('');
  let busy = $state(false);
  let conv = $state(newConv());
  function newConv() { return `try-${Date.now().toString(36)}`; }
  function restart() { msgs = []; conv = newConv(); }

  async function send() {
    const message = input.trim();
    if (!message || busy) return;
    input = '';
    msgs = [...msgs, { role: 'you', text: message }, { role: 'agent', text: '' }];
    const at = msgs.length - 1;
    busy = true;
    try {
      const res = await fetch(`${API_BASE}/chat`, {
        method: 'POST',
        headers: { Authorization: `Bearer ${getAccessToken() ?? ''}`, 'Content-Type': 'application/json', Accept: 'text/event-stream' },
        body: JSON.stringify({ session_id: `${agentId}:${conv}`, message }),
      });
      if (!res.ok || !res.body) {
        let why = `${res.status}`;
        try { why = (await res.json()).error ?? why; } catch { /* not JSON */ }
        msgs[at] = { role: 'agent', text: why, err: true };
        return;
      }
      const reader = res.body.pipeThrough(new TextDecoderStream()).getReader();
      let buf = '';
      for (;;) {
        const { value, done } = await reader.read();
        if (done) break;
        buf += value;
        const lines = buf.split('\n'); buf = lines.pop() ?? '';
        for (const line of lines) {
          if (!line.startsWith('data: ') || line === 'data: [DONE]') continue;
          let ev: any; try { ev = JSON.parse(line.slice(6)); } catch { continue; }
          if (ev.type === 'text-delta') msgs[at] = { role: 'agent', text: msgs[at].text + ev.delta };
          if (ev.type === 'error') msgs[at] = { role: 'agent', text: ev.errorText ?? 'It failed.', err: true };
        }
      }
      if (!msgs[at].text) msgs[at] = { role: 'agent', text: 'No answer.', err: true };
    } catch (e) {
      msgs[at] = { role: 'agent', text: String((e as Error).message ?? e), err: true };
    } finally {
      busy = false;
      onanswered?.();
    }
  }
</script>

<div class="try">
  {#if msgs.length}
    <ol class="msgs">
      {#each msgs as m, i (i)}
        <li class={m.role} class:err={m.err}><b>{m.role === 'you' ? 'You' : 'Agent'}</b><span>{m.text || '…'}</span></li>
      {/each}
    </ol>
  {/if}
  <form class="row" onsubmit={(e) => { e.preventDefault(); send(); }}>
    <input type="text" class="grow" bind:value={input} placeholder="Say something to it" disabled={busy} />
    <button class="btn2 primary" type="submit" disabled={busy || !input.trim()}>{busy ? 'Answering…' : 'Send'}</button>
    {#if msgs.length}<button class="link" type="button" onclick={restart} disabled={busy}>New conversation</button>{/if}
  </form>
  <p class="dim">As you, on <span class="mono">POST /chat</span> with session <span class="mono">{agentId}:{conv}</span> — what an app sends with a key.</p>
</div>

<style>
  .try { display: flex; flex-direction: column; gap: 8px; }
  .msgs { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 6px; max-height: 280px; overflow-y: auto; }
  .msgs li { display: grid; grid-template-columns: 52px 1fr; gap: 8px; padding: 7px 10px; border-radius: 8px; font-size: 13px; white-space: pre-wrap; }
  .msgs li.you { background: var(--bg-subtle); }
  .msgs li.agent { background: var(--bg-surface); border: 1px solid var(--border); }
  .msgs li.err { color: var(--err); }
  .msgs b { font-size: 11.5px; color: var(--text-3); font-weight: 600; }
  .grow { flex: 1 1 auto; }
  .mono { font-family: var(--font-mono); font-size: 0.92em; }
</style>
