<script lang="ts">
  /// Who governs an org: who may change its policies and its identity, who
  /// approves a change, and what each role can do as a result. The second
  /// approver is live; the lock (security admins only) and the approver role
  /// are a preview until the server enforces them.
  let { org, myRole, infosec, fourEyes, busy, onFourEyes }: {
    org: string; myRole: string; infosec: boolean; fourEyes: boolean; busy: boolean; onFourEyes: () => void;
  } = $props();

  const canGovern = $derived(infosec || myRole === 'securityadmin');
  let policyLock = $state<'both' | 'security'>('both');
  let identityLock = $state<'both' | 'security'>('both');
  let approver = $state('securityadmin');

  const Y = '✓';
  const matrix = $derived([
    { what: 'See sign-in, groups and people', sa: Y, sec: Y, osa: Y, oa: Y, m: 'themselves' },
    { what: 'Change sign-in, SCIM, group mapping', sa: Y, sec: Y, osa: Y, oa: identityLock === 'both' ? Y : 'propose', m: '—' },
    { what: 'Read rules and decisions, try rules', sa: Y, sec: Y, osa: Y, oa: Y, m: 'rules on them' },
    { what: 'Write rules and policy files', sa: Y, sec: Y, osa: Y, oa: policyLock === 'both' ? Y : 'propose', m: '—' },
    { what: 'Approve a change', sa: Y, sec: Y, osa: Y, oa: approver === 'admin' ? Y : '—', m: '—' },
    { what: 'This page (who governs)', sa: Y, sec: Y, osa: Y, oa: 'read', m: '—' },
    { what: "Platform sign-in and built-in rules", sa: Y, sec: Y, osa: 'read', oa: '—', m: '—' },
  ]);
</script>

<div class="gs">
  <div class="note">{#if canGovern}You can change this page: you are {infosec ? 'platform InfoSec' : `${org}'s security admin`}. {org}'s admins can read it.{:else}You can read this page. {org}'s security admins and the platform's SuperAdmin and SecurityAdmin change it.{/if}</div>

  <div class="two">
    <div class="card">
      <div class="card-head"><h3>Who may change policies</h3><span class="tag sample">preview</span></div>
      <label class="radio"><input type="radio" bind:group={policyLock} value="both" disabled={!canGovern} /><span><b>Org admins and org security admins</b><small>rules, roles, access files, calls out, agents' own policies</small></span></label>
      <label class="radio"><input type="radio" bind:group={policyLock} value="security" disabled={!canGovern} /><span><b>Org security admins only</b><small>admins still read every rule, try it, and propose a change that a security admin approves</small></span></label>
    </div>
    <div class="card">
      <div class="card-head"><h3>Who may change identity</h3><span class="tag sample">preview</span></div>
      <label class="radio"><input type="radio" bind:group={identityLock} value="both" disabled={!canGovern} /><span><b>Org admins and org security admins</b><small>the org's provider, SCIM, groups → roles, members</small></span></label>
      <label class="radio"><input type="radio" bind:group={identityLock} value="security" disabled={!canGovern} /><span><b>Org security admins only</b></span></label>
    </div>
  </div>

  <div class="card">
    <div class="card-head"><h3>Approving changes</h3></div>
    <dl class="kv">
      <dt>Second approver</dt><dd class="row"><span class="dot" class:ok={fourEyes}></span>{fourEyes ? 'on: every policy change waits for another person' : 'off: a change applies when saved'}{#if canGovern}<button class="btn2" disabled={busy} onclick={onFourEyes}>{fourEyes ? 'Turn off…' : 'Turn on'}</button>{/if}</dd>
      <dt>Who approves <span class="tag sample">preview</span></dt><dd><select class="tin" bind:value={approver} disabled={!canGovern}><option value="securityadmin">an org security admin</option><option value="admin">an org admin or security admin</option></select></dd>
      <dt>Own changes</dt><dd>never approved by their author</dd>
      <dt>Platform roles</dt><dd>SuperAdmin and SecurityAdmin can always change {org}'s identity and policies; {org} can't lock them out, and their changes are shown to {org}'s security admins.</dd>
    </dl>
  </div>

  <div class="card">
    <div class="card-head"><h3>Who can do what in {org}</h3><span>with the choices above</span></div>
    <table>
      <thead><tr><th></th><th>SuperAdmin</th><th>SecurityAdmin</th><th>Org security admin</th><th>Org admin</th><th>Member</th></tr></thead>
      <tbody>{#each matrix as x (x.what)}<tr><td>{x.what}</td>{#each [x.sa, x.sec, x.osa, x.oa, x.m] as v, i (i)}<td class:y={v === Y} class:p={v === 'propose'}>{v}</td>{/each}</tr>{/each}</tbody>
    </table>
  </div>
  <p class="dim small"><span class="tag sample">preview</span> These choices aren't kept or enforced yet: today org admins and org security admins can both change everything.</p>
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
  .tin { height: 30px; padding: 0 8px; border: 1px solid var(--border); border-radius: 7px; background: var(--bg-surface); font: 12.5px var(--font-sans); color: var(--text-1); }
  table { width: 100%; border-collapse: collapse; font-size: 12.5px; }
  th { font: 500 10.5px var(--font-sans); letter-spacing: 0.05em; text-transform: uppercase; color: var(--text-4); padding: 6px; border-bottom: 1px solid var(--border); text-align: center; }
  td { padding: 7px 6px; border-bottom: 1px solid var(--border); text-align: center; color: var(--text-4); }
  td:first-child, th:first-child { text-align: left; color: var(--text-2); }
  td.y { color: var(--ok, #1a7158); font-weight: 600; }
  td.p { color: var(--warn, #b45309); }
  tr:last-child td { border-bottom: 0; }
  @container details (max-width: 760px) { .two { grid-template-columns: 1fr; } table { font-size: 11.5px; } }
</style>
