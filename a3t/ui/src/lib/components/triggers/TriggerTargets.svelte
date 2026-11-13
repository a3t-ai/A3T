<script lang="ts">
  /// Where a trigger's firings go, a card per target with what it gets: a flow's
  /// input, an agent's message and conversation — jq over the event. Empty means
  /// the default (a flow gets the body as it came; an agent the trigger's message,
  /// in the conversation trigger-<name>). On the agent tabs' system (.agt).
  import '$lib/components/agent/agent.css';
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

  const KIND: Record<string, { name: string; does: string }> = {
    workflow: { name: 'Flow', does: 'runs the flow with the input below' },
    a3t_agent: { name: 'Agent', does: 'a turn of the agent, in the conversation below' },
    webhook: { name: 'URL', does: 'posts the event on to another service' },
    agent: { name: 'Inbox', does: "an agent's inbox (older triggers)" },
  };

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

<section class="agt">
  <p class="dim">Each firing goes to every target below. Their fields are jq over the event
    <span class="mono">{'{'} body, message, source, trigger, time {'}'}</span> — <span class="mono">.body</span> is {BODY_OF[source]}.</p>

  {#each targets as t, i (i)}
    <div class="card">
      <div class="card-head"><h3>{i + 1} · {KIND[t.type]?.name ?? t.type}</h3><span>{KIND[t.type]?.does}</span>
        <span class="grow"></span><button class="btn2 quiet rm" onclick={() => remove(i)}>Remove</button></div>

      {#if t.type === 'workflow'}
        <div class="fields">
          <label class="fld full">Flow
            <select value={t.namespace && t.name ? `${t.namespace}/${t.name}` : ''} onchange={(e) => setFlow(i, e.currentTarget.value)}>
              <option value="">— pick a flow —</option>
              {#if t.namespace && t.name && !flows.some((f) => f.value === `${t.namespace}/${t.name}`)}<option value="{t.namespace}/{t.name}">{t.namespace}/{t.name}</option>{/if}
              {#each flows as f (f.value)}<option value={f.value}>{f.label}</option>{/each}
            </select></label>
          <label class="fld full">Its input <small>jq over the event; empty: the body as it came</small>
            <textarea rows="2" bind:value={targets[i].input_mapping} placeholder={'${ {order: .body.order_id, amount: .body.amount} }'}></textarea></label>
        </div>
      {:else if t.type === 'a3t_agent'}
        <div class="fields">
          <label class="fld full">Agent
            <select bind:value={targets[i].agent_id}>
              <option value="">— pick an agent —</option>
              {#if t.agent_id && !agents.some((a) => a.value === t.agent_id)}<option value={t.agent_id}>{t.agent_id}</option>{/if}
              {#each agents as a (a.value)}<option value={a.value}>{a.label}</option>{/each}
            </select></label>
          <label class="fld">Message <small>jq; empty: the trigger's message</small>
            <textarea rows="2" bind:value={targets[i].message} placeholder={'${ "New order " + (.body.id | tostring) }'}></textarea></label>
          <label class="fld">Conversation <small>jq; empty: trigger-{trigger || '<name>'}</small>
            <textarea rows="2" bind:value={targets[i].conversation} placeholder={'${ .body.customer_id }'}></textarea></label>
        </div>
        <p class="dim">The turn runs in conversation <span class="mono">{t.agent_id || '<agent>'}:&lt;conversation&gt;</span>; firings with the same conversation share its context.</p>
      {:else if t.type === 'webhook'}
        <div class="fields">
          <label class="fld full">URL<input type="url" bind:value={targets[i].url} placeholder="https://…" /></label>
        </div>
        <p class="dim">It is posted <span class="mono">{'{'} trigger_id, trigger_name, source, content, context {'}'}</span>.</p>
      {:else}
        <p class="dim">Inbox of <span class="mono">{t.agent_id}</span>. Replace it with an Agent target to map its message and conversation.</p>
      {/if}
    </div>
  {:else}
    <div class="card"><p class="empty">No target yet: add the flow or agent each firing goes to.</p></div>
  {/each}

  <div class="acts"><span class="dim">Add a target</span>
    <button class="btn2" onclick={() => add('workflow')}>+ Flow</button>
    <button class="btn2" onclick={() => add('a3t_agent')}>+ Agent</button>
    <button class="btn2" onclick={() => add('webhook')}>+ URL</button>
  </div>
</section>
