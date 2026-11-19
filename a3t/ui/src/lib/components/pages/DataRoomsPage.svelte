<script lang="ts">
  /// Files: where agents read and write files — storage resources (S3, MinIO,
  /// GCS, Azure, FTP). Items lists them; the one picked opens here in the agents'
  /// frame: Overview (where it points, its credentials, who uses it), Files (its
  /// contents through its own driver: a bucket's prefixes and objects, or an FTP
  /// server's directories; a file's first 64 KiB), Used by (agents, canvases,
  /// triggers and the flows that name it), Configuration (the driver's fields,
  /// credentials as Secret references only). "New storage" makes one.
  import { untrack } from 'svelte';
  import SlidePanel from '$lib/components/shared/SlidePanel.svelte';
  import PropertiesTree from '$lib/components/shared/PropertiesTree.svelte';
  import WizardHead from '$lib/components/shared/WizardHead.svelte';
  import '$lib/components/agent/agent.css';
  import '$lib/components/triggers/triggers.css';
  import { resources as resourcesMcp, storageApi, mcpCall } from '$lib/services/api';
  import { getActivePanel, navigateTo, navigateToDetail } from '$lib/stores/panels.svelte.ts';
  import { namespacesFor, getProject } from '$lib/stores/context.svelte.ts';
  import { bumpItems } from '$lib/stores/apps.svelte.ts';
  import { usedBy, type Use } from '$lib/components/resources/resourceUse';

  type Driver = 's3' | 'minio' | 'gcs' | 'azure' | 'ftp';
  const DRIVERS: { d: Driver; label: string; what: string }[] = [
    { d: 's3', label: 'S3', what: 'an Amazon S3 bucket' },
    { d: 'minio', label: 'MinIO', what: 'an S3-compatible bucket at your endpoint' },
    { d: 'gcs', label: 'Google Cloud', what: 'a Google Cloud Storage bucket' },
    { d: 'azure', label: 'Azure', what: 'an Azure Blob container' },
    { d: 'ftp', label: 'FTP', what: 'a directory on an FTP server' },
  ];
  /// The fields each driver takes: [key, label, hint, kind]. "secret" fields are
  /// references to Secret resources — a key is never stored with the storage.
  const FIELDS: Record<Driver, [string, string, string, 'text' | 'number' | 'bool' | 'secret'][]> = {
    s3: [['bucket', 'Bucket', '', 'text'], ['region', 'Region', 'e.g. eu-west-1', 'text'], ['prefix', 'Prefix', 'only under this folder', 'text'],
      ['access_key_id_ref', 'Access key id', 'a Secret', 'secret'], ['secret_access_key_ref', 'Secret access key', 'a Secret', 'secret'], ['anonymous', 'Public bucket', 'read without keys', 'bool']],
    minio: [['endpoint', 'Endpoint', 'http://minio.example:9000', 'text'], ['bucket', 'Bucket', '', 'text'], ['region', 'Region', 'us-east-1 if unsure', 'text'], ['prefix', 'Prefix', 'only under this folder', 'text'],
      ['access_key_id_ref', 'Access key', 'a Secret', 'secret'], ['secret_access_key_ref', 'Secret key', 'a Secret', 'secret']],
    gcs: [['bucket', 'Bucket', '', 'text'], ['prefix', 'Prefix', 'only under this folder', 'text'], ['service_account_ref', 'Service account key', 'a Secret holding the key JSON', 'secret'], ['anonymous', 'Public bucket', 'read without a key', 'bool']],
    azure: [['account', 'Storage account', '', 'text'], ['container', 'Container', '', 'text'], ['prefix', 'Prefix', 'only under this folder', 'text'], ['access_key_ref', 'Access key', 'a Secret', 'secret']],
    ftp: [['host', 'Host', 'ftp.example.com', 'text'], ['port', 'Port', '21', 'number'], ['user', 'User', 'anonymous if empty', 'text'], ['password_ref', 'Password', 'a Secret', 'secret'], ['root', 'Root directory', '/ if empty', 'text']],
  };

  type Store = { ns: string; name: string; ref: string; type: Driver; config: Record<string, any> };
  let st = $state<Store | null>(null);
  let creating = $state(false);
  let tab = $state('overview');
  let secrets = $state<string[]>([]);
  let uses = $state<Use[] | null>(null);
  let flowsUsing = $state<{ ref: string }[] | null>(null);

  let shown = '';
  $effect(() => {
    const p = getActivePanel()?.props ?? {};
    const id = p.selectedId as string | undefined;
    const create = p.create;
    untrack(() => {
      if (create && !id) { openCreate(); return; }
      if (id && id !== shown) { shown = id; void open(id); } else if (!id) { shown = ''; st = null; }
    });
  });
  async function loadSecrets() {
    const lists = await Promise.all(namespacesFor('picker').map((ns) => mcpCall('list_resources', { namespace: ns })));
    secrets = lists.flatMap((r) => (r.ok ? (r.data?.resources ?? []) : [])).filter((r: any) => ['secret', 'credential'].includes(String(r.type).toLowerCase())).map((r: any) => `${r.namespace}/${r.name}`).sort();
  }
  async function open(id: string) {
    creating = false;
    const [ns, ...rest] = id.split('~'); const name = rest.join('~');
    tab = 'overview'; uses = null; flowsUsing = null; listing = null; listErr = ''; prefix = ''; previewOf = null; cfgMsg = '';
    const r = await resourcesMcp.get(name, ns);
    const d: any = r.ok ? r.data : {};
    st = { ns, name, ref: `${ns}/${name}`, type: (String(d.type ?? d.type_ref ?? 's3').toLowerCase() as Driver), config: d.config ?? {} };
    draft = { ...st.config };
    void loadSecrets();
    usedBy(st.ref).then((u) => { if (st?.ref === `${ns}/${name}`) uses = u; });
  }
  const close = () => navigateTo('datarooms', 'Files');

  // ── Files: one level at a time ──
  let prefix = $state('');
  let listing = $state<any>(null);
  let listErr = $state('');
  let listing_ = $state(false);
  async function list(p = prefix) {
    if (!st) return;
    prefix = p; listing_ = true; listErr = ''; previewOf = null;
    const r = await storageApi.list(st.ns, st.name, p);
    listing_ = false;
    if (r.ok) listing = r.data; else { listing = null; listErr = (r as any).data?.error ?? r.error ?? 'It could not be listed.'; }
  }
  $effect(() => { if (tab === 'files' && st && !listing && !listErr) untrack(() => void list('')); });
  const crumbs = $derived(prefix.split('/').filter(Boolean).map((part, i, all) => ({ part, path: all.slice(0, i + 1).join('/') + '/' })));
  let previewOf = $state<any>(null);
  let previewing = $state('');
  async function preview(f: any) {
    if (!st) return;
    previewing = f.path;
    const r = await storageApi.preview(st.ns, st.name, f.path);
    previewing = '';
    previewOf = r.ok ? { ...r.data, name: f.name } : { name: f.name, error: (r as any).data?.error ?? r.error };
  }
  const size = (n: number) => n < 1024 ? `${n} B` : n < 1048576 ? `${(n / 1024).toFixed(1)} KB` : n < 1073741824 ? `${(n / 1048576).toFixed(1)} MB` : `${(n / 1073741824).toFixed(2)} GB`;
  const when = (iso: string) => { const d = new Date(iso); return isNaN(+d) ? '' : d.toLocaleString(undefined, { year: 'numeric', month: 'short', day: 'numeric', hour: '2-digit', minute: '2-digit' }); };

  // ── Used by: who names it, and the flows that do ──
  async function scanFlows() {
    if (!st) return;
    flowsUsing = null;
    const ref = st.ref;
    const lists = await Promise.all(namespacesFor('picker').map((ns) => mcpCall('list_flows', { namespace: ns })));
    const all = lists.flatMap((r) => (r.ok ? (r.data?.workflows ?? []) : [])).map((w: any) => ({ ns: w.namespace, name: w.name }));
    const out: { ref: string }[] = [];
    for (let i = 0; i < all.length; i += 8) {
      const got = await Promise.all(all.slice(i, i + 8).map((w) => mcpCall('get_flow', { namespace: w.ns, name: w.name }).then((g) => (typeof g.data === 'string' && g.data.includes(ref) ? `${w.ns}/${w.name}` : ''))));
      out.push(...got.filter(Boolean).map((r) => ({ ref: r })));
    }
    if (st?.ref === ref) flowsUsing = out;
  }
  $effect(() => { if (tab === 'used' && st && flowsUsing === null) untrack(() => void scanFlows()); });

  // ── Configuration ──
  let draft = $state<Record<string, any>>({});
  let cfgMsg = $state('');
  let saving = $state(false);
  const dirty = $derived(!!st && JSON.stringify(draft) !== JSON.stringify(st.config));
  async function save() {
    if (!st) return;
    saving = true; cfgMsg = '';
    const cfg = Object.fromEntries(Object.entries(draft).filter(([, v]) => v !== '' && v !== null && v !== undefined));
    const r = await resourcesMcp.update(st.ns, st.name, cfg);
    saving = false;
    if (r.ok) { st.config = cfg; draft = { ...cfg }; cfgMsg = 'Saved'; listing = null; listErr = ''; bumpItems(); } else cfgMsg = r.error ?? 'Not saved.';
  }

  // ── New storage ──
  let nType = $state<Driver>('s3');
  let nName = $state('');
  let nNs = $state(getProject() || 'default');
  let nCfg = $state<Record<string, any>>({});
  let nErr = $state('');
  function openCreate() { creating = true; st = null; shown = ''; nName = ''; nCfg = {}; nErr = ''; nType = 's3'; void loadSecrets(); }
  async function create() {
    nErr = '';
    const cfg = Object.fromEntries(Object.entries(nCfg).filter(([, v]) => v !== '' && v !== null && v !== undefined));
    const r = await resourcesMcp.create(nName.trim(), nType, cfg, nNs);
    if (!r.ok) { nErr = r.error ?? 'Not created.'; return; }
    creating = false; bumpItems();
    navigateToDetail('datarooms', 'Files', `${nNs}~${nName.trim()}`);
  }

  const steps = [
    { id: 'overview', label: 'Overview', icon: '<rect x="2" y="2" width="12" height="12" rx="2"/><path d="M5 6h6M5 9h4"/>', hint: 'Where it points, its credentials, and who uses it' },
    { id: 'files', label: 'Files', icon: '<path d="M2 4h4l1.5 1.5H14v7.5H2z"/>', hint: 'Its contents, through its own driver' },
    { id: 'used', label: 'Used by', icon: '<circle cx="4" cy="4" r="2"/><circle cx="12" cy="4" r="2"/><circle cx="8" cy="12" r="2"/><path d="M5.5 5.5L7 10.5M10.5 5.5L9 10.5"/>', hint: 'The agents, flows, canvases and triggers that name it' },
    { id: 'config', label: 'Configuration', icon: '<path d="M8 1v4M8 11v4M1 8h4M11 8h4"/><circle cx="8" cy="8" r="3"/>', hint: 'Its driver settings — credentials as Secret references only' },
  ];
  const where = (s: Store) => s.type === 'ftp' ? `ftp://${s.config.host ?? '?'}:${s.config.port ?? 21}${s.config.root ?? '/'}` : s.type === 'azure' ? `az://${s.config.account ?? '?'}/${s.config.container ?? '?'}/${s.config.prefix ?? ''}` : `${s.type === 'gcs' ? 'gs' : 's3'}://${s.config.bucket ?? '?'}/${s.config.prefix ?? ''}`;
  const creds = (s: Store) => FIELDS[s.type]?.filter((f) => f[3] === 'secret').map((f) => ({ label: f[1], ref: s.config[f[0]] as string | undefined })) ?? [];
</script>

{#snippet fieldsFor(type: Driver, cfg: Record<string, any>)}
  <div class="fields">
    {#each FIELDS[type] as [k, label, hint, kind] (k)}
      {#if kind === 'bool'}
        <label class="fld chk"><input type="checkbox" checked={!!cfg[k]} onchange={(e) => (cfg[k] = e.currentTarget.checked || undefined)} /> {label} <small>{hint}</small></label>
      {:else if kind === 'secret'}
        <label class="fld">{label} <small>{hint}</small>
          <select value={cfg[k] ?? ''} onchange={(e) => (cfg[k] = e.currentTarget.value || undefined)}>
            <option value="">— none —</option>
            {#if cfg[k] && !secrets.includes(cfg[k])}<option value={cfg[k]}>{cfg[k]} (missing)</option>{/if}
            {#each secrets as s (s)}<option value={s}>{s}</option>{/each}
          </select></label>
      {:else}
        <label class="fld">{label} {#if hint}<small>{hint}</small>{/if}<input class="mono" type={kind === 'number' ? 'number' : 'text'} value={cfg[k] ?? ''} oninput={(e) => (cfg[k] = kind === 'number' ? (e.currentTarget.value ? Number(e.currentTarget.value) : undefined) : e.currentTarget.value)} /></label>
      {/if}
    {/each}
  </div>
{/snippet}

<SlidePanel open={!!st || creating} onclose={close}>
  {#if creating}
    <div class="panel-layout">
      <PropertiesTree icon="+" color="#2f6f8f" name={nName || 'New storage'} meta="storage" nodes={[]} steps={[{ id: 'new', label: 'Storage', hint: 'Where its files are, and the Secrets that open it', icon: '<path d="M2 4h4l1.5 1.5H14v7.5H2z"/>' }]} activeNode="new" onselect={() => {}} />
      <div class="panel-col"><div class="panel-content"><div class="pf">
        <WizardHead step={{ id: 'new', label: 'New storage', hint: 'Where its files are, and the Secrets that open it' }} />
        <section class="agt">
          <div class="card">
            <div class="card-head"><h3>1 · Driver</h3><span>{DRIVERS.find((d) => d.d === nType)?.what}</span></div>
            <div class="seg src" role="tablist">{#each DRIVERS as d (d.d)}<button role="tab" class:on={nType === d.d} onclick={() => { nType = d.d; nCfg = {}; }}>{d.label}</button>{/each}</div>
          </div>
          <div class="card">
            <div class="card-head"><h3>2 · Name</h3><span>how flows and agents refer to it: project/name</span></div>
            <div class="fields">
              <label class="fld">Name<input class="mono" bind:value={nName} placeholder="contracts-bucket" /></label>
              <label class="fld">Project<select bind:value={nNs}>{#each [...new Set([nNs, ...namespacesFor('owned')].filter(Boolean))] as ns (ns)}<option value={ns}>{ns}</option>{/each}</select></label>
            </div>
          </div>
          <div class="card">
            <div class="card-head"><h3>3 · Where, and how it opens</h3><span>keys are Secrets, picked here — never typed in</span></div>
            {@render fieldsFor(nType, nCfg)}
          </div>
          {#if nErr}<div class="note bad">{nErr}</div>{/if}
          <div class="acts"><button class="btn2 primary" disabled={!nName.trim()} onclick={create}>Create storage</button><button class="btn2 quiet" onclick={close}>Cancel</button></div>
        </section>
      </div></div></div>
    </div>
  {:else if st}
    <div class="panel-layout">
      <PropertiesTree icon={st.type.slice(0, 2).toUpperCase()} color="#2f6f8f" name={st.name} meta={`${DRIVERS.find((d) => d.d === st!.type)?.label ?? st.type} · ${st.ns}`} nodes={[]} {steps} activeNode={tab} onselect={(id) => (tab = id)} />
      <div class="panel-col"><div class="panel-content"><div class="pf">
        <WizardHead step={steps.find((s) => s.id === tab)} />
        <section class="agt">
        {#if tab === 'overview'}
          <div class="card">
            <div class="card-head"><h3>1 · Where it points</h3><span>{DRIVERS.find((d) => d.d === st!.type)?.what}</span></div>
            <ul class="rows">
              <li><span class="lbl">Location</span><span class="val mono">{where(st)}</span><button class="btn2" onclick={() => (tab = 'files')}>Browse</button></li>
              <li><span class="lbl">Driver</span><span class="val">{DRIVERS.find((d) => d.d === st!.type)?.label} <span class="dim">· read only from the console</span></span><span></span></li>
              {#if st.config.region}<li><span class="lbl">Region</span><span class="val mono">{st.config.region}</span><span></span></li>{/if}
            </ul>
          </div>
          <div class="card">
            <div class="card-head"><h3>2 · Credentials</h3><span>Secret references — the keys themselves are not here</span></div>
            <ul class="rows">
              {#each creds(st) as c (c.label)}
                <li><span class="lbl">{c.label}</span><span class="val">{#if c.ref}<span class="dot" class:ok={secrets.length === 0 || secrets.includes(c.ref)} class:err={secrets.length > 0 && !secrets.includes(c.ref)}></span> <span class="mono">{c.ref}</span>{:else}<span class="dot"></span> none{/if}</span>
                  <button class="btn2" onclick={() => (tab = 'config')}>Change</button></li>
              {/each}
              {#if st.config.anonymous}<li><span class="lbl">Access</span><span class="val"><span class="dot warn"></span> public: read without keys</span><span></span></li>{/if}
            </ul>
          </div>
          <div class="card">
            <div class="card-head"><h3>3 · Used by</h3><span>who names it</span></div>
            <ul class="rows"><li><span class="lbl">Named by</span><span class="val">{uses === null ? '…' : uses.length ? `${uses.length}: ${uses.map((u) => u.name).join(', ')}` : 'no agent, canvas or trigger'}</span><button class="btn2" onclick={() => (tab = 'used')}>See</button></li></ul>
          </div>

        {:else if tab === 'files'}
          <div class="card">
            <div class="card-head"><h3>{st.type === 'ftp' ? 'Directory' : 'Bucket'}</h3><span class="mono">{listing?.location ?? where(st)}</span><span class="grow"></span><button class="btn2 quiet rm" disabled={listing_} onclick={() => list(prefix)}>Refresh</button></div>
            <nav class="crumbs"><button class="link" onclick={() => list('')}>{st.type === 'ftp' ? (st.config.root ?? '/') : (st.config.bucket ?? st.config.container ?? 'root')}</button>
              {#each crumbs as c (c.path)}<span class="dim">/</span><button class="link" onclick={() => list(c.path)}>{c.part}</button>{/each}</nav>
            {#if listing_}<p class="dim">Listing…</p>
            {:else if listErr}<div class="note bad">{listErr}</div>
            {:else if listing}
              {#if !listing.dirs.length && !listing.files.length}<p class="empty">Empty.</p>{/if}
              <ul class="flist">
                {#each listing.dirs as d (d.path)}<li><button class="row-btn" onclick={() => list(d.path)}><span class="ico">▸</span><span class="nm">{d.name}/</span><span class="dim">{st.type === 'ftp' ? 'directory' : 'prefix'}</span><span></span></button></li>{/each}
                {#each listing.files as f (f.path)}<li><button class="row-btn" class:on={previewOf?.path === f.path} onclick={() => preview(f)}><span class="ico">·</span><span class="nm">{f.name}</span><span class="dim">{size(f.size)}</span><span class="dim">{when(f.modified)}</span></button></li>{/each}
              </ul>
              {#if listing.truncated}<p class="dim">Showing the first 1,000.</p>{/if}
            {/if}
          </div>
          {#if previewing}<p class="dim">Reading {previewing}…</p>{/if}
          {#if previewOf}
            <div class="card">
              <div class="card-head"><h3>{previewOf.name}</h3><span>{previewOf.size != null ? size(previewOf.size) : ''}{previewOf.truncated ? ' · the first 64 KB' : ''}</span><span class="grow"></span><button class="btn2 quiet rm" onclick={() => (previewOf = null)}>Close</button></div>
              {#if previewOf.error}<div class="note bad">{previewOf.error}</div>
              {:else if previewOf.binary}<p class="dim">A binary file — no preview.</p>
              {:else}<pre class="code out">{previewOf.text}</pre>{/if}
            </div>
          {/if}

        {:else if tab === 'used'}
          <div class="card">
            <div class="card-head"><h3>1 · Agents, canvases, triggers</h3><span>whose settings name it</span></div>
            {#if uses === null}<p class="dim">Looking…</p>{:else if uses.length}<ul class="rows wide">{#each uses as u (u.id)}<li><span class="lbl">{u.name}</span><span class="val">{u.kind} · {u.how}</span><span></span></li>{/each}</ul>{:else}<p class="empty">None.</p>{/if}
          </div>
          <div class="card">
            <div class="card-head"><h3>2 · Flows</h3><span>whose definition names <span class="mono">{st.ref}</span></span></div>
            {#if flowsUsing === null}<p class="dim">Reading the flows…</p>
            {:else if flowsUsing.length}<ul class="rows wide">{#each flowsUsing as f (f.ref)}<li><span class="lbl mono">{f.ref}</span><span class="val"></span><button class="btn2" onclick={() => navigateToDetail('resources', 'Resources', `flows:${f.ref}`)}>Open</button></li>{/each}</ul>
            {:else}<p class="empty">No flow names it.</p>{/if}
          </div>

        {:else}
          <div class="card">
            <div class="card-head"><h3>1 · {DRIVERS.find((d) => d.d === st!.type)?.label}</h3><span>where its files are, and the Secrets that open it</span></div>
            {@render fieldsFor(st.type, draft)}
          </div>
          <div class="acts"><button class="btn2 primary" disabled={saving || !dirty} onclick={save}>{saving ? 'Saving…' : 'Save'}</button>
            <button class="btn2 quiet" disabled={saving || !dirty} onclick={() => (draft = { ...st!.config })}>Discard</button>
            {#if cfgMsg}<span class={cfgMsg === 'Saved' ? 'ok' : 'err'}>{cfgMsg}</span>{:else if dirty}<span class="dim">Not saved yet.</span>{/if}</div>
        {/if}
        </section>
      </div></div></div>
    </div>
  {/if}
</SlidePanel>

<style>
  .panel-layout { display: grid; grid-template-columns: 280px 1fr; height: 100%; }
  .panel-col { display: flex; flex-direction: column; min-height: 0; min-width: 0; height: 100%; }
  .panel-content { overflow-y: auto; flex: 1; min-height: 0; }
  .pf { padding: 24px 32px; max-width: var(--detail-w); box-sizing: border-box; }
  @container details (max-width: 560px) { .pf { padding: 16px 14px; } }
  .crumbs { display: flex; flex-wrap: wrap; gap: 6px; align-items: center; font: 12.5px var(--font-mono); }
  .flist { list-style: none; margin: 0; padding: 0; border: 1px solid var(--border); border-radius: 8px; overflow: hidden; }
  .flist li + li { border-top: 1px solid var(--divider, var(--border)); }
  .row-btn { width: 100%; display: grid; grid-template-columns: 18px 1fr auto auto; gap: 12px; align-items: center; padding: 8px 12px; border: 0; background: var(--bg-surface); font: 13px var(--font-sans); color: var(--text-1); text-align: left; cursor: pointer; }
  .row-btn:hover, .row-btn.on { background: var(--bg-subtle); }
  .row-btn .nm { overflow-wrap: anywhere; font-family: var(--font-mono); font-size: 12.5px; }
  .row-btn .ico { color: var(--text-4); text-align: center; }
  .row-btn .dim { font-size: 12px; white-space: nowrap; }
  .fld.chk { flex-direction: row; align-items: center; gap: 8px; }
  .fld.chk input { width: auto; height: auto; }
  @container details (max-width: 560px) { .row-btn { grid-template-columns: 18px 1fr auto; } .row-btn > :last-child { display: none; } }
</style>
