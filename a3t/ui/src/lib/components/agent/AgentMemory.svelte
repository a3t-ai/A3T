<script lang="ts">
  /// Memory — what it remembers, by type, from its memory space on the org's ADB
  /// (read through agent_a3t_agents's tool_a3t_agent_memory). Its memory setting
  /// (kept across conversations or not) is changed in Model & tools.
  import { insight, type MemoryType } from './controlPlane';
  import type { AgentRec } from './agentData';

  let { agent, ongo }: { agent: AgentRec; ongo: (tab: string) => void } = $props();

  let mem = $state<{ namespace: string; counts: Record<MemoryType, number>; memory: Record<MemoryType, any[]> } | null>(null);
  let err = $state('');
  let open = $state<MemoryType>('episodic');
  async function load(id: string) {
    mem = null; err = '';
    const r = await insight.memory(id);
    if (agent.id !== id) return;
    if (r.ok) mem = r.data; else err = r.error ?? 'Its memory could not be read.';
  }
  $effect(() => { load(agent.id); });

  const MEM: { k: MemoryType; t: string; d: string }[] = [
    { k: 'working', t: 'Working', d: 'The conversation it is in: task, recent turns.' },
    { k: 'episodic', t: 'Episodic', d: 'Past conversations and runs, as events.' },
    { k: 'semantic', t: 'Semantic', d: 'Facts and summaries, searched by meaning.' },
    { k: 'procedural', t: 'Procedural', d: 'Steps and preferences it has learned; what a merge carries back.' },
  ];
  const keeps = $derived(agent.mode === 'persistent');
  const text = (e: any) => {
    const t = e?.content ?? e?.text ?? e?.reply ?? e?.message ?? e?.pattern ?? e?.object ?? e;
    const s = typeof t === 'string' ? t : JSON.stringify(t);
    return s.length > 300 ? s.slice(0, 300) + '…' : s;
  };
</script>

<section class="agt">
  <div class="card">
    <div class="card-head"><h3>Memory</h3><span>{keeps ? 'kept across conversations' : 'only the conversation it is in'}</span><span class="grow"></span>
      <button class="link" onclick={() => load(agent.id)}>Refresh</button>
      <button class="link" onclick={() => ongo('tools')}>Change in Model &amp; tools →</button></div>
    {#if err}<p class="err">{err}</p>{/if}
    <div class="mem">
      {#each MEM as m (m.k)}
        <button class="card" class:sel={open === m.k} class:off={!keeps && m.k !== 'working'} onclick={() => (open = m.k)}>
          <b>{m.t}</b><span class="dim">{m.d}</span>
          <em class="mono">{mem ? `${mem.counts?.[m.k] ?? 0} entries` : '…'}</em>
        </button>
      {/each}
    </div>
    {#if mem}
      <p class="dim">Namespace <span class="mono">{mem.namespace}</span> on the org's ADB. Versions and branches of it are in Releases.</p>
      <ol class="memlist">
        {#each (mem.memory?.[open] ?? []).slice(0, 30) as e, i (i)}<li class="mono">{text(e)}</li>
        {:else}<li class="dim">Nothing in {open} memory yet — talk to it on Overview.</li>{/each}
      </ol>
    {/if}
  </div>
</section>

<style>
  .grow { flex: 1 1 auto; }
  .mem { display: grid; grid-template-columns: repeat(auto-fill, minmax(190px, 1fr)); gap: 10px; }
  .mem .card { gap: 6px; padding: 12px 14px; text-align: left; font: inherit; cursor: pointer; }
  .mem .card.off { opacity: 0.55; }
  .mem .card.sel { border-color: var(--spec); background: var(--signal-bg); }
  .mem em { font-style: normal; font-size: 11px; color: var(--text-4); }
  .memlist { margin: 0; padding: 0 0 0 18px; display: flex; flex-direction: column; gap: 6px; font-size: 12px; color: var(--text-2); }
  .mono { font-family: var(--font-mono); }
</style>
