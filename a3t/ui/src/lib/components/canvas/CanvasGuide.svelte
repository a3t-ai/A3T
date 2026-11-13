<script lang="ts">
  /// What a canvas is, in four parts and one YAML, and where to start: on the
  /// Canvas page and in Details when no canvas is picked.
  import { navigateTo, openInStudio } from '$lib/stores/panels.svelte.ts';
  import '$lib/components/triggers/triggers.css';
  import { getProject } from '$lib/stores/context.svelte.ts';

  let { oncreate }: { oncreate?: () => void } = $props();
  const create = () => (oncreate ? oncreate() : navigateTo('canvas', 'Canvas', { create: true }));

  const SAMPLE = `flowRef: default/flow_default_refund
description: refunds, kept warm for the shop
placement:
  mode: shared            # or isolated: a sandbox of its own
  runtimeSelector: { class: cpu }
lifecycle:
  state: running
  session: persistent     # checkpoint and resume between runs
  scaling: { min: 1, max: 3 }
  restart: { policy: on-failure, backoff: exponential, maxRetries: 5 }
health:
  dependsOn:
    - resource: a3t/claude-cli-max
  onDependencyDown: queue`;
</script>

<section class="cg trg-ui">
  <div class="steps">
    <div><b>1 · Flow</b><span>The one flow it deploys. Each deployed flow has a canvas.</span></div>
    <div><b>2 · Placement</b><span>Where it loads: a <span class="mono">shared</span> runtime pool, or an <span class="mono">isolated</span> sandbox — picked by runtime labels or pinned.</span></div>
    <div><b>3 · Lifecycle</b><span>How it runs: running or paused, <span class="mono">ephemeral</span> or <span class="mono">persistent</span> sessions, scaling, restarts.</span></div>
    <div><b>4 · Dependencies</b><span>What it needs — models, MCP servers, databases — and what happens to its runs when one is down.</span></div>
  </div>
  <div class="code">
    <div class="bar"><span>refunds-canvas.yaml</span><span class="grow"></span><button onclick={() => navigator.clipboard?.writeText(SAMPLE)}>Copy</button></div>
    <pre>{SAMPLE}</pre>
  </div>
  <div class="acts"><button class="btn2 primary" onclick={create}>New canvas</button>
    <button class="btn2" onclick={() => openInStudio('canvas', getProject() || 'default', '', undefined)}>Write one in Studio</button>
    <span class="dim">A canvas is a file next to its flow: this page shows where each one lands, deploys it and watches it run.</span></div>
</section>

<style>
  .cg { container: cg / inline-size; display: flex; flex-direction: column; gap: 12px; }
  .steps { display: grid; grid-template-columns: repeat(4, 1fr); gap: 10px; }
  .steps > div { display: flex; flex-direction: column; gap: 4px; padding: 12px 14px; border: 1px solid var(--border); border-radius: 10px; background: var(--bg-surface); font-size: 12.5px; color: var(--text-2); line-height: 1.5; }
  .steps b { font: 400 16px var(--font-serif); color: var(--text-1); }
  @container cg (max-width: 860px) { .steps { grid-template-columns: 1fr 1fr; } }
  @container cg (max-width: 520px) { .steps { grid-template-columns: 1fr; } }
  .code { border-radius: 10px; background: #111214; overflow: hidden; }
  .bar { display: flex; align-items: center; padding: 6px 12px; border-bottom: 1px solid #26282c; font: 12px var(--font-mono); color: #9a9da3; }
  .grow { flex: 1 1 auto; }
  .bar button { border: 1px solid #3a3d42; background: none; color: #c7c9cd; border-radius: 6px; padding: 2px 10px; font: 500 12px var(--font-sans); cursor: pointer; }
  pre { margin: 0; padding: 14px 16px; color: #f4f4f5; font: 12.5px/1.6 var(--font-mono); white-space: pre-wrap; overflow-wrap: anywhere; }
  .acts { display: flex; align-items: center; gap: 10px; flex-wrap: wrap; }
  .dim { font-size: 12.5px; color: var(--text-3); }
  .mono { font-family: var(--font-mono); }
</style>
