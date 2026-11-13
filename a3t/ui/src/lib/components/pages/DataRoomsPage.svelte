<script lang="ts">
  /// Data Rooms — the governed places agents read and write documents: an S3
  /// bucket or prefix, a Box or SharePoint folder. For security and governance:
  /// what each room holds, how it is classified, and which agents may read or
  /// write it. SAMPLE: the platform has no storage resource types and no
  /// per-agent scoped access yet (SRV-21); nothing here is saved.
  type Access = { agent: string; mode: 'read' | 'write' };
  type Room = { name: string; store: 'S3' | 'Box' | 'SharePoint'; location: string; project: string; classification: 'restricted' | 'confidential' | 'internal'; owner: string; access: Access[]; lastRead: string };
  const ROOMS: Room[] = [
    { name: 'deal-q4', store: 'S3', location: 's3://acme-datarooms/deal-q4/', project: 'acme', classification: 'restricted', owner: 'legal@acme.com',
      access: [{ agent: 'diligence-reader', mode: 'read' }, { agent: 'contract-redliner', mode: 'write' }], lastRead: '12 min ago' },
    { name: 'support-kb', store: 'Box', location: 'Box › Support › Knowledge base', project: 'support', classification: 'internal', owner: 'support-lead@acme.com',
      access: [{ agent: 'refund-triage', mode: 'read' }, { agent: 'triage-bot', mode: 'read' }], lastRead: '2 h ago' },
    { name: 'hr-policies', store: 'SharePoint', location: 'HR › Policies', project: 'acme', classification: 'confidential', owner: 'hr@acme.com',
      access: [], lastRead: '—' },
  ];
  let open = $state('');
</script>

<div class="screen-header">
  <div class="breadcrumb">Data Rooms <span class="sep">/</span> All</div>
  <h1 class="screen-title">Data Rooms</h1>
</div>
<div class="body">
  <div class="banner">
    The places agents read and write documents — an S3 bucket or prefix, a Box or SharePoint folder — each with a classification and the agents allowed in, read or write.
  </div>
  <div class="sample"><b>Sample</b> Data rooms are not stored yet: the platform has no storage resources (S3, Box, SharePoint) or per-agent access to them (SRV-21). This shows how the screen will work; nothing here is saved.</div>

  <table class="tbl">
    <thead><tr><th>Room</th><th>Store</th><th>Classification</th><th>Agents with access</th><th>Last read</th></tr></thead>
    <tbody>
      {#each ROOMS as r (r.name)}
        <tr class:on={open === r.name} onclick={() => (open = open === r.name ? '' : r.name)}>
          <td><b>{r.name}</b><div class="dim mono">{r.location}</div></td>
          <td>{r.store}</td>
          <td><span class="cls {r.classification}">{r.classification}</span></td>
          <td>{#if r.access.length}{r.access.map((a) => `${a.agent} (${a.mode})`).join(', ')}{:else}<span class="dim">none</span>{/if}</td>
          <td class="dim">{r.lastRead}</td>
        </tr>
        {#if open === r.name}
          <tr class="detail"><td colspan="5">
            <dl class="kv">
              <dt>Project</dt><dd class="mono">{r.project}</dd>
              <dt>Owner</dt><dd>{r.owner}</dd>
              <dt>Write access</dt><dd>{r.access.filter((a) => a.mode === 'write').map((a) => a.agent).join(', ') || 'no agent may write'}</dd>
              <dt>Rules</dt><dd>{r.classification === 'restricted' ? 'Only agents in its project; every read is logged; no external model without approval.' : r.classification === 'confidential' ? 'Agents need an approver to be granted access.' : 'Any agent in the project may be granted read.'}</dd>
            </dl>
          </td></tr>
        {/if}
      {/each}
    </tbody>
  </table>
</div>

<style>
  .screen-header { padding: 18px 32px 12px; border-bottom: 1px solid var(--border); background: var(--bg-page); }
  .breadcrumb { font-size: 12px; color: var(--text-3); margin-bottom: 8px; font-family: var(--font-mono); }
  .breadcrumb .sep { color: var(--text-5, var(--text-4)); }
  .screen-title { font-family: var(--font-serif); font-size: 26px; font-weight: 500; margin: 0; }
  .body { padding: 24px 32px; display: flex; flex-direction: column; gap: 14px; max-width: var(--detail-w); overflow-y: auto; }
  .banner { padding: 12px 14px; border-radius: 8px; background: var(--bg-subtle); border: 1px solid var(--border); font-size: 13px; color: var(--text-2); line-height: 1.55; }
  .sample { display: flex; gap: 10px; align-items: baseline; padding: 9px 12px; border: 1px dashed var(--warn, #b45309); border-radius: 8px; background: var(--warn-bg, #fbf0e0); font-size: 12.5px; color: var(--text-2); }
  .sample b { color: var(--warn, #b45309); }
  .dim { color: var(--text-4); font-size: 12px; }
  .mono { font-family: var(--font-mono); }
  .tbl { width: 100%; border-collapse: collapse; background: var(--bg-surface); border: 1px solid var(--border); border-radius: 8px; overflow: hidden; font-size: 13px; }
  .tbl th { text-align: left; font-size: 10.5px; font-weight: 600; text-transform: uppercase; letter-spacing: 0.05em; color: var(--text-4); padding: 10px 14px; border-bottom: 1px solid var(--border); }
  .tbl td { padding: 10px 14px; border-top: 1px solid var(--divider, var(--border)); vertical-align: top; }
  .tbl tbody tr { cursor: pointer; }
  .tbl tbody tr:hover td, .tbl tbody tr.on td { background: var(--bg-subtle); }
  .tbl tr.detail { cursor: default; }
  .cls { font: 600 10px var(--font-mono); text-transform: uppercase; letter-spacing: 0.06em; padding: 2px 7px; border-radius: 4px; background: var(--bg-subtle); color: var(--text-3); }
  .cls.restricted { background: color-mix(in srgb, var(--err) 12%, transparent); color: var(--err); }
  .cls.confidential { background: var(--warn-bg, #fbf0e0); color: var(--warn, #b45309); }
  .kv { display: grid; grid-template-columns: 110px 1fr; gap: 6px 14px; margin: 0; font-size: 12.5px; }
  .kv dt { color: var(--text-3); }
  .kv dd { margin: 0; }
</style>
