<script lang="ts">
  /// Who governs an org: who may change its policies and its identity, who
  /// approves a held change, the second approver, the org's time zone (rule
  /// hours and dates are in it), and what each role can do as a result. Kept
  /// and enforced by the server; changed by the org's security admins and the
  /// platform's SuperAdmin and SecurityAdmin, read by its admins.
  import type { OrgGovernance, OrgSettings } from '$lib/services/api';

  let { org, settings, busy, onFourEyes, onSave }: {
    org: string; settings: OrgSettings | null; busy: string;
    onFourEyes: () => void; onSave: (g: Partial<OrgGovernance>) => void;
  } = $props();

  const g = $derived<OrgGovernance>(settings?.governance ?? { policy_lock: 'both', identity_lock: 'both', approver: 'securityadmin', timezone: null });
  const canGovern = $derived(!!settings?.can_govern);
  const fourEyes = $derived(!!settings?.four_eyes);
  const ZONES = ['UTC', 'Europe/London', 'Europe/Berlin', 'Europe/Paris', 'Europe/Amsterdam', 'Europe/Stockholm', 'America/New_York', 'America/Chicago', 'America/Denver', 'America/Los_Angeles', 'America/Sao_Paulo', 'Asia/Kolkata', 'Asia/Singapore', 'Asia/Tokyo', 'Australia/Sydney'];
  const zones = $derived(g.timezone && !ZONES.includes(g.timezone) ? [g.timezone, ...ZONES] : ZONES);

  const Y = '✓';
  const matrix = $derived([
    { what: 'See sign-in, groups and people', sa: Y, sec: Y, osa: Y, oa: Y, m: 'themselves' },
    { what: 'Change sign-in, SCIM, members, group mapping', sa: Y, sec: Y, osa: Y, oa: g.identity_lock === 'both' ? Y : '—', m: '—' },
    { what: 'Read rules and decisions, try rules', sa: Y, sec: Y, osa: Y, oa: Y, m: 'rules on them' },
    { what: 'Write rules and policy files', sa: Y, sec: Y, osa: Y, oa: g.policy_lock === 'both' ? (fourEyes ? 'propose' : Y) : 'propose', m: '—' },
    { what: 'Approve a held change', sa: Y, sec: Y, osa: Y, oa: g.approver === 'admin' ? Y : '—', m: '—' },
    { what: 'This page (who governs)', sa: Y, sec: Y, osa: Y, oa: 'read', m: '—' },
    { what: 'Platform sign-in and built-in rules', sa: Y, sec: Y, osa: 'read', oa: '—', m: '—' },
  ]);
  const roleWord = (r: string) => (r === 'platform' ? 'platform InfoSec' : r === 'securityadmin' ? `${org}'s security admin` : r === 'admin' ? `${org}'s admin` : r || 'a reader');
</script>

<div class="gs">
  <div class="note">{#if canGovern}You can change this page: you are {roleWord(settings?.my_role ?? '')}. {org}'s admins can read it.{:else}You can read this page. {org}'s security admins and the platform's SuperAdmin and SecurityAdmin change it.{/if}</div>

  <div class="two">
    <div class="card">
      <div class="card-head"><h3>Who may change policies</h3></div>
      <label class="radio"><input type="radio" name="pl" checked={g.policy_lock === 'both'} disabled={!canGovern || !!busy} onchange={() => onSave({ policy_lock: 'both' })} /><span><b>Org admins and org security admins</b><small>rules, roles, access files, networks, calls out</small></span></label>
      <label class="radio"><input type="radio" name="pl" checked={g.policy_lock === 'security'} disabled={!canGovern || !!busy} onchange={() => onSave({ policy_lock: 'security' })} /><span><b>Org security admins only</b><small>admins still read every rule, try it, and propose a change that is held until approved</small></span></label>
    </div>
    <div class="card">
      <div class="card-head"><h3>Who may change identity</h3></div>
      <label class="radio"><input type="radio" name="il" checked={g.identity_lock === 'both'} disabled={!canGovern || !!busy} onchange={() => onSave({ identity_lock: 'both' })} /><span><b>Org admins and org security admins</b><small>the org's provider, SCIM, members, groups → roles</small></span></label>
      <label class="radio"><input type="radio" name="il" checked={g.identity_lock === 'security'} disabled={!canGovern || !!busy} onchange={() => onSave({ identity_lock: 'security' })} /><span><b>Org security admins only</b><small>admins see members and sign-in, and can't change them</small></span></label>
    </div>
  </div>

  <div class="card">
    <div class="card-head"><h3>Approving changes</h3></div>
    <dl class="kv">
      <dt>Second approver</dt><dd class="row"><span class="dot" class:ok={fourEyes}></span>{fourEyes ? 'on: every policy change waits for another person' : 'off: a change applies when saved'}{#if canGovern}<button class="btn2" disabled={!!busy} onclick={onFourEyes}>{fourEyes ? 'Turn off…' : 'Turn on'}</button>{/if}</dd>
      <dt>Who approves</dt><dd><select class="tin" value={g.approver} disabled={!canGovern || !!busy} onchange={(e) => onSave({ approver: e.currentTarget.value as OrgGovernance['approver'] })}><option value="securityadmin">an org security admin</option><option value="admin">an org admin or security admin</option></select>{#if settings && !settings.can_approve}<span class="dim small"> · not you</span>{/if}</dd>
      <dt>Own changes</dt><dd>never approved by their author</dd>
      <dt>Platform roles</dt><dd>SuperAdmin and SecurityAdmin can always change {org}'s identity and policies; {org} can't lock them out, and what they change is in the decision log.</dd>
    </dl>
  </div>

  <div class="card">
    <div class="card-head"><h3>Time zone</h3><span>rule hours and dates are in it</span></div>
    <dl class="kv">
      <dt>{org}'s time zone</dt><dd><select class="tin" value={g.timezone ?? 'UTC'} disabled={!canGovern || !!busy} onchange={(e) => onSave({ timezone: e.currentTarget.value === 'UTC' ? null : e.currentTarget.value })}>{#each zones as z (z)}<option value={z}>{z.replace('_', ' ')}</option>{/each}</select></dd>
    </dl>
  </div>

  <div class="card">
    <div class="card-head"><h3>Who can do what in {org}</h3><span>with the choices above</span></div>
    <table>
      <thead><tr><th></th><th>SuperAdmin</th><th>SecurityAdmin</th><th>Org security admin</th><th>Org admin</th><th>Member</th></tr></thead>
      <tbody>{#each matrix as x (x.what)}<tr><td>{x.what}</td>{#each [x.sa, x.sec, x.osa, x.oa, x.m] as v, i (i)}<td class:y={v === Y} class:p={v === 'propose'}>{v}</td>{/each}</tr>{/each}</tbody>
    </table>
  </div>
</div>

<style>
  .gs { display: flex; flex-direction: column; gap: 14px; }
  .small { font-size: 11.5px; }
  .two { display: grid; grid-template-columns: 1fr 1fr; gap: 14px; }
  .radio, .radio * { text-transform: none; letter-spacing: normal; font-family: var(--font-sans); }
  .radio { display: flex; gap: 10px; align-items: flex-start; font-size: 13px; color: var(--text-1); cursor: pointer; }
  .radio input { margin-top: 3px; }
  .radio span { display: flex; flex-direction: column; gap: 2px; }
  .radio small { color: var(--text-3); font-size: 12px; }
  .radio b { font-weight: 500; }
  .kv { grid-template-columns: 150px 1fr; }
  .tin { height: 30px; padding: 0 8px; border: 1px solid var(--border); border-radius: 7px; background: var(--bg-surface); font: 12.5px var(--font-sans); color: var(--text-1); width: auto; }
  table { width: 100%; border-collapse: collapse; font-size: 12.5px; }
  th { font: 500 10.5px var(--font-sans); letter-spacing: 0.05em; text-transform: uppercase; color: var(--text-4); padding: 6px; border-bottom: 1px solid var(--border); text-align: center; }
  td { padding: 7px 6px; border-bottom: 1px solid var(--border); text-align: center; color: var(--text-4); }
  td:first-child, th:first-child { text-align: left; color: var(--text-2); }
  td.y { color: var(--ok, #1a7158); font-weight: 600; }
  td.p { color: var(--warn, #b45309); }
  tr:last-child td { border-bottom: 0; }
  @container details (max-width: 760px) { .two { grid-template-columns: 1fr; } table { font-size: 11.5px; } }
</style>
