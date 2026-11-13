<script lang="ts">
  /// The agent's two addresses: the control plane (its config, runtime, lineage,
  /// sharing; for a backend, with a secret key) and the data plane (chat and SSE;
  /// for an app). Shown at the top of Overview, with a call that works today.
  import { addresses, chatCurl } from './controlPlane';
  import { copy } from './agentApi';

  let { id, onkeys }: { id: string; onkeys?: () => void } = $props();
  const a = $derived(addresses(id));
  const curl = $derived(chatCurl(id));
  let copied = $state('');
  async function cp(k: string, s: string) { if (await copy(s)) { copied = k; setTimeout(() => (copied = ''), 1500); } }
</script>

<div class="card">
  <div class="card-head"><h3>Endpoints</h3><span>every agent has two</span></div>
  <ul class="planes">
    <li>
      <div class="who"><b>Control plane</b><span class="dim">config, runtime, lineage, sharing · for your backend</span></div>
      <code class="url">{a.control}</code>
      <button class="btn2" onclick={() => cp('c', a.control)}>{copied === 'c' ? 'Copied' : 'Copy'}</button>
    </li>
    <li>
      <div class="who"><b>Data plane</b><span class="dim">chat, streamed as it is written · for your app</span></div>
      <code class="url">{a.data} <span class="dim">· session <span class="mono">{a.session}</span></span></code>
      <button class="btn2" onclick={() => cp('d', a.data)}>{copied === 'd' ? 'Copied' : 'Copy'}</button>
    </li>
  </ul>
  <div class="callit">
    <div class="row between"><span class="dim">Call it</span><button class="link" onclick={() => cp('x', curl)}>{copied === 'x' ? 'Copied' : 'Copy'}</button></div>
    <pre>{curl}</pre>
  </div>
  <p class="dim">Every call needs a token: <span class="mono">$A3T_AGENT_KEY</span> is one of its keys — make and revoke them on its <button class="link" onclick={() => onkeys?.()}>Keys</button> tab.</p>
  <p class="dim">Own addresses per agent (<span class="mono">{id}.run.a3t.io</span>) come with the gateway; these paths keep working.</p>
</div>

<style>
  .planes { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; }
  .planes li { display: grid; grid-template-columns: 210px minmax(0, 1fr) auto; gap: 12px; align-items: center; padding: 9px 0; border-top: 1px solid var(--divider); }
  .who { display: flex; flex-direction: column; gap: 2px; }
  .who b { font-weight: 600; color: var(--text-1); font-size: 13px; }
  .url { font-family: var(--font-mono); font-size: 12px; padding: 7px 10px; background: var(--bg-subtle); border-radius: 6px; overflow-x: auto; white-space: nowrap; color: var(--text-1); }
  .callit { border-radius: 8px; background: #1d1c1a; padding: 8px 12px 10px; }
  .callit .dim, .callit .link { color: #c8c3b6; }
  .callit pre { margin: 6px 0 0; font: 12px/1.65 var(--font-mono); color: #eceae4; white-space: pre; overflow-x: auto; }
</style>
