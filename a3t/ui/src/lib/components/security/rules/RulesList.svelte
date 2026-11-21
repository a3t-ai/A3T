<script lang="ts">
  /// Every rule of an org, one row per rule (not per file): its sentence, where
  /// it applies, whether it is on, in shadow or off, and what it decided in the
  /// last 7 days. A row opens the rule; the file it lives in is shown small.
  import * as YAML from 'yaml';
  import { audit, type OrgPolicyFiles } from '$lib/services/api';
  import { rulesOf, sentence } from './model';

  let { org, files, projects, perms, canEdit, onopen }: {
    org: string; files: OrgPolicyFiles | null; projects: string[];
    perms: { id: string; label: string }[]; canEdit: boolean;
    onopen: (t: { path: string; index: number } | null) => void;
  } = $props();

  const parse = (t: string): any => { try { return YAML.parse(t) ?? {}; } catch { return {}; } };
  const label = (id: string) => perms.find((p) => p.id === id)?.label ?? (id.startsWith('tools.') ? `call ${id.slice(6)}` : id);

  type Row = { path: string; index: number; id: string; say: string; where: string; projects: string[]; state: 'on' | 'shadow' | 'off'; raw: boolean };
  const rows = $derived.by(() => {
    const out: Row[] = [];
    for (const f of files?.files ?? []) {
      if (f.kind !== 'policy') continue;
      const y = parse(f.content);
      const ps: string[] = y?.applies_to?.projects ?? [];
      const where = y?.applies_to?.org || !ps.length ? 'whole org' : ps.join(', ');
      const state = f.enforced ? 'on' : (f as any).shadow ? 'shadow' : 'off';
      rulesOf(y).forEach((r, i) => {
        const raw = y.rules?.[i];
        out.push({ path: f.path, index: i, id: `${org}/${y.name}/r${i + 1}`, projects: ps, where, state, raw: !r,
          say: r ? sentence(r, label) : (raw?.reason ? `${raw.reason}` : 'A rule this page can\'t show') });
      });
      if (y?.cedar) out.push({ path: f.path, index: -1, id: `${org}/${y.name}/cedar`, projects: ps, where, state, raw: true, say: 'Cedar written by hand (expert block)' });
    }
    return out;
  });
  const others = $derived((files?.files ?? []).filter((f) => f.kind !== 'policy'));

  let scope = $state('');
  const shown = $derived(rows.filter((x) => !scope || !x.projects.length || x.projects.includes(scope)));
  const n = (s: string) => rows.filter((x) => x.state === s).length;

  // What each rule decided in the last 7 days (refused, asked), from the decision log.
  let counts = $state<Record<string, { deny: number; ask: number }>>({});
  let countsNote = $state('');
  $effect(() => {
    const h = org;
    void (async () => {
      const r = await audit.query({ org: h, since_ms: Date.now() - 7 * 864e5, limit: 1000 });
      if (h !== org) return;
      if (!r.ok) { countsNote = `Decisions could not be read: ${r.error ?? r.status}`; return; }
      const c: Record<string, { deny: number; ask: number }> = {};
      for (const d of r.data?.records ?? []) {
        const ids: string[] = d.policy_ids ?? d.policies ?? [];
        for (const id of ids) {
          const key = id.replace(/^(.+\/r\d+)\..*$/, '$1');
          c[key] ??= { deny: 0, ask: 0 };
          if (d.decision === 'deny') c[key].deny++; else if (d.decision === 'ask') c[key].ask++;
        }
      }
      counts = c; countsNote = '';
    })();
  });
  const did = (id: string, state: string) => {
    const c = counts[id]; if (!c || (!c.deny && !c.ask)) return '—';
    const verb = state === 'shadow' ? 'would refuse' : 'refused';
    return [c.deny ? `${verb} ${c.deny}` : '', c.ask ? `asked ${c.ask}` : ''].filter(Boolean).join(' · ');
  };
</script>

<div class="rl">
  <div class="row">
    <select class="tin" bind:value={scope} aria-label="Where">
      <option value="">{org} · all projects</option>{#each projects as p (p)}<option value={p}>{p}</option>{/each}
    </select>
    <span class="st on">On {n('on')}</span><span class="st shadow">Shadow {n('shadow')}</span><span class="st off">Off {n('off')}</span>
    <span class="grow"></span>
    {#if canEdit}<button class="btn2 primary" onclick={() => onopen(null)}>+ New rule</button>{/if}
  </div>

  {#if !rows.length}
    <div class="card"><p class="empty">{org} has no rules yet. {canEdit ? 'Add one with New rule: describe it in words or start from a pattern.' : ''}</p></div>
  {:else}
    <div class="card tbl">
      <table>
        <thead><tr><th>Rule</th><th>Where</th><th>State</th><th>Last 7 days</th></tr></thead>
        <tbody>
          {#each shown as x (x.id)}
            <tr onclick={() => x.index >= 0 && onopen({ path: x.path, index: x.index })} class:raw={x.raw}>
              <td><div class="say">{x.say}</div><div class="file mono">{x.path}{x.index >= 0 ? ` · rule ${x.index + 1}` : ''}</div></td>
              <td>{x.where}</td>
              <td><span class="st {x.state}">{x.state}</span></td>
              <td class="num">{did(x.id, x.state)}</td>
            </tr>
          {/each}
        </tbody>
      </table>
    </div>
  {/if}
  {#if countsNote}<p class="dim">{countsNote}</p>{/if}
  {#if others.length}
    <p class="dim small">Also deciding: {others.map((f) => f.path).join(', ')} (roles and access, under Files), and the platform's built-in rules.</p>
  {/if}
</div>

<style>
  .rl { display: flex; flex-direction: column; gap: 12px; }
  .small { font-size: 11.5px; }
  .tin { height: 32px; padding: 0 10px; border: 1px solid var(--border); border-radius: 8px; background: var(--bg-surface); font: 13px var(--font-sans); color: var(--text-1); }
  .st { font: 500 11px var(--font-sans); border-radius: 999px; padding: 2px 9px; background: var(--bg-subtle); color: var(--text-3); text-transform: capitalize; white-space: nowrap; }
  .st.on { background: color-mix(in srgb, var(--ok, #1a7158) 13%, transparent); color: var(--ok, #1a7158); }
  .st.shadow { background: var(--warn-bg, #fbf0e0); color: var(--warn, #b45309); }
  .tbl { padding: 4px 8px; }
  table { width: 100%; border-collapse: collapse; font-size: 13px; }
  th { text-align: left; font: 500 10.5px var(--font-sans); letter-spacing: 0.05em; text-transform: uppercase; color: var(--text-4); padding: 8px 8px; border-bottom: 1px solid var(--border); }
  td { padding: 9px 8px; border-bottom: 1px solid var(--border); vertical-align: top; color: var(--text-2); }
  tr:last-child td { border-bottom: 0; }
  tbody tr { cursor: pointer; }
  tbody tr:hover td { background: var(--bg-subtle); }
  tr.raw .say { color: var(--text-3); font-style: italic; }
  .say { color: var(--text-1); line-height: 1.4; }
  .file { font-size: 11px; color: var(--text-4); margin-top: 2px; }
  .num { white-space: nowrap; }
  @container details (max-width: 700px) { th:nth-child(2), td:nth-child(2) { display: none; } }
</style>
