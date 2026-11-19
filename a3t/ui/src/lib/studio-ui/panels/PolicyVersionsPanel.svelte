<script lang="ts">
  /// Versions (below a policy file): every save, switch and removal, with who
  /// and when. Load one into the editor (Save then keeps it as the newest
  /// version), or compare it with the editor.
  import { untrack } from 'svelte';
  import { orgPolicy, type Doc, type Workspace } from '$lib/studio-core';
  import { compareVersions } from '../sync';

  let { doc, workspace }: { doc: Doc | null; workspace: Workspace } = $props();

  const org = $derived(doc ? orgPolicy.orgOf(doc.ref.project) : '');
  type H = { version: string; action: string; content: string; by: string; at: string };
  let rows = $state<H[]>([]);
  let err = $state('');
  let note = $state('');
  async function refresh() {
    if (!doc || !org || doc.isNew) { rows = []; return; }
    const r = await orgPolicy.file(org, doc.ref.name);
    if (!r.ok) { err = r.error; rows = []; return; }
    err = ''; rows = r.data.history;
  }
  $effect(() => { void doc?.address; void doc?.saved; untrack(() => void refresh()); });

  const WHAT: Record<string, string> = { create: 'created', save: 'saved', delete: 'removed', on: 'switched on', off: 'switched off' };
  const when = (s: string) => { const t = Date.parse(s); return Number.isFinite(t) ? new Date(t).toLocaleString(undefined, { month: 'short', day: 'numeric', hour: '2-digit', minute: '2-digit' }) : s; };
  const isText = (h: H) => h.action === 'create' || h.action === 'save';

  async function load(h: H) {
    if (!doc) return;
    if (workspace.isDirty(doc) && !(await workspace.host.confirm(`Replace the edits in ${doc.ref.name} with ${h.version}? Unsaved edits are lost.`))) return;
    doc.text = h.content;
    note = `${h.version} is in the editor: Save makes it the newest version (if it passes Validate).`;
  }
</script>

<div class="pv">
  {#if !doc || !org}<p class="dim">Open an org's policy file to see its versions.</p>
  {:else if doc.isNew}<p class="dim">Not saved yet: its first save is version v1.</p>
  {:else if err}<p class="bad">{err}</p>
  {:else}
    {#if note}<p class="note">{note}</p>{/if}
    <table>
      <thead><tr><th>Version</th><th>What</th><th>Who</th><th>When</th><th></th></tr></thead>
      <tbody>
        {#each rows as h, i (i)}
          <tr>
            <td class="mono">{h.version}{doc.data?.file?.version === h.version && isText(h) ? ' · now' : ''}</td>
            <td>{WHAT[h.action] ?? h.action}</td>
            <td>{h.by}</td>
            <td>{when(h.at)}</td>
            <td class="acts">{#if isText(h)}<button class="lnk" onclick={() => load(h)}>Load</button><button class="lnk" onclick={() => compareVersions(doc!, workspace, 'editor', h.version)}>Compare</button>{/if}</td>
          </tr>
        {:else}<tr><td colspan="5" class="dim">No versions yet.</td></tr>{/each}
      </tbody>
    </table>
  {/if}
</div>

<style>
  .pv { padding: 10px 14px; font-size: 12.5px; }
  table { width: 100%; border-collapse: collapse; }
  th { text-align: left; font-weight: 550; color: var(--text-4); font-size: 11px; padding: 4px 8px; border-bottom: 1px solid var(--border); }
  td { padding: 5px 8px; border-bottom: 1px solid var(--divider, var(--border)); }
  .mono { font-family: var(--font-mono); }
  .acts { text-align: right; white-space: nowrap; }
  .lnk { border: 0; background: none; padding: 0 0 0 10px; color: var(--accent, #5530a8); cursor: pointer; font: inherit; }
  .dim { color: var(--text-4); margin: 0; }
  .bad { color: var(--err, #b42318); margin: 0; }
  .note { margin: 0 0 8px; color: var(--text-2); }
</style>
