<script lang="ts">
  /// Where a trigger's firings go, a card per target with what it gets: a flow's
  /// input, an agent's message and conversation — jq over the event. Empty means
  /// the default (a flow gets the body as it came; an agent the trigger's message,
  /// in the conversation trigger-<name>). On the agent tabs' system (.agt).
  import '$lib/components/agent/agent.css';
  import './triggers.css';
  import { BODY_OF, type SourceKind, type StoredTarget } from './triggerSpec';
  import TriggerMapping, { type MapCtx } from './TriggerMapping.svelte';

  type Opt = { value: string; label: string };
  let { targets = $bindable(), samples = $bindable(), flows, agents, source, trigger, ctx }: {
    targets: StoredTarget[];
    /// "<project>/<flow>"
    flows: Opt[];
    /// agent id → its name
    agents: Opt[];
    source: SourceKind;
    trigger: string;
    /// Sample bodies (JSON text) the mappings are checked on.
    samples: string[];
    ctx: MapCtx;
  } = $props();
  const agentName = (id?: string) => agents.find((a) => a.value === id)?.label ?? id ?? '';

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
  <p class="dim">Each firing goes to every target below. Their mappings are jq over the event
    <span class="mono">{'{'} body, message, source, trigger, time {'}'}</span> — <span class="mono">.body</span> is {BODY_OF[source]}.
    Write them, or ask <b>agent_a3t_mapper</b> to: either way they are checked on the samples and approved before they are used. When the trigger fires, only the approved jq runs.</p>

  <div class="card">
    <div class="card-head"><h3>Sample events</h3><span>what the mappings are checked on, and what the mapper writes from — {source === 'webhook' ? 'bodies a caller would POST' : '.body as the source makes it'}</span></div>
    {#each samples as _, k (k)}
      <div class="smp">
        <textarea rows="3" bind:value={samples[k]} spellcheck="false" placeholder={'{"order_id": "A-17", "amount": 4.5}'}></textarea>
        <button class="btn2 quiet" onclick={() => (samples = samples.filter((_, j) => j !== k))} aria-label="Remove sample {k + 1}">Remove</button>
      </div>
    {/each}
    <div class="acts"><button class="btn2" onclick={() => (samples = [...samples, ''])}>+ Sample</button>
      <span class="dim">Use real shapes, with made-up values: samples go to the mapper's model.</span></div>
  </div>

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
        </div>
        <TriggerMapping bind:value={targets[i].input_mapping} field="input" label="Its input" hint="jq over the event; empty: the body as it came"
          placeholder={'${ {order: .body.order_id, amount: .body.amount} }'} target="flow:{t.namespace}/{t.name}" flowRef={t.namespace && t.name ? `${t.namespace}/${t.name}` : undefined}
          targetName="{t.namespace}/{t.name}" {ctx} />
      {:else if t.type === 'a3t_agent'}
        <div class="fields">
          <label class="fld full">Agent
            <select bind:value={targets[i].agent_id}>
              <option value="">— pick an agent —</option>
              {#if t.agent_id && !agents.some((a) => a.value === t.agent_id)}<option value={t.agent_id}>{t.agent_id}</option>{/if}
              {#each agents as a (a.value)}<option value={a.value}>{a.label}</option>{/each}
            </select></label>
        </div>
        <TriggerMapping bind:value={targets[i].message} field="message" label="Message" hint="jq; empty: the trigger's message"
          placeholder={'${ "New order " + (.body.id | tostring) }'} target="agent:{t.agent_id}" targetName={agentName(t.agent_id)} {ctx} />
        <TriggerMapping bind:value={targets[i].conversation} field="conversation" label="Conversation" hint="jq; empty: trigger-{trigger || '<name>'}"
          placeholder={'${ .body.customer_id }'} target="agent:{t.agent_id}" targetName={agentName(t.agent_id)} {ctx} />
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

<style>
  .smp { display: grid; grid-template-columns: 1fr auto; gap: 8px; align-items: start; }
  .smp textarea { width: 100%; box-sizing: border-box; padding: 8px 11px; border: 1px solid var(--border); border-radius: 8px; background: var(--bg-surface); color: var(--text-1); font: 12.5px/1.5 var(--font-mono); resize: vertical; outline: none; }
  .smp textarea:focus { border-color: var(--spec); }
</style>
