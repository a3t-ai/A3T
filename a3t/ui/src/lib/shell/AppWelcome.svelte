<script lang="ts">
  /// What Details shows for a list app with nothing picked: the app's banner,
  /// where to start, then learning material — docs to read and videos to watch.
  /// The docs and videos are placeholders until the real ones are written.
  import PageBanner from '$lib/components/shared/PageBanner.svelte';
  import { contextLabel } from '$lib/stores/apps.svelte.ts';

  type Accent = 'spec' | 'runtime' | 'control' | 'ephemeral';
  let { appType, title, onitems }: { appType: string; title: string; onitems?: () => void } = $props();

  /// The banner each app's own page shows, kept short here.
  const BANNER: Record<string, { headline: string; highlight: string; subtitle: string; description: string; accent: Accent }> = {
    projects: { headline: 'Organise work into projects.', highlight: 'projects', accent: 'spec',
      subtitle: 'Yours, or your organisation\'s — each one a namespace with its members and rules.',
      description: 'A project you create lives under your handle; an organisation\'s under its name. Make it private to keep it to yourself or to its members.' },
    templates: { headline: 'Start with a template.', highlight: 'template', accent: 'spec',
      subtitle: 'Give it memory, knowledge, and tools. Wire it into the Canvas.',
      description: 'Every A3T agent begins as a typed template. Templates encode domain expertise, so you configure rather than engineer.' },
    agents: { headline: 'Configure an agent.', highlight: 'agent', accent: 'runtime',
      subtitle: 'Bind a template to resources. Set mode, memory, and tools.',
      description: 'An agent is a template with frozen inputs: which LLM to use, which memory store, which MCP servers. Configure once, deploy to any Canvas.' },
    canvas: { headline: 'Deploy a flow as a Canvas.', highlight: 'Canvas', accent: 'control',
      subtitle: 'A canvas is one flow\'s deployment: where it loads, how it scales, what it depends on.',
      description: 'Pick the flow, set placement, lifecycle and health. The canvas resolves to a runtime pool; Deploy puts it there.' },
    triggers: { headline: 'Route inputs to flows.', highlight: 'inputs', accent: 'runtime',
      subtitle: 'HTTP endpoints, cron schedules, and event listeners that start workflows and agents.',
      description: 'Triggers are independent. Any trigger can fan out to several flows. Pause, throttle or rewire without changing the flow itself.' },
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

  /// Placeholders until the docs and videos exist.
  const DOCS = $derived([
    { title: `Getting started with ${title}`, sub: 'The first ten minutes, end to end' },
    { title: `${title}: the concepts`, sub: 'What each part is and how they fit' },
    { title: `How-to guides`, sub: 'Common tasks, one page each' },
  ]);
  const VIDEOS = $derived([
    { title: `${title} in 3 minutes`, len: '3:00' },
    { title: `Your first one, step by step`, len: '8:00' },
    { title: `Tips from the team`, len: '5:00' },
  ]);
</script>

<div class="aw">
  <PageBanner headline={b.headline} highlightWord={b.highlight} subtitle={b.subtitle} description={b.description} accent={b.accent} />

  <div class="aw-start">
    <span>Pick from {what} in Items to open it here, or ask in Chat.</span>
    <span class="sp"></span>
    {#if onitems}<button class="btn" onclick={onitems}>Show Items</button>{/if}
  </div>

  <section class="aw-sec">
    <h3>Learn <span class="ph">placeholder</span></h3>
    <div class="aw-grid">
      {#each DOCS as d (d.title)}
        <div class="aw-card" aria-disabled="true" title="Coming soon">
          <span class="aw-ico doc" aria-hidden="true">
            <svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><rect x="5" y="3" width="14" height="18" rx="2" /><path d="M9 8h6M9 12h6M9 16h4" /></svg>
          </span>
          <span class="aw-t"><b>{d.title}</b><small>{d.sub}</small></span>
        </div>
      {/each}
    </div>
  </section>

  <section class="aw-sec">
    <h3>Watch <span class="ph">placeholder</span></h3>
    <div class="aw-grid">
      {#each VIDEOS as v (v.title)}
        <div class="aw-video" aria-disabled="true" title="Coming soon">
          <div class="aw-thumb">
            <span class="aw-play" aria-hidden="true"><svg width="22" height="22" viewBox="0 0 24 24" fill="currentColor"><path d="M8 5v14l11-7z" /></svg></span>
            <span class="aw-len">{v.len}</span>
          </div>
          <b>{v.title}</b>
        </div>
      {/each}
    </div>
  </section>
</div>

<style>
  .aw { height: 100%; overflow-y: auto; padding: 20px 24px 32px; box-sizing: border-box; display: flex; flex-direction: column; gap: 18px; }
  .aw-start { display: flex; align-items: center; gap: 8px; flex-wrap: wrap; padding: 10px 14px; border: 1px solid var(--border); border-radius: 12px; background: var(--bg-surface); font-size: 13px; color: var(--text-2); }
  .sp { flex: 1 1 auto; }
  .btn { height: 30px; padding: 0 12px; border-radius: 8px; border: 1px solid var(--border); background: var(--bg-surface); font: 500 13px var(--font-sans); color: var(--spec); cursor: pointer; white-space: nowrap; }
  .btn:hover { border-color: var(--spec); }
  .aw-sec h3 { margin: 0 0 12px; font: 400 20px var(--font-serif); color: var(--text-1); display: flex; align-items: center; gap: 8px; }
  .ph { font: 500 10px var(--font-mono); letter-spacing: 0.06em; text-transform: uppercase; color: var(--text-4); border: 1px dashed var(--border-strong, #d9d4c6); border-radius: 4px; padding: 1px 6px; }
  .aw-grid { display: grid; grid-template-columns: repeat(auto-fit, minmax(260px, 1fr)); gap: 14px; }
  .aw-card { display: flex; gap: 14px; align-items: center; min-height: 96px; padding: 18px 20px; border: 1px dashed var(--border-strong, #d9d4c6); border-radius: 12px; background: var(--bg-surface); }
  .aw-ico { flex: 0 0 44px; width: 44px; height: 44px; border-radius: 11px; display: inline-flex; align-items: center; justify-content: center; background: var(--spec-bg); color: var(--spec); }
  .aw-t { display: flex; flex-direction: column; gap: 2px; min-width: 0; }
  .aw-t b { font-size: 15px; font-weight: 600; color: var(--text-1); }
  .aw-t small { font-size: 13px; color: var(--text-3); }
  .aw-video { display: flex; flex-direction: column; gap: 8px; }
  .aw-video b { font-size: 14.5px; font-weight: 600; color: var(--text-1); }
  .aw-thumb { position: relative; aspect-ratio: 16 / 9; border-radius: 12px; border: 1px dashed var(--border-strong, #d9d4c6); background: repeating-linear-gradient(135deg, transparent 0 10px, rgba(234, 230, 220, 0.45) 10px 20px), var(--bg-subtle); display: flex; align-items: center; justify-content: center; }
  .aw-play { width: 56px; height: 56px; border-radius: 50%; background: var(--bg-surface); color: var(--text-2); display: inline-flex; align-items: center; justify-content: center; box-shadow: 0 1px 4px rgba(30, 25, 15, 0.12); padding-left: 2px; }
  .aw-len { position: absolute; right: 8px; bottom: 8px; font: 500 10.5px var(--font-mono); color: var(--text-2); background: var(--bg-surface); border-radius: 4px; padding: 1px 6px; }
</style>
