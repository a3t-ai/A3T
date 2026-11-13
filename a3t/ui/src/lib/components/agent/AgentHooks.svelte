<script lang="ts">
  /// Hooks: every phase of a turn is a hook, and flows attach to hooks. They pass
  /// down the family: the core agent's (system/tmpl_system_agent_engine) fire on every agent,
  /// the template adds its own, and the agent adds its own; none replaces what it
  /// inherits. Every hook shown is read from the definitions (core and template);
  /// what the core runs on every turn is said as it is. Hooks don't run inside the
  /// agent call yet: FlowR AH-1..4 (.ai/CORE_AGENT_HOOKS_SPEC.md).
  import * as YAML from 'yaml';
  import type { AgentRec, TemplateRec } from './agentData';

  let { agent, templates }: { agent: AgentRec; templates: (TemplateRec & { spec_yaml: string })[] } = $props();

  const PHASES: { name: string; when: string }[] = [
    { name: 'on_turn_start', when: 'before the turn' },
    { name: 'before_recall', when: 'before memory recall' },
    { name: 'before_llm', when: 'before each model call' },
    { name: 'on_llm_response', when: 'after each model call' },
    { name: 'on_tool_call', when: 'before each tool call' },
    { name: 'on_tool_error', when: 'a tool call failed' },
    { name: 'before_store', when: 'before the memory write' },
    { name: 'after_response', when: 'the reply is ready' },
    { name: 'on_turn_end', when: 'after the turn' },
  ];
  /// The flows or tasks a definition's hooks run, by hook name.
  function hooksOf(yaml: string | undefined): Record<string, string[]> {
    const out: Record<string, string[]> = {};
    if (!yaml || !yaml.includes('extensions')) return out;
    try {
      const doc = YAML.parse(yaml) ?? {};
      for (const e of doc?.use?.extensions ?? []) {
        const [name, body] = Object.entries(e ?? {})[0] ?? [];
        if (!name || !body) continue;
        const tasks = [...((body as any).before ?? []), ...((body as any).after ?? [])];
        const names = tasks.map((t: any) => {
          const [tn, tb] = Object.entries(t ?? {})[0] ?? [];
          return (tb as any)?.run?.workflow?.name ?? tn;
        }).filter(Boolean);
        if (names.length) out[name as string] = names as string[];
      }
    } catch { /* not YAML we can read: no hooks shown */ }
    return out;
  }

  /// What a definition says about errors: its base, and whether its error
  /// handler is locked for everything that extends it.
  function errorsOf(yaml: string | undefined): { base: string; locked: boolean } | null {
    if (!yaml) return null;
    try {
      const a3t = (YAML.parse(yaml) ?? {})?.metadata?.a3t ?? {};
      return { base: a3t.extends ?? '', locked: (a3t.locked ?? []).includes('onError') };
    } catch { return null; }
  }

  const template = $derived(templates.find((t) => t.id === agent.template_id));
  const core = $derived(templates.find((t) => t.id === 'tmpl_system_agent_engine' || t.name === 'tmpl_system_agent_engine'));
  const fromCore = $derived(hooksOf(core?.spec_yaml));
  const fromTemplate = $derived(hooksOf(template?.spec_yaml));
  const coreErrors = $derived(errorsOf(core?.spec_yaml));
</script>

<section class="agt">
  <div class="order mono">core › template › agent › <b>phase</b> › agent › template › core</div>

  <div class="card">
    <div class="card-head"><h3>What runs on every turn</h3><span>from the core agent's definition{core ? ` (${core.name}@${core.version})` : ''}</span></div>
    <ul class="runs">
      <li><b>Errors</b> — a failed turn goes to <span class="mono">system/flow_system_error_handler</span>, from its base
        <span class="mono">{coreErrors?.base || 'system/flow_system_base'}</span>{coreErrors?.locked ? ', locked: no template or agent replaces it' : ''}. A failed memory or knowledge step is reported there and the turn goes on.</li>
      <li><b>Knowledge</b> — what it knows about A3T is recalled before the model is called (SEMANTIC, <span class="mono">system-knowledge</span>).</li>
      <li><b>Tools</b> — the tools that fit the message are recalled from the tool catalogue (PROCEDURAL, <span class="mono">system-tools</span>).</li>
      <li><b>Turn log</b> — one row per turn in <span class="mono">a3t_agent_turns</span>: its Metrics on Overview.</li>
    </ul>
    <p class="dim">These are steps of the core, not hooks: every agent runs them. The phases below are where flows attach once the engine runs hooks inside the agent call (FlowR AH-1 to AH-4); until then no phase runs a flow, and none can be added here.</p>
  </div>

  <div class="card">
    <table>
      <thead>
        <tr>
          <th scope="col">Hook</th>
          <th scope="col">tmpl_system_agent_engine <span class="dim">core</span></th>
          <th scope="col">{template?.name ?? 'template'} <span class="dim">template</span></th>
          <th scope="col">{agent.name} <span class="dim">this agent</span></th>
        </tr>
      </thead>
      <tbody>
        {#each PHASES as p (p.name)}
          <tr>
            <th scope="row"><span class="mono">{p.name}</span><br /><span class="dim">{p.when}</span></th>
            <td>{#each fromCore[p.name] ?? [] as f}<span class="chip core">{f}</span>{/each}</td>
            <td>{#each fromTemplate[p.name] ?? [] as f}<span class="chip tpl">{f}</span>{/each}</td>
            <td></td>
          </tr>
        {/each}
      </tbody>
    </table>
  </div>
</section>

<style>
  .order { align-self: flex-start; padding: 7px 10px; border: 1px solid var(--border); border-radius: 6px; background: var(--bg-surface); font-size: 11.5px; }
  .order b { color: var(--text-1); }
  table { width: 100%; border-collapse: collapse; font-size: 12.5px; }
  th, td { padding: 8px 8px; border-top: 1px solid var(--divider); text-align: left; vertical-align: top; font-weight: 400; }
  thead th { border-top: 0; color: var(--text-2); font-weight: 500; background: var(--bg-subtle); }
  th[scope='row'] { width: 170px; }
  th[scope='row'] .dim { font-size: 11px; }
  .chip { display: inline-block; margin: 0 4px 4px 0; padding: 2px 8px; border-radius: 5px; font: 11.5px var(--font-mono); }
  .chip.core { background: var(--bg-subtle); color: var(--text-2); }
  .chip.tpl { background: var(--ephemeral-bg); color: var(--ephemeral-ink); }
  .runs { margin: 0; padding-left: 18px; display: flex; flex-direction: column; gap: 6px; font-size: 12.5px; color: var(--text-2); }
  .mono { font-family: var(--font-mono); font-size: 0.92em; }
</style>
