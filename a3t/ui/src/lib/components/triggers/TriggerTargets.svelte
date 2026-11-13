<script lang="ts">
  /// Where a trigger's firings go, and what each target gets: a flow's input, an
  /// agent's message and conversation — jq over the event. Empty means the
  /// default (a flow gets the body as it came; an agent the trigger's message, in
  /// the conversation trigger-<name>).
  import './triggers.css';
  import { BODY_OF, type SourceKind, type StoredTarget } from './triggerSpec';

  type Opt = { value: string; label: string };
  let { targets = $bindable(), flows, agents, source, trigger }: {
    targets: StoredTarget[];
    /// "<project>/<flow>"
    flows: Opt[];
    /// agent id → its name
    agents: Opt[];
    source: SourceKind;
    trigger: string;
  } = $props();

  function add(kind: 'workflow' | 'a3t_agent' | 'webhook') {
    const t: StoredTarget = kind === 'workflow' ? { type: 'workflow', namespace: '', name: '', input_mapping: '' }
      : kind === 'a3t_agent' ? { type: 'a3t_agent', agent_id: '', message: '', conversation: '' }
      : { type: 'webhook', url: '' };
    targets = [...targets, t];
  }
  function remove(i: number) { targets = targets.filter((_, j) => j !== i); }
  function setFlow(i: number, ref: string) {
    const [ns, ...rest] = ref.split('/');
    targets[i] = { ...targets[i], namespace: ns, name: rest.join('/') };
  }
</script>

<div class="tt trg-ui">
  <div class="event">
    <b>The event</b> — what every expression below reads:
    <code>{'{'} body, message, source, trigger, time {'}'}</code>
    <span>· <span class="mono">.body</span> is {BODY_OF[source]}.</span>
  </div>

  {#each targets as t, i (i)}
    <div class="card tgt">
      <div class="head">
        <span class="kind {t.type}">{t.type === 'workflow' ? 'Flow' : t.type === 'a3t_agent' ? 'Agent' : t.type === 'webhook' ? 'URL' : 'Inbox'}</span>
        {#if t.type === 'workflow'}
          <select value={t.namespace && t.name ? `${t.namespace}/${t.name}` : ''} onchange={(e) => setFlow(i, e.currentTarget.value)}>
            <option value="">— pick a flow —</option>
            {#if t.namespace && t.name && !flows.some((f) => f.value === `${t.namespace}/${t.name}`)}<option value="{t.namespace}/{t.name}">{t.namespace}/{t.name}</option>{/if}
            {#each flows as f (f.value)}<option value={f.value}>{f.label}</option>{/each}
          </select>
        {:else if t.type === 'a3t_agent'}
          <select bind:value={targets[i].agent_id}>
            <option value="">— pick an agent —</option>
            {#if t.agent_id && !agents.some((a) => a.value === t.agent_id)}<option value={t.agent_id}>{t.agent_id}</option>{/if}
            {#each agents as a (a.value)}<option value={a.value}>{a.label}</option>{/each}
          </select>
        {:else if t.type === 'webhook'}
          <input type="url" bind:value={targets[i].url} placeholder="https://…" />
        {:else}
          <span class="mono">{t.agent_id}</span>
        {/if}
        <button class="link rm" onclick={() => remove(i)}>Remove</button>
      </div>

      {#if t.type === 'workflow'}
        <label>Its input <small>jq over the event — empty: the body as it came</small>
          <textarea rows="2" class="mono" bind:value={targets[i].input_mapping} placeholder={'${ {order: .body.order_id, amount: .body.amount} }'}></textarea></label>
      {:else if t.type === 'a3t_agent'}
        <div class="two">
          <label>Message <small>jq — empty: the trigger's message</small>
            <textarea rows="2" class="mono" bind:value={targets[i].message} placeholder={'${ "New order " + (.body.id | tostring) }'}></textarea></label>
          <label>Conversation <small>jq — empty: trigger-{trigger || '<name>'}</small>
            <textarea rows="2" class="mono" bind:value={targets[i].conversation} placeholder={'${ .body.customer_id }'}></textarea></label>
        </div>
        <span class="dim">It runs a turn of the agent, in conversation <span class="mono">{t.agent_id || '<agent>'}:&lt;conversation&gt;</span>; the same conversation keeps its context.</span>
      {:else if t.type === 'webhook'}
        <span class="dim">It is POSTed <span class="mono">{'{'} trigger_id, trigger_name, source, content, context {'}'}</span>.</span>
      {/if}
    </div>
  {:else}
    <p class="dim">No target yet: add the flow or agent each firing goes to.</p>
  {/each}

  <div class="adds">
    <button class="btn2" onclick={() => add('workflow')}>+ Flow</button>
    <button class="btn2" onclick={() => add('a3t_agent')}>+ Agent</button>
    <button class="btn2" onclick={() => add('webhook')}>+ URL</button>
  </div>
</div>

<style>
  .tt { display: flex; flex-direction: column; gap: 10px; }
  .event { padding: 10px 12px; border-radius: 8px; background: var(--bg-subtle); font-size: 12.5px; color: var(--text-2); line-height: 1.6; }
  .event code { font: 12px var(--font-mono); color: var(--text-1); }
  .tgt { gap: 8px; padding: 12px 14px; }
  .head { display: flex; align-items: center; gap: 10px; }
  .head select, .head input { flex: 1 1 auto; min-width: 0; }
  .kind { font: 600 10.5px var(--font-mono); text-transform: uppercase; letter-spacing: 0.05em; padding: 2px 7px; border-radius: 4px; background: var(--bg-subtle); color: var(--text-2); }
  .kind.workflow { background: var(--runtime-bg); color: var(--runtime); }
  .kind.a3t_agent { background: var(--spec-bg); color: var(--spec); }
  .rm { white-space: nowrap; border: 0; background: none; padding: 0 4px; color: var(--text-3); font: 500 12.5px var(--font-sans); cursor: pointer; }
  .rm:hover { color: var(--err, #b42318); }
  label { display: flex; flex-direction: column; gap: 4px; font-size: 12.5px; color: var(--text-2); text-transform: none; letter-spacing: 0; font-weight: 500; }
  small { font-weight: 400; color: var(--text-4); }
  textarea { width: 100%; font: 12.5px/1.5 var(--font-mono); resize: vertical; }
  .two { display: grid; grid-template-columns: 1fr 1fr; gap: 10px; }
  @container details (max-width: 560px) { .two { grid-template-columns: 1fr; } }
  .adds { display: flex; gap: 8px; flex-wrap: wrap; }
  .dim { font-size: 12px; color: var(--text-3); }
  .mono { font-family: var(--font-mono); }
</style>
