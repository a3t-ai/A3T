<script lang="ts">
  /// What Details shows for a list app with nothing picked: the app's banner and
  /// where to start.
  import PageBanner from '$lib/components/shared/PageBanner.svelte';
  import TriggersGuide from '$lib/components/triggers/TriggersGuide.svelte';
  import CanvasGuide from '$lib/components/canvas/CanvasGuide.svelte';
  import { contextLabel } from '$lib/stores/apps.svelte.ts';

  type Accent = 'spec' | 'runtime' | 'control' | 'ephemeral';
  let { appType, title, onitems }: { appType: string; title: string; onitems?: () => void } = $props();

  /// The banner each app's own page shows, kept short here.
  const BANNER: Record<string, { headline: string; highlight: string; subtitle: string; description: string; accent: Accent }> = {
    projects: { headline: 'Organise work into projects.', highlight: 'projects', accent: 'spec',
      subtitle: 'Yours, or your organisation\'s — each one a namespace with its members and rules.',
      description: 'A project you create lives under your handle; an organisation\'s under its name. Make it private to keep it to yourself or to its members.' },
    templates: { headline: 'Start with a template.', highlight: 'template', accent: 'spec',
      subtitle: 'A template is code: a flow that runs on the core agent, with its hooks and tools.',
      description: 'Every agent is made from a template. The template says how a turn runs; the agent fills in its own values — model, instructions, tools, memory.' },
    agents: { headline: 'Agents over an API.', highlight: 'agent', accent: 'runtime',
      subtitle: 'Made from a template, or cloned, forked or branched from another agent.',
      description: 'Each agent has a data plane (chat, streamed) for the apps that call it and a control plane for its config, runtime and memory — its own figures, history, versions and branches.' },
    canvas: { headline: 'Deploy a flow with a canvas.', highlight: 'canvas', accent: 'control',
      subtitle: 'A canvas is one flow\'s deployment file: where it loads, how it runs, what it depends on.',
      description: 'It is YAML next to its flow. Here you see which runtime takes it, deploy it, and watch it run.' },
    triggers: { headline: 'Start flows and agents from events.', highlight: 'events', accent: 'runtime',
      subtitle: 'A webhook call, a schedule, a message on an ADB topic or a platform event — each firing goes to the flows and agents you name, with the input you map.',
      description: 'A trigger is YAML: what fires it, and where each firing goes. Write it here or in the Studio; pause or rewire it without changing the flow.' },
    memory: { headline: 'What each agent remembers.', highlight: 'remembers', accent: 'spec',
      subtitle: 'Working, episodic, semantic and procedural memory, kept in ADB — and the spaces each agent reads.',
      description: 'Pick an agent in Items: see each kind of its memory, filter it, or read it with AQL. Changing what it remembers stays on the agent.' },
    resources: { headline: 'Connect your resources.', highlight: 'resources', accent: 'ephemeral',
      subtitle: 'Databases, LLMs, MCP servers, memory — everything agents need.',
      description: 'Resources are named connections to external systems. Register them once, reference them by name in any workflow or agent.' },
    security: { headline: 'Decide who gets in, and what they may do.', highlight: 'who', accent: 'spec',
      subtitle: 'Identity from your SSO provider; Cedar policies over every action.',
      description: 'Realm roles map to A3T roles at sign-in, and those roles are what the policies decide on. Every allow and deny is logged.' },
    runtime: { headline: 'Operate what is running, where it runs.', highlight: 'running', accent: 'runtime',
      subtitle: 'Hosts → runtimes → the agents and flows deployed on them.',
      description: 'Pick a runtime or a deployment. Pause, resume, restart, undeploy or probe it; change how it deploys in its canvas.' },
    executions: { headline: 'Every run, step by step.', highlight: 'run', accent: 'control',
      subtitle: 'What ran, when, for whom, and how long each step took.',
      description: 'Pick a run to see its steps, timeline, journal and input and output. Re-run it, or re-run it in debug to step through.' },
    settings: { headline: 'Configure your platform.', highlight: 'platform', accent: 'spec',
      subtitle: 'Identity, security, infrastructure and billing — all in one place.',
      description: 'Community Edition runs open: anyone who signs in can edit. Enterprise adds roles, Cedar policy, SSO and audit, marked EE.' },
  };
  const b = $derived(BANNER[appType] ?? { headline: title, highlight: '', subtitle: '', description: '', accent: 'spec' as Accent });
  const what = $derived(contextLabel(appType).toLowerCase());

</script>

<div class="aw">
  <PageBanner headline={b.headline} highlightWord={b.highlight} subtitle={b.subtitle} description={b.description} accent={b.accent} />
  <!-- An app with a concept to learn before its first item shows it here. -->
  {#if appType === 'triggers'}<TriggersGuide />{:else if appType === 'canvas'}<CanvasGuide />{/if}

  <div class="aw-start">
    <span>Pick from {what} in Items to open it here, or ask in Chat.</span>
    <span class="sp"></span>
    {#if onitems}<button class="btn" onclick={onitems}>Show Items</button>{/if}
  </div>

</div>

<style>
  .aw { height: 100%; overflow-y: auto; padding: 20px 24px 32px; box-sizing: border-box; display: flex; flex-direction: column; gap: 18px; }
  .aw-start { display: flex; align-items: center; gap: 8px; flex-wrap: wrap; padding: 10px 14px; border: 1px solid var(--border); border-radius: 12px; background: var(--bg-surface); font-size: 13px; color: var(--text-2); }
  .sp { flex: 1 1 auto; }
  .btn { height: 30px; padding: 0 12px; border-radius: 8px; border: 1px solid var(--border); background: var(--bg-surface); font: 500 13px var(--font-sans); color: var(--spec); cursor: pointer; white-space: nowrap; }
  .btn:hover { border-color: var(--spec); }
</style>
