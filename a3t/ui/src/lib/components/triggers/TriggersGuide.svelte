<script lang="ts">
  /// What a trigger is, in three steps and one YAML, and where to start: on the
  /// Triggers page and in Details when no trigger is picked.
  import { navigateTo, openInStudio } from '$lib/stores/panels.svelte.ts';
  import './triggers.css';
  import { getProject } from '$lib/stores/context.svelte.ts';

  /// Opens the editor; by default, the Triggers app with its create step.
  let { oncreate }: { oncreate?: () => void } = $props();
  const create = () => (oncreate ? oncreate() : navigateTo('triggers', 'Triggers', { create: true }));

  const SAMPLE = `name: refunds
project: default
on:
  webhook: { path: /shop/refund, method: POST }
send:
  - flow: default/flow_default_refund
    input: '\${ {order: .body.order_id, amount: .body.amount} }'
  - agent: agent_default_support
    message: '\${ "Refund asked for order " + (.body.order_id | tostring) }'
    conversation: '\${ .body.customer_id }'`;
</script>

<section class="tg trg-ui">
  <div class="steps">
    <div><b>1 · On</b><span>What fires it: <span class="mono">webhook</span> (a POST), <span class="mono">cron</span> (a schedule), <span class="mono">adb_topic</span> (each message on a topic — Kafka, Postgres and Pulsar arrive this way, through ADB), <span class="mono">event</span> (the platform's own).</span></div>
    <div><b>2 · Map</b><span>Each target's input is jq over the event <span class="mono">{'{'} body, message, source, trigger, time {'}'}</span> — e.g. the POST body's fields into the flow's input.</span></div>
    <div><b>3 · Send</b><span>To flows, to agents (a turn in a conversation you choose), or on to a URL. One firing can fan out to several.</span></div>
  </div>
  <div class="code">
    <div class="bar"><span>refunds.yaml</span><span class="grow"></span><button onclick={() => navigator.clipboard?.writeText(SAMPLE)}>Copy</button></div>
    <pre>{SAMPLE}</pre>
  </div>
  <div class="acts"><button class="btn2 primary" onclick={create}>New trigger</button>
    <button class="btn2" onclick={() => openInStudio('trigger', getProject() || 'default', '', undefined)}>Write one in Studio</button>
    <span class="dim">Developers keep triggers next to their flows as code; this page shows them, tests them and switches them on and off.</span></div>
</section>

<style>
  .tg { container: tg / inline-size; display: flex; flex-direction: column; gap: 12px; }
  .steps { display: grid; grid-template-columns: repeat(3, 1fr); gap: 10px; }
  .steps > div { display: flex; flex-direction: column; gap: 4px; padding: 12px 14px; border: 1px solid var(--border); border-radius: 10px; background: var(--bg-surface); font-size: 12.5px; color: var(--text-2); line-height: 1.5; }
  .steps b { font: 400 16px var(--font-serif); color: var(--text-1); }
  @container tg (max-width: 720px) { .steps { grid-template-columns: 1fr; } }
  .code { border-radius: 10px; background: #111214; overflow: hidden; }
  .bar { display: flex; align-items: center; padding: 6px 12px; border-bottom: 1px solid #26282c; font: 12px var(--font-mono); color: #9a9da3; }
  .grow { flex: 1 1 auto; }
  .bar button { border: 1px solid #3a3d42; background: none; color: #c7c9cd; border-radius: 6px; padding: 2px 10px; font: 500 12px var(--font-sans); cursor: pointer; }
  pre { margin: 0; padding: 14px 16px; color: #f4f4f5; font: 12.5px/1.6 var(--font-mono); white-space: pre-wrap; overflow-wrap: anywhere; }
  .acts { display: flex; align-items: center; gap: 10px; flex-wrap: wrap; }
  .dim { font-size: 12.5px; color: var(--text-3); }
  .mono { font-family: var(--font-mono); }
</style>
