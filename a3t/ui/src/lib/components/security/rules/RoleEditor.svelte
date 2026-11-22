<script lang="ts">
  /// An org's own role: a baseline role (Admin, Operator, Viewer) with some of
  /// its permissions taken away. Saved as the policy file roles/<name>.yaml,
  /// checked with the org's other files by the server, held for approval when
  /// the org's settings say so. People and groups get it in a project's
  /// access file, like a baseline role.
  import * as YAML from 'yaml';
  import { untrack } from 'svelte';
  import { orgPolicy, type CatalogueEntry } from '$lib/services/api';

  let { org, perms, path = null, content = '', canEdit, taken = [], onclose, onsaved }: {
    org: string; perms: CatalogueEntry[]; path?: string | null; content?: string; canEdit: boolean;
    /// Names already used by the org's other roles.
    taken?: string[];
    onclose: () => void; onsaved: (msg: string) => void;
  } = $props();

  const BASE = [
    { id: 'Admin', what: 'builds and changes flows, agents and resources' },
    { id: 'Operator', what: 'runs them' },
    { id: 'Viewer', what: 'reads them' },
  ];

  const y0 = (() => { try { return (YAML.parse(content || '') ?? {}) as any; } catch { return {}; } })();
  const known = new Set(untrack(() => perms).map((p) => p.id));
  const removed0: string[] = Array.isArray(y0.remove) ? y0.remove.map(String) : [];
  let name = $state(String(y0.name ?? ''));
  let description = $state(String(y0.description ?? ''));
  let base = $state(String(y0.extends ?? 'Operator'));
  let remove = $state<string[]>(removed0.filter((x) => known.has(x)));
  let patterns = $state(removed0.filter((x) => !known.has(x)).join(', '));
  let busy = $state(false);
  let err = $state('');

  const editing = $derived(path !== null);
  const nameErr = $derived(!name ? '' : !/^[a-z0-9][a-z0-9-]{0,62}$/.test(name) ? 'lowercase letters, digits and dashes' : BASE.some((b) => b.id.toLowerCase() === name) || (!editing && taken.includes(name)) ? 'already a role' : '');
  const patternList = $derived(patterns.split(',').map((s) => s.trim()).filter(Boolean));
  const patternErr = $derived(patternList.find((p) => !/^tools\.[a-z0-9_*-]+$/.test(p)) ?? '');

  // Group the catalogue by what it is about: apps, tools, memory, flows…
  const groups = $derived.by(() => {
    const m = new Map<string, CatalogueEntry[]>();
    for (const p of perms) { const g = p.enforced_by === 'console' ? 'Console apps (hidden)' : p.id.split('.')[0]; (m.get(g) ?? m.set(g, []).get(g)!).push(p); }
    return [...m.entries()];
  });
  const toggle = (id: string) => (remove = remove.includes(id) ? remove.filter((x) => x !== id) : [...remove, id]);

  const yaml = $derived.by(() => {
    const doc: any = { kind: 'a3t/role', name: name || '<name>' };
    if (description.trim()) doc.description = description.trim();
    doc.extends = base;
    const all = [...remove, ...patternList];
    if (all.length) doc.remove = all;
    return YAML.stringify(doc);
  });

  async function save() {
    if (!name || nameErr || patternErr) return;
    busy = true; err = '';
    const p = path ?? `roles/${name}.yaml`;
    const r = await orgPolicy.save(org, p, yaml);
    busy = false;
    if (!r.ok) { err = ((r as any).detail?.errors ?? [r.error ?? `HTTP ${r.status}`]).join('; '); return; }
    onsaved(r.data?.pending ? (r.data.message ?? 'Held for approval (Policies › Changes).') : `Role ${name} saved in ${p}.`);
  }
  async function del() {
    if (!path || !confirm(`Delete the role ${name}? Anyone given it in an access file loses it.`)) return;
    busy = true; err = '';
    const r = await orgPolicy.remove(org, path);
    busy = false;
    if (!r.ok) { err = ((r as any).detail?.errors ?? [r.error ?? `HTTP ${r.status}`]).join('; '); return; }
    onsaved((r.data as any)?.pending ? ((r.data as any).message ?? 'Held for approval (Policies › Changes).') : `Role ${name} deleted.`);
  }
  let showYaml = $state(false);
</script>

<div class="card">
  <div class="card-head"><h3>{editing ? `Role ${name}` : 'New role'}</h3><span>a baseline role with some permissions taken away</span><span class="grow"></span><button class="btn2 quiet" onclick={onclose}>← Roles</button></div>
  <div class="fields">
    <label class="fld">Name<input class="mono" bind:value={name} readonly={editing || !canEdit} placeholder="support-operator" />{#if nameErr}<small class="bad">{nameErr}</small>{/if}</label>
    <label class="fld">Description<input bind:value={description} readonly={!canEdit} placeholder="Runs support flows; can't change memory" /></label>
  </div>
  <h4 class="sub">Starts from</h4>
  <div class="base">
    {#each BASE as b (b.id)}
      <label class="radio"><input type="radio" name="base" value={b.id} bind:group={base} disabled={!canEdit} /><span><b>{b.id}</b><small>{b.what}</small></span></label>
    {/each}
  </div>
  <h4 class="sub">Takes away <span class="dim">({remove.length + patternList.length})</span></h4>
  <div class="groups">
    {#each groups as [g, list] (g)}
      <div class="grp"><div class="gname">{g}</div>
        {#each list as p (p.id)}
          <label class="perm" title={p.note ?? p.id}><input type="checkbox" checked={remove.includes(p.id)} disabled={!canEdit} onchange={() => toggle(p.id)} /><span>{p.label}</span></label>
        {/each}
      </div>
    {/each}
  </div>
  <label class="fld">Tools by name (optional)<input class="mono" bind:value={patterns} readonly={!canEdit} placeholder="tools.delete_*, tools.send_email" />{#if patternErr}<small class="bad">{patternErr}: tools.&lt;name&gt;, * matches any text</small>{/if}</label>
  <p class="dim small">Give the role to people or groups in a project's access file (Org members › Groups), as you would Admin, Operator or Viewer.</p>
  {#if err}<div class="note bad">{err}</div>{/if}
  <div class="acts">
    {#if canEdit}<button class="btn2 primary" disabled={busy || !name || !!nameErr || !!patternErr} onclick={save}>{busy ? 'Saving…' : editing ? 'Save' : 'Create role'}</button>{/if}
    <button class="btn2 quiet" onclick={() => (showYaml = !showYaml)}>{showYaml ? 'Hide' : 'Show'} YAML</button>
    {#if canEdit && editing}<span class="grow"></span><button class="btn2 quiet danger" disabled={busy} onclick={del}>Delete role</button>{/if}
  </div>
  {#if showYaml}<pre class="mono yaml">{yaml}</pre>{/if}
</div>

<style>
  .grow { flex: 1; }
  .bad { color: var(--bad, #b42318); font-size: 11.5px; }
  .base { display: grid; grid-template-columns: repeat(3, minmax(0, 1fr)); gap: 10px; }
  .radio, .radio * { text-transform: none; letter-spacing: normal; font-family: var(--font-sans); }
  .radio { display: flex; gap: 8px; align-items: flex-start; font-size: 13px; color: var(--text-1); cursor: pointer; padding: 8px 10px; border: 1px solid var(--border); border-radius: 8px; }
  .radio span { display: flex; flex-direction: column; gap: 2px; }
  .radio small { color: var(--text-3); font-size: 12px; }
  .radio b { font-weight: 500; }
  .groups { display: grid; grid-template-columns: repeat(auto-fill, minmax(220px, 1fr)); gap: 12px; margin: 4px 0 10px; }
  .gname { font: 600 11px var(--font-sans); letter-spacing: 0.05em; text-transform: uppercase; color: var(--text-4); margin-bottom: 4px; }
  .perm { display: flex; gap: 6px; align-items: baseline; font-size: 12.5px; color: var(--text-2); padding: 2px 0; text-transform: none; letter-spacing: normal; font-family: var(--font-sans); cursor: pointer; }
  .sub { margin: 10px 0 6px; font: 600 12.5px var(--font-sans); color: var(--text-2); }
  .yaml { white-space: pre-wrap; font-size: 12px; background: var(--bg-page); padding: 10px; border-radius: 8px; }
  .danger { color: var(--bad, #b42318); }
  @container details (max-width: 760px) { .base { grid-template-columns: 1fr; } }
</style>
