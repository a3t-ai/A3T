<script lang="ts">
  /// The agent's own figures, from the core's turn log (a3t_agent_turns), read
  /// through agent_a3t_agents's tool_a3t_agent_turns.
  import { insight, type TurnSummary, type Turn } from './controlPlane';
  import { ago } from './agentData';

  let { id }: { id: string } = $props();
  let summary = $state<TurnSummary | null>(null);
  let turns = $state<Turn[]>([]);
  let error = $state('');

  async function load(agentId: string) {
    summary = null; error = '';
    const r = await insight.turns(agentId);
    if (r.ok) { summary = r.data.summary; turns = r.data.turns ?? []; } else error = r.error ?? 'Figures could not be read.';
  }
  $effect(() => { load(id); });
</script>

<div class="card">
  <div class="card-head"><h3>Metrics</h3><span>its own turns, from the turn log</span><span class="grow"></span><button class="link" onclick={() => load(id)}>Refresh</button></div>
  {#if error}
    <p class="err">{error}</p>
  {:else if !summary}
    <p class="dim">Reading…</p>
  {:else}
    <div class="tiles">
      <div class="tile"><span class="v">{summary.turns}</span><span class="k">Turns</span></div>
      <div class="tile"><span class="v" class:bad={summary.failed > 0}>{summary.failed}</span><span class="k">Failed</span></div>
      <div class="tile"><span class="v">{summary.tool_calls}</span><span class="k">Tool calls</span></div>
      <div class="tile"><span class="v">{summary.avg_ms ? `${(summary.avg_ms / 1000).toFixed(1)} s` : '—'}</span><span class="k">Average turn</span></div>
    </div>
    {#if turns.length}
      <table class="turns">
        <thead><tr><th scope="col">When</th><th scope="col">Session</th><th scope="col">Outcome</th><th scope="col">Tools</th><th scope="col">Time</th></tr></thead>
        <tbody>
          {#each turns.slice(0, 8) as t (t.at + t.session_id)}
            <tr><td>{ago(t.at)}</td><td class="mono">{t.session_id.split(':').slice(1).join(':') || t.session_id}</td><td>{t.outcome === 'null' || !t.outcome ? 'answered' : t.outcome}{t.degraded ? ' · degraded' : ''}</td><td>{t.tool_calls}</td><td>{(Number(t.duration_ms) / 1000).toFixed(1)} s</td></tr>
          {/each}
        </tbody>
      </table>
    {:else}
      <p class="dim">No turns yet. Call it on its data plane and they show here.</p>
    {/if}
  {/if}
</div>

<style>
  .tiles { display: grid; grid-template-columns: repeat(4, minmax(0, 1fr)); gap: 10px; }
  .tile { display: flex; flex-direction: column; gap: 4px; padding: 10px 12px; border: 1px solid var(--border); border-radius: 8px; background: var(--bg-ivory); }
  .tile .v { font-size: 20px; font-weight: 500; color: var(--text-1); }
  .tile .v.bad { color: var(--err); }
  .tile .k { font-size: 12px; color: var(--text-3); }
  .turns { width: 100%; border-collapse: collapse; font-size: 12.5px; }
  .turns th, .turns td { padding: 6px 4px; border-top: 1px solid var(--divider); text-align: left; font-weight: 400; }
  .turns thead th { border-top: 0; color: var(--text-3); font-weight: 500; }
</style>
