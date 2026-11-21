<script lang="ts">
  /// One rule, on one screen: say it in words (or start from a pattern), then
  /// each part of the sentence is a choice; the rule is read back, checked
  /// against the org's other files, replayed against the last 7 days, and saved
  /// into a policy file (shown, so it's clear where it lives). YAML and Cedar
  /// are one click away for experts. Parts A3T can't check yet are marked
  /// "not yet": the rule can be tried here, but not saved.
  import * as YAML from 'yaml';
  import { untrack } from 'svelte';
  import { orgPolicy, type OrgPolicyFiles, type ReplayResult } from '$lib/services/api';
  import { draftPolicy } from '../policyAgent';
  import {
    type Rule, type Cond, type Effect, blankRule, CONDS, GROUPS, condDef, newCond, condSay, sentence, notCovered, notYet,
    fileYaml, toYamlRule, rulesOf, cedarPreview, PATTERNS, fromPattern, slug,
  } from './model';

  let { org, files, projects, roles, perms, canEdit, target, onclose, onsaved }: {
    org: string; files: OrgPolicyFiles | null; projects: string[]; roles: string[];
    perms: { id: string; label: string }[]; canEdit: boolean;
    /// An existing rule (its file and index) or null for a new one.
    target: { path: string; index: number } | null;
    onclose: () => void; onsaved: (msg: string) => void;
  } = $props();

  const parse = (t: string): any => { try { return YAML.parse(t) ?? {}; } catch { return {}; } };
  const label = (id: string) => perms.find((p) => p.id === id)?.label ?? (id.startsWith('tools.') ? `call ${id.slice(6)}` : id);

  // The file it lives in, and the rule.
  // Read once: the editor is re-made for each rule (keyed by its file and index).
  const existing = untrack(() => (target ? files?.files.find((f) => f.path === target.path) ?? null : null));
  const existingY = existing ? parse(existing.content) : null;
  const start = untrack(() => (existingY ? rulesOf(existingY)[target!.index] ?? blankRule() : blankRule()));
  let r = $state<Rule>(start);
  let path = $state(existing?.path ?? '');
  let fileName = $state(existingY?.name ?? '');

  const autoPath = $derived(`policies/${slug(fileName || r.reason || r.doing.join(' ') || 'new-rule')}.yaml`);
  const savePath = $derived(path || autoPath);
  const policyFiles = $derived((files?.files ?? []).filter((f) => f.kind === 'policy'));
  const ruleCountIn = (p: string) => rulesOf(parse(files?.files.find((f) => f.path === p)?.content ?? '')).length;

  // ── Words ──
  const say = $derived(sentence(r, label));
  const notCov = $derived(notCovered(r, roles));
  const later = $derived(notYet(r));

  // ── Describe it (the policy agent) ──
  let desc = $state('');
  let drafting = $state(false);
  let draftNote = $state('');
  async function draft() {
    drafting = true; draftNote = '';
    const res = await draftPolicy(org, desc, { projects, files: (files?.files ?? []).map((f) => f.path), roles });
    drafting = false;
    if (!res.ok) { draftNote = res.error; return; }
    const f = res.draft.files[0]; const y = parse(f.yaml); const got = rulesOf(y)[0];
    if (!got) { draftNote = 'The agent answered with a rule this page can\'t show; open it under Files.'; return; }
    r = got; fileName = y.name ?? ''; path = '';
    draftNote = `Filled from your words${res.draft.assumptions.length ? `; assumed: ${res.draft.assumptions.join('; ')}` : ''}. Check each part below.`;
  }
  let showPatterns = $state(false);
  function usePattern(id: string) { const p = PATTERNS.find((x) => x.id === id); if (p) { r = fromPattern(p); fileName = slug(p.title); path = ''; draftNote = `Started from "${p.title}". Change any part.`; } }

  // ── Parts ──
  const EFFECTS: { id: Effect; label: string; hint: string }[] = [
    { id: 'block', label: 'Blocks', hint: 'the call is refused' },
    { id: 'ask', label: 'Asks someone', hint: 'the call waits for a person to approve' },
    { id: 'only', label: 'Allows only when…', hint: 'refused (or asks) unless the conditions hold' },
    { id: 'log', label: 'Only logs', hint: 'nothing is refused; what it would refuse is logged' },
  ];
  const toggle = (xs: string[], x: string) => (xs.includes(x) ? xs.filter((y) => y !== x) : [...xs, x]);
  let addDoing = $state('');
  let toolGlob = $state('');
  function addAction() { if (addDoing && !r.doing.includes(addDoing)) r.doing = [...r.doing, addDoing]; addDoing = ''; }
  function addTool() { const g = toolGlob.trim(); if (g) { r.doing = [...r.doing, `tools.${g}`]; toolGlob = ''; } }
  const permGroups = $derived(Object.entries(perms.reduce((m: Record<string, { id: string; label: string }[]>, p) => {
    const k = p.id.split('.')[0]; (m[k] ??= []).push(p); return m; }, {})));

  // Conditions: a menu to add; a chip opens its fields.
  let menu = $state<'' | 'conds' | 'unless'>('');
  let editing = $state<{ list: 'conds' | 'unless'; i: number } | null>(null);
  function add(list: 'conds' | 'unless', kind: string) {
    r[list] = [...r[list], newCond(kind)]; menu = ''; editing = { list, i: r[list].length - 1 };
  }
  function remove(list: 'conds' | 'unless', i: number) { r[list] = r[list].filter((_, j) => j !== i); editing = null; }

  // ── Check and replay ──
  let errors = $state<string[] | null>(null);
  let replay = $state<ReplayResult | null>(null);
  let checking = $state(false);
  function content(): string {
    if (existingY && path === existing?.path) {
      const others = (existingY.rules ?? []).map((x: any, i: number) => (i === target!.index ? toYamlRule(r) : x));
      const doc = { ...existingY, rules: others };
      doc.applies_to = r.projects.length ? { projects: [...r.projects] } : { org: true };
      return YAML.stringify(doc, { lineWidth: 100 });
    }
    if (path) {
      const f = policyFiles.find((x) => x.path === path); const y = parse(f?.content ?? '');
      return fileYaml(y.name ?? slug(path), y.description ?? '', r, y.rules ?? []);
    }
    return fileYaml(slug(fileName || autoPath.replace(/^policies\/|\.yaml$/g, '')), r.reason, r);
  }
  const yamlText = $derived.by(() => { void JSON.stringify(r); void path; void fileName; return content(); });
  async function check() {
    if (later.length) return;
    checking = true; errors = null; replay = null;
    const files_ = [{ path: savePath, content: content() }];
    const [v, p] = await Promise.all([orgPolicy.validate(org, files_), orgPolicy.replay(org, { files: files_, days: 7 })]);
    checking = false;
    errors = v.ok ? (v.data?.ok ? [] : v.data?.errors ?? ['it does not check']) : [v.error ?? `HTTP ${v.status}`];
    replay = p.ok ? p.data ?? null : null;
  }
  // Check as the rule changes (debounced), when it can be checked.
  let timer: ReturnType<typeof setTimeout> | undefined;
  $effect(() => {
    void yamlText;
    if (!r.doing.length || later.length || !r.reason.trim()) { errors = null; replay = null; return; }
    clearTimeout(timer); timer = setTimeout(check, 700);
    return () => clearTimeout(timer);
  });

  // ── Save ──
  let busy = $state('');
  let saveErr = $state('');
  async function save(mode: 'shadow' | 'enforce') {
    busy = mode; saveErr = '';
    const p = savePath; const body = content();
    const s = await orgPolicy.save(org, p, body);
    if (!s.ok) { busy = ''; saveErr = ((s as any).detail?.errors ?? [s.error ?? `HTTP ${s.status}`]).join('; '); return; }
    if ((s.data as any)?.pending) { busy = ''; onsaved((s.data as any).message ?? 'Held for a second approver (Changes).'); return; }
    const wantShadow = mode === 'shadow' || r.effect === 'log';
    if (wantShadow) {
      const all = await orgPolicy.files(org);
      const on = (all.data?.files ?? []).filter((f) => f.enforced && f.path !== p).map((f) => f.path);
      const e = await orgPolicy.setEnforced(org, on);
      if (!e.ok) { busy = ''; saveErr = `Saved, but still enforced: ${((e as any).detail?.errors ?? [e.error]).join('; ')}`; return; }
      if ((e.data as any)?.pending) { busy = ''; onsaved(`Saved; switching it to shadow waits for a second approver.`); return; }
      const sh = await orgPolicy.setShadow(org, p, true);
      busy = '';
      if (!sh.ok) { saveErr = `Saved and switched off, but not in shadow: ${((sh as any).detail?.errors ?? [sh.error]).join('; ')}`; return; }
      onsaved(`Saved in ${p}, running in shadow: what it would refuse goes to the decision log.`);
    } else {
      busy = '';
      onsaved(`Saved in ${p} and enforced.`);
    }
  }

  let showCode = $state(false);
  const cedar = $derived(cedarPreview(`${org}/${slug(fileName || 'new-rule')}/r${(target?.index ?? 0) + 1}`, r));
</script>

<div class="re">
  <div class="row between">
    <button class="link" onclick={onclose}>← All rules</button>
    <span class="dim small">{target ? `Rule ${target.index + 1} of ${existing?.path}` : 'New rule'}</span>
  </div>

  {#if !target}
    <div class="card">
      <div class="card-head"><h3>Describe it</h3><span>in your words, and the parts below fill in</span></div>
      <div class="desc">
        <textarea rows="2" bind:value={desc} placeholder="Operators in acme-p1 may delete flows only on weekdays 9 to 6, otherwise an Admin approves." disabled={!canEdit}></textarea>
        <button class="btn2 primary" disabled={!canEdit || drafting || desc.trim().length < 8} onclick={draft}>{drafting ? 'Filling…' : 'Fill in'}</button>
      </div>
      <div class="chips pats">
        <span class="dim small">or start from</span>
        {#each (showPatterns ? PATTERNS : PATTERNS.slice(0, 5)) as p (p.id)}<button title={p.what} onclick={() => usePattern(p.id)}>{p.title}</button>{/each}
        <button class="link" onclick={() => (showPatterns = !showPatterns)}>{showPatterns ? 'fewer' : `${PATTERNS.length - 5} more`}</button>
      </div>
      {#if draftNote}<p class="dim">{draftNote}</p>{/if}
    </div>
  {/if}

  <div class="card">
    <div class="card-head"><h3>The rule</h3></div>
    <div class="parts">
      <span class="pl">This rule</span>
      <div><div class="seg">{#each EFFECTS as e (e.id)}<button class:on={r.effect === e.id} title={e.hint} onclick={() => (r.effect = e.id)}>{e.label}</button>{/each}</div>
        <div class="dim small">{EFFECTS.find((e) => e.id === r.effect)?.hint}</div></div>

      <span class="pl">Who</span>
      <div class="chips">
        {#each roles as x (x)}<button class:on={r.roles.includes(x)} onclick={() => (r.roles = toggle(r.roles, x))}>{x}</button>{/each}
        <select class="tin sm" bind:value={r.callers} aria-label="People or agents">
          <option value="both">people and agents</option><option value="agents">agents only</option><option value="people">people only</option>
        </select>
        {#if !r.roles.length}<span class="dim small">no role picked: anyone</span>{/if}
      </div>

      <span class="pl">Doing</span>
      <div class="chips">
        {#each r.doing as d (d)}<span class="pick">{label(d)}<button aria-label="Remove" onclick={() => (r.doing = r.doing.filter((x) => x !== d))}>×</button></span>{/each}
        <select class="tin sm" bind:value={addDoing} onchange={addAction} aria-label="Add an action">
          <option value="">+ action…</option>
          {#each permGroups as [g, ps] (g)}<optgroup label={g}>{#each ps as p (p.id)}<option value={p.id}>{p.label}</option>{/each}</optgroup>{/each}
        </select>
        <input class="tin sm mono" style="width:130px" bind:value={toolGlob} placeholder="+ tool: delete_*" onkeydown={(e) => { if (e.key === 'Enter') addTool(); }} aria-label="Add a tool pattern" />
      </div>

      <span class="pl">Where</span>
      <div class="chips">
        <button class:on={!r.projects.length} onclick={() => (r.projects = [])}>whole org</button>
        {#each projects as x (x)}<button class:on={r.projects.includes(x)} onclick={() => (r.projects = toggle(r.projects, x))}>{x}</button>{/each}
      </div>

      <span class="pl">{r.effect === 'only' ? 'Only when' : 'When'}</span>
      <div class="chips rel">
        {#each r.conds as c, i (i)}
          <span class="pick cond" class:later={!condDef(c.kind)?.today} class:sel={editing?.list === 'conds' && editing.i === i}>
            <button class="ct" onclick={() => (editing = editing?.list === 'conds' && editing.i === i ? null : { list: 'conds', i })}>{condSay(c)}</button>
            <button aria-label="Remove" onclick={() => remove('conds', i)}>×</button></span>
        {/each}
        <button class="addc" onclick={() => (menu = menu === 'conds' ? '' : 'conds')}>+ condition</button>
        {#if !r.conds.length}<span class="dim small">{r.effect === 'only' ? 'add what must hold' : 'always'}</span>{/if}
        {#if menu === 'conds'}{@render condMenu('conds')}{/if}
      </div>
      {#if editing?.list === 'conds' && r.conds[editing.i]}{@render condFields(r.conds[editing.i])}{/if}

      {#if r.effect === 'only'}
        <span class="pl">Otherwise</span>
        <div class="chips"><div class="seg"><button class:on={r.otherwise === 'ask'} onclick={() => (r.otherwise = 'ask')}>someone approves</button><button class:on={r.otherwise === 'block'} onclick={() => (r.otherwise = 'block')}>refused</button></div></div>
      {/if}
      {#if r.effect === 'ask' || (r.effect === 'only' && r.otherwise === 'ask')}
        <span class="pl">Approver</span>
        <div class="chips">
          <select class="tin sm" bind:value={r.askFrom} aria-label="Who approves"><option value="">the person the agent acts for</option>{#each roles as x (x)}<option value={x}>an {x}</option>{/each}</select>
          <span class="dim small">within</span><select class="tin sm" bind:value={r.timeout} aria-label="Timeout">{#each ['30m', '1h', '4h', '1d', '2d'] as t (t)}<option>{t}</option>{/each}</select>
        </div>
      {/if}

      <span class="pl">Except</span>
      <div class="chips rel">
        {#each r.unless as c, i (i)}
          <span class="pick cond exc" class:later={!condDef(c.kind)?.today} class:sel={editing?.list === 'unless' && editing.i === i}>
            <button class="ct" onclick={() => (editing = editing?.list === 'unless' && editing.i === i ? null : { list: 'unless', i })}>{condSay(c)}</button>
            <button aria-label="Remove" onclick={() => remove('unless', i)}>×</button></span>
        {/each}
        <button class="addc" onclick={() => (menu = menu === 'unless' ? '' : 'unless')}>+ exception</button>
        {#if menu === 'unless'}{@render condMenu('unless')}{/if}
      </div>
      {#if editing?.list === 'unless' && r.unless[editing.i]}{@render condFields(r.unless[editing.i])}{/if}

      <span class="pl">Why</span>
      <input class="tin" bind:value={r.reason} placeholder="Shown to whoever is refused, and to the approver" />

      <span class="pl">Ends</span>
      <div class="chips"><input class="tin sm" type="date" bind:value={r.until} aria-label="Ends on" />{#if r.until}<button class="link" onclick={() => (r.until = '')}>no end</button>{:else}<span class="dim small">no end</span>{/if}</div>
    </div>

    <div class="readback">
      {#if say}<div class="say">{say}</div>{#if notCov}<div class="dim small">{notCov}</div>{/if}
      {:else}<div class="dim">Pick what it is about (Doing), and the sentence appears here.</div>{/if}
    </div>
  </div>

  {#if later.length}
    <div class="note warn"><b>Preview only.</b> A3T can't check these yet, so this rule can be tried here but not saved: {later.join('; ')}.</div>
  {/if}

  <div class="card">
    <div class="card-head"><h3>Check</h3><span class="grow"></span><button class="link" onclick={() => (showCode = !showCode)}>{showCode ? 'Hide' : 'Show'} YAML and Cedar</button></div>
    {#if later.length}<p class="dim">Checked once A3T can check every part.</p>
    {:else if !r.doing.length || !r.reason.trim()}<p class="dim">Pick what it is about and say why; it is checked as you go.</p>
    {:else if checking}<p class="dim">Checking with {org}'s other files and replaying the last 7 days…</p>
    {:else if errors?.length}<div class="note bad">{#each errors as e (e)}<div class="mono">{e}</div>{/each}</div>
    {:else if errors}
      <p class="ok">✓ Works with {org}'s other files.</p>
      {#if replay}<p class="dim">Last {replay.days} days: {replay.replayed} decisions replayed, <b>{replay.flipped} would change</b>.</p>
        {#if replay.flips.length}<ul class="tl">{#each replay.flips.slice(0, 8) as x (x.ts_ms + x.resource)}<li class="ev"><span class="ic">→</span><div><div class="t">{x.principal}{x.on_behalf_of ? ` for ${x.on_behalf_of}` : ''} · <span class="mono">{x.action} {x.resource}</span></div><div class="m">{x.before} → {x.after}</div></div><time>{new Date(x.ts_ms).toLocaleString()}</time></li>{/each}</ul>{/if}{/if}
    {/if}
    {#if showCode}
      <div class="code2">
        <div><div class="dim small">YAML, saved in <span class="mono">{savePath}</span></div><pre class="code">{yamlText}</pre></div>
        <div><div class="dim small">Cedar (preview; A3T compiles it at save)</div><pre class="code">{cedar}</pre></div>
      </div>
    {/if}
  </div>

  <div class="card foot">
    <div class="filerow">
      <span class="pl">Saved in</span>
      {#if target}
        <span class="mono">{existing?.path}</span><span class="dim small">· a policy file with {ruleCountIn(existing?.path ?? '')} rule{ruleCountIn(existing?.path ?? '') === 1 ? '' : 's'}; Where applies to all of them</span>
      {:else}
        <select class="tin sm" bind:value={path} aria-label="File">
          <option value="">a new file</option>
          {#each policyFiles as f (f.path)}<option value={f.path}>add to {f.path}</option>{/each}
        </select>
        {#if !path}<input class="tin sm mono" bind:value={fileName} placeholder={autoPath.replace(/^policies\/|\.yaml$/g, '')} aria-label="File name" /><span class="dim small mono">→ {autoPath}</span>{/if}
      {/if}
    </div>
    <p class="dim small">Every rule lives in a policy file of the org (Files). A file is switched on, off or to shadow as a whole, and each save is a version.</p>
    {#if saveErr}<div class="note bad">{saveErr}</div>{/if}
    <div class="acts">
      {#if canEdit}
        <button class="btn2 primary" disabled={!!busy || !!later.length || !!errors?.length || errors === null} onclick={() => save('shadow')}>{busy === 'shadow' ? 'Saving…' : 'Save in shadow'}</button>
        {#if r.effect !== 'log'}<button class="btn2" disabled={!!busy || !!later.length || !!errors?.length || errors === null} onclick={() => save('enforce')}>{busy === 'enforce' ? 'Saving…' : 'Save and enforce'}</button>{/if}
      {:else}<span class="dim">You can read and try this rule; changing {org}'s rules is for its security admins.</span>{/if}
      <button class="btn2 quiet" onclick={onclose}>Cancel</button>
    </div>
  </div>
</div>

{#snippet condMenu(list: 'conds' | 'unless')}
  <div class="menu" role="menu">
    {#each GROUPS as g (g)}
      {@const items = CONDS.filter((c) => c.group === g && (list === 'conds' || ['Who exactly', 'From where', 'When'].includes(g)))}
      {#if items.length}
        <div class="mg"><div class="mh">{g}</div>
          {#each items as c (c.kind)}<button role="menuitem" onclick={() => add(list, c.kind)}>{c.label}{#if !c.today}<span class="tag sample">not yet</span>{/if}</button>{/each}
        </div>
      {/if}
    {/each}
  </div>
{/snippet}

{#snippet condFields(c: Cond)}
  {@const d = condDef(c.kind)}
  <span></span>
  <div class="cf">
    {#each d?.params ?? [] as prm (prm.k)}
      <label>{prm.label}
        {#if prm.opts}<select class="tin sm" bind:value={c.p[prm.k]}>{#each prm.opts as o (o)}<option>{o}</option>{/each}</select>
        {:else}<input class="tin sm" class:mono={prm.mono} bind:value={c.p[prm.k]} />{/if}
      </label>
    {:else}<span class="dim small">Nothing to set.</span>{/each}
    {#if d && !d.today}<span class="dim small">A3T needs to pass {d.needs} for this.</span>{/if}
  </div>
{/snippet}

<style>
  .re { display: flex; flex-direction: column; gap: 14px; }
  .small { font-size: 11.5px; }
  .desc { display: flex; gap: 8px; align-items: flex-start; }
  .desc textarea { flex: 1; min-height: 52px; padding: 8px 10px; border: 1px solid var(--border); border-radius: 8px; font: 13px var(--font-sans); background: var(--bg-surface); color: var(--text-1); resize: vertical; }
  .pats { gap: 6px; }
  .parts { display: grid; grid-template-columns: 92px 1fr; gap: 10px 12px; align-items: start; }
  .pl { font: 600 10.5px var(--font-sans); letter-spacing: 0.06em; text-transform: uppercase; color: var(--text-4); padding-top: 8px; }
  .tin { height: 32px; padding: 0 10px; border: 1px solid var(--border); border-radius: 8px; background: var(--bg-surface); font: 13px var(--font-sans); color: var(--text-1); min-width: 0; }
  .tin.sm { height: 28px; font-size: 12.5px; padding: 0 8px; width: auto; flex: 0 0 auto; }
  .re .chips .tin.sm, .re .cf .tin.sm, .re .filerow .tin.sm { width: auto; max-width: 220px; }
  .mg :global(.tag) { white-space: nowrap; flex: none; }
  .chips { display: flex; flex-wrap: wrap; gap: 6px; align-items: center; }
  .chips > button:not(.link):not(.addc) { border: 1px solid var(--border); background: var(--bg-surface); border-radius: 999px; padding: 4px 11px; font-size: 12px; color: var(--text-2); cursor: pointer; }
  .chips > button.on:not(.link):not(.addc) { background: var(--text-1); color: var(--bg-page); border-color: var(--text-1); }
  .pick { display: inline-flex; align-items: center; gap: 4px; border: 1px solid var(--border); background: var(--bg-surface); border-radius: 999px; padding: 3px 4px 3px 11px; font-size: 12px; color: var(--text-1); }
  .pick > button { border: 0; background: none; color: var(--text-4); cursor: pointer; font-size: 13px; padding: 0 4px; }
  .pick .ct { color: var(--text-1); font-size: 12px; padding: 0; }
  .pick.cond { background: var(--spec-bg, #f4f1fd); border-color: color-mix(in srgb, var(--spec) 30%, var(--border)); }
  .pick.exc { background: var(--warn-bg, #fbf0e0); border-color: color-mix(in srgb, var(--warn) 35%, var(--border)); }
  .pick.later { border-style: dashed; }
  .pick.sel { box-shadow: 0 0 0 2px color-mix(in srgb, var(--spec) 35%, transparent); }
  .addc { border: 1px dashed color-mix(in srgb, var(--spec) 50%, var(--border)); background: none; color: var(--spec); border-radius: 999px; padding: 4px 11px; font-size: 12px; cursor: pointer; }
  .rel { position: relative; }
  .menu { position: absolute; top: 100%; left: 0; z-index: 20; margin-top: 6px; width: min(640px, 100%); max-height: 380px; overflow: auto; display: grid; grid-template-columns: repeat(auto-fill, minmax(190px, 1fr)); gap: 10px; padding: 12px; background: var(--bg-surface); border: 1px solid var(--border); border-radius: 10px; box-shadow: 0 8px 24px rgba(0,0,0,.12); }
  .mg { display: flex; flex-direction: column; gap: 2px; }
  .mh { font: 600 10.5px var(--font-sans); letter-spacing: 0.06em; text-transform: uppercase; color: var(--text-4); margin-bottom: 2px; }
  .mg button { text-align: left; border: 0; background: none; padding: 4px 6px; border-radius: 6px; font-size: 12.5px; color: var(--text-1); cursor: pointer; display: flex; gap: 6px; align-items: center; justify-content: space-between; }
  .mg button:hover { background: var(--bg-subtle); }
  .cf { display: flex; flex-wrap: wrap; gap: 10px; align-items: flex-end; padding: 8px 10px; border: 1px solid var(--border); border-radius: 8px; background: var(--bg-subtle); }
  .cf label { display: flex; flex-direction: column; gap: 3px; font-size: 11px; color: var(--text-3); }
  .readback { margin-top: 4px; padding: 10px 12px; border: 1px solid var(--border); border-radius: 8px; background: var(--bg-subtle); display: flex; flex-direction: column; gap: 4px; }
  .say { font: 400 16px/1.4 var(--font-serif); color: var(--text-1); }
  .code2 { display: grid; grid-template-columns: 1fr 1.2fr; gap: 10px; }
  .code2 .code { margin-top: 4px; font-size: 11.5px; max-height: 360px; overflow: auto; }
  .filerow { display: flex; flex-wrap: wrap; gap: 8px; align-items: center; }
  .filerow .pl { padding-top: 0; }
  @container details (max-width: 760px) { .parts { grid-template-columns: 1fr; gap: 4px; } .pl { padding-top: 6px; } .code2 { grid-template-columns: 1fr; } .desc { flex-direction: column; } }
</style>
