<script lang="ts">
  /// Hooks: every phase of a turn is a hook, and flows attach to hooks. They pass
  /// down the family: the core agent's (system/tmpl_system_agent_engine) fire on every agent,
  /// the template adds its own, and the agent adds its own; none replaces what it
  /// inherits. The template's hooks are read from its definition. Hooks don't run
  /// inside the agent call yet, and flows can't attach yet: FlowR AH-1..4
  /// (.ai/CORE_AGENT_HOOKS_SPEC.md, branch core-agent).
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
  /// What the core agent attaches (spec: .ai/CORE_AGENT_HOOKS_SPEC.md).
  const CORE: Record<string, string> = {
    on_turn_start: 'flow_system_hook_turn_start',
    before_llm: 'flow_system_hook_knowledge',
    on_tool_error: 'flow_system_error_handler',
    on_turn_end: 'flow_system_hook_turn_log',
  };

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

  const template = $derived(templates.find((t) => t.id === agent.template_id));
  const fromTemplate = $derived(hooksOf(template?.spec_yaml));
</script>

<section class="agt">
  <div class="order mono">core › template › agent › <b>phase</b> › agent › template › core</div>

  <div class="sample"><b>Not yet</b><span>Hooks don't run inside the agent call yet, and flows can't be attached here until they do. Requested from FlowR as AH-1 to AH-4.</span></div>

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
            <td>{#if CORE[p.name]}<span class="chip core" title="Locked: every agent runs it">{CORE[p.name]}</span>{/if}</td>
            <td>{#each fromTemplate[p.name] ?? [] as f}<span class="chip tpl">{f}</span>{/each}</td>
            <td><button class="add" disabled title="Attaching flows comes with FlowR AH-1..4">+ flow</button></td>
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
  .add { height: 24px; padding: 0 8px; border: 1px dashed var(--border-strong); border-radius: 5px; background: none; color: var(--text-3); font: inherit; font-size: 11.5px; }
</style>
