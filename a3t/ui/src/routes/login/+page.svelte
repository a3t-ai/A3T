<script lang="ts">
  import { goto } from '$app/navigation';
  import { login, isAuthenticated } from '$lib/stores/auth';
  import { API_BASE } from '$lib/config';
  import { onMount } from 'svelte';
  import { startEarlyAccess, finishEarlyAccess, providersAvailable, PROVIDER_LABEL, type Provider, type Identity } from '$lib/core/auth/earlyAccess';

  let email = $state('');
  let password = $state('');
  let error = $state('');
  let loading = $state(false);

  onMount(() => {
    // Already logged in? Redirect to app
    const unsub = isAuthenticated.subscribe(v => { if (v) goto('/'); });
    // Back from LinkedIn: file the request with the verified identity.
    void (async () => {
      const r = await finishEarlyAccess();
      if (!r) return;
      showForm = true;
      void providersAvailable().then((ok) => (providersOn = ok));   // so "try again" has the buttons
      if (r.error || !r.identity) { formError = r.error ?? 'Sign-in failed.'; return; }
      await fromProvider(r.identity);
    })();
    return unsub;
  });

  async function handleSubmit(e: Event) {
    e.preventDefault();
    error = '';
    loading = true;
    const result = await login(email, password);
    loading = false;
    if (result.ok) {
      goto('/');
    } else {
      error = result.error || 'Login failed';
    }
  }

  // ── Request Early Access. The visitor isn't signed in, so this posts to a public
  // webhook (/webhook/* needs no token), whose trigger runs the `a3t/flow_a3t_early_access`
  // flow with the posted body as its input. /message (execute_flow) needs a token. ──
  const EARLY_ACCESS_HOOK = '/webhook/a3t/early-access';
  let showForm = $state(false);
  let submitting = $state(false);
  type Request = { name: string; email: string; company: string; use_case: string; provider?: Provider; verified?: boolean };
  let submitted = $state<Request | null>(null);
  // `website` is a honeypot: hidden from people, filled in by form-filling bots.
  let form = $state({ name: '', email: '', company: '', use_case: '', website: '' });
  let formError = $state('');
  let leaving = $state<Provider | null>(null);

  /// The provider buttons show only once the sign-in realm answers; until it is
  /// set up the popup is just the form.
  let providersOn = $state(false);
  function openForm() {
    showForm = true; submitted = null; formError = '';
    void providersAvailable().then((ok) => (providersOn = ok));
  }
  function closeForm() { showForm = false; }

  async function go(p: Provider) {
    formError = ''; leaving = p;
    try { await startEarlyAccess(p); }
    catch { leaving = null; formError = `Could not start signing in with ${PROVIDER_LABEL[p]}.`; }
  }

  /// Back from a provider: it vouched for the name and email.
  function fromProvider(who: Identity) {
    return fileRequest({
      name: who.name, email: who.email, company: '',
      use_case: `Signed in with ${PROVIDER_LABEL[who.provider]}${who.verified ? '' : ' (email not verified)'}`,
      provider: who.provider, verified: who.verified,
    });
  }

  /// Or typed in by hand.
  async function submitForm(e: Event) {
    e.preventDefault();
    if (form.website) { submitted = { ...form }; return; } // a bot: look accepted, send nothing
    await fileRequest({ name: form.name.trim(), email: form.email.trim(), company: form.company.trim(), use_case: form.use_case.trim() });
    if (submitted) form = { name: '', email: '', company: '', use_case: '', website: '' };
  }

  async function fileRequest(who: Request) {
    submitting = true; formError = '';
    try {
      const payload = {
        id: `ea-${crypto.randomUUID().slice(0, 8)}`,
        name: who.name,
        email: who.email,
        company: who.company,
        use_case: who.use_case,
        ...(who.provider ? { provider: who.provider, email_verified: who.verified } : {}),
        created_at: new Date().toISOString(),
      };
      const res = await fetch(`${API_BASE}${EARLY_ACCESS_HOOK}`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(payload),
      });
      const data = await res.json().catch(() => null);
      // 200 with delivered 0 = no trigger on that path: the request went nowhere.
      if (!res.ok || !data?.delivered) throw new Error(data?.message || data?.error || 'Request failed. Please try again later.');
      submitted = who;
    } catch (err: any) {
      formError = err?.message || 'Something went wrong. Please try again.';
    } finally {
      submitting = false;
    }
  }
</script>

<svelte:head>
  <title>Sign in — A3T</title>
</svelte:head>

<div class="auth-page">
  <div class="auth-card">
    <div class="auth-header">
      <span class="auth-logo">A3T</span>
      <p class="auth-tagline">the agent platform</p>
    </div>

    <h1>Sign in</h1>
    <p class="auth-sub">Enter your credentials to access the platform.</p>

    {#if error}
      <div class="auth-error">{error}</div>
    {/if}

    <form onsubmit={handleSubmit}>
      <div class="field">
        <label for="email">Email</label>
        <input id="email" type="email" bind:value={email} placeholder="you@company.com" required autocomplete="email" />
      </div>
      <div class="field">
        <label for="password">Password</label>
        <input id="password" type="password" bind:value={password} placeholder="Password" required autocomplete="current-password" />
      </div>
      <button type="submit" class="auth-submit" disabled={loading}>
        {#if loading}
          <span class="spinner"></span> Signing in...
        {:else}
          Sign in
        {/if}
      </button>
    </form>

    <div class="auth-footer">
      <span>Don't have access yet?</span>
      <button type="button" class="link-btn" onclick={openForm}>Request early access</button>
    </div>
  </div>

  {#if showForm}
    <div class="modal-overlay" onclick={closeForm} role="presentation">
      <div class="modal" onclick={(e) => e.stopPropagation()} role="dialog" aria-modal="true">
        <button class="modal-close" onclick={closeForm} aria-label="Close">×</button>
        {#if submitted}
          <div class="modal-success">
            <div class="success-check">✓</div>
            <h2>Thank you{submitted.name ? `, ${submitted.name.split(' ')[0]}` : ''}</h2>
            <p>We've saved your request{submitted.provider ? ` (via ${PROVIDER_LABEL[submitted.provider]})` : ''}. We'll be in touch soon at <b>{submitted.email}</b> to set up your account.</p>
            <button type="button" class="auth-submit" onclick={closeForm}>Done</button>
          </div>
        {:else}
          <h2>Request Early Access</h2>
          <p class="modal-sub">A3T is invite-only during private beta. Tell us a bit about you.</p>
          {#if formError}<div class="auth-error">{formError}</div>{/if}
          {#if submitting}
            <p class="ea-wait"><span class="spinner dark"></span> Sending your request…</p>
          {:else}
{#if providersOn}
            <div class="ea-providers">
              <button type="button" class="ea-btn" disabled={!!leaving} onclick={() => go('linkedin')}>
                <svg viewBox="0 0 24 24" aria-hidden="true"><rect width="24" height="24" rx="4" fill="#0A66C2"/><path fill="#fff" d="M7.1 9.5H4.6V19h2.5V9.5zM5.9 5a1.5 1.5 0 1 0 0 3 1.5 1.5 0 0 0 0-3zM19.4 13.6c0-2.6-1.4-4.3-3.7-4.3-1.3 0-2.2.7-2.6 1.4V9.5h-2.4V19h2.5v-5c0-1.3.6-2.2 1.8-2.2 1.1 0 1.6.8 1.6 2.2v5h2.5v-5.4z"/></svg>
                <span>{leaving === 'linkedin' ? 'Opening LinkedIn…' : 'Continue with LinkedIn'}</span>
              </button>
            </div>
            <p class="ea-note">Signing in only shares your name and email with us — it doesn't create an account or give you access.</p>

            <div class="ea-or"><span>or fill in</span></div>

            {/if}

            <form onsubmit={submitForm}>
              <div class="field">
                <label for="ea-name">Name</label>
                <input id="ea-name" bind:value={form.name} placeholder="Jane Doe" required />
              </div>
              <div class="field">
                <label for="ea-email">Work email</label>
                <input id="ea-email" type="email" bind:value={form.email} placeholder="jane@company.com" required />
              </div>
              <div class="field">
                <label for="ea-company">Company <span class="opt">(optional)</span></label>
                <input id="ea-company" bind:value={form.company} placeholder="Acme Inc." />
              </div>
              <div class="field">
                <label for="ea-use">What do you want to build? <span class="opt">(optional)</span></label>
                <textarea id="ea-use" rows="3" bind:value={form.use_case} placeholder="Multi-agent workflows for…"></textarea>
              </div>
              <div class="hp" aria-hidden="true">
                <label for="ea-website">Website</label>
                <input id="ea-website" bind:value={form.website} tabindex="-1" autocomplete="off" />
              </div>
              <button type="submit" class="auth-submit">Submit request</button>
            </form>
          {/if}
        {/if}
      </div>
    </div>
  {/if}

  <div class="auth-aside">
    <div class="aside-content">
      <h2>Build agents that think, remember, and act.</h2>
      <p>A3T is the infrastructure layer for autonomous AI agents — templates, memory, tools, and orchestration in one platform.</p>
      <div class="aside-features">
        <div class="af"><span class="af-ico">T</span> Templates — CNCF Serverless Workflow specs</div>
        <div class="af"><span class="af-ico">A</span> Agents — containers with memory &amp; lifecycle</div>
        <div class="af"><span class="af-ico">C</span> Canvas — K8s CRDs for multi-agent orchestration</div>
      </div>
      <div class="aside-cta">
        <button type="button" class="cta-btn" onclick={openForm}>
          Request Early Access <span class="cta-arrow">→</span>
        </button>
        <span class="cta-note">Invite-only during private beta.</span>
      </div>
    </div>
  </div>
</div>

<style>
  .auth-page {
    display: flex;
    height: 100vh;
    background: var(--bg-page);
  }

  .auth-card {
    width: 440px;
    display: flex;
    flex-direction: column;
    justify-content: center;
    padding: 48px 52px;
    background: var(--bg-surface);
    border-right: 1px solid var(--border);
  }

  .auth-header { margin-bottom: 36px; }
  .auth-logo {
    font-family: var(--font-serif);
    font-size: 28px;
    font-weight: 600;
    letter-spacing: -0.02em;
  }
  .auth-logo::after {
    content: '';
    display: inline-block;
    width: 6px; height: 6px;
    background: var(--ephemeral);
    border-radius: 50%;
    margin-left: 6px;
    vertical-align: middle;
    margin-bottom: 3px;
  }
  .auth-tagline {
    font-family: var(--font-serif);
    font-size: 13px;
    font-style: italic;
    color: var(--text-4);
    margin-top: 2px;
  }

  h1 {
    font-family: var(--font-serif);
    font-size: 24px;
    font-weight: 500;
    letter-spacing: -0.02em;
    margin-bottom: 6px;
  }
  .auth-sub { font-size: 13px; color: var(--text-3); margin-bottom: 24px; }

  .auth-error {
    background: var(--err-bg);
    color: #952d2d;
    border: 1px solid var(--err);
    border-radius: var(--r-sm);
    padding: 8px 12px;
    font-size: 13px;
    margin-bottom: 16px;
  }

  form { display: flex; flex-direction: column; gap: 16px; }

  .field { display: flex; flex-direction: column; gap: 4px; }
  .field label {
    font-size: 12px; font-weight: 500; color: var(--text-2);
  }
  .field input {
    padding: 10px 14px;
    border: 1px solid var(--border);
    border-radius: var(--r-sm);
    font-size: 14px;
    font-family: var(--font-sans);
    background: var(--bg-surface);
    transition: border-color 0.15s;
  }
  .field input:focus {
    outline: none;
    border-color: var(--spec);
    box-shadow: 0 0 0 2px var(--spec-bg);
  }
  .field input::placeholder { color: var(--text-5); }

  .auth-submit {
    padding: 11px 20px;
    background: var(--text-1);
    color: white;
    border: none;
    border-radius: var(--r-sm);
    font-size: 14px;
    font-weight: 500;
    cursor: pointer;
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 8px;
    transition: background 0.15s;
    margin-top: 4px;
  }
  .auth-submit:hover { background: #000; }
  .auth-submit:disabled { opacity: 0.6; cursor: not-allowed; }

  .spinner {
    width: 14px; height: 14px;
    border: 2px solid rgba(255,255,255,0.3);
    border-top-color: white;
    border-radius: 50%;
    animation: spin 0.6s linear infinite;
  }
  .spinner.dark { border-color: var(--border); border-top-color: var(--spec); }
  @keyframes spin { to { transform: rotate(360deg); } }

  .auth-footer {
    margin-top: 24px;
    font-size: 13px;
    color: var(--text-3);
    display: flex;
    gap: 6px;
  }
  .auth-footer a,
  .link-btn {
    color: var(--spec);
    font-weight: 500;
    text-decoration: none;
  }
  .auth-footer a:hover,
  .link-btn:hover { text-decoration: underline; }
  .link-btn {
    background: none;
    border: none;
    padding: 0;
    font-size: 13px;
    font-family: inherit;
    cursor: pointer;
  }

  /* Request Early Access modal */
  .modal-overlay {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.45);
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 24px;
    z-index: 100;
    animation: fade 0.15s ease;
  }
  @keyframes fade { from { opacity: 0; } to { opacity: 1; } }
  .modal {
    position: relative;
    width: 440px;
    max-width: 100%;
    max-height: 90vh;
    overflow-y: auto;
    background: var(--bg-surface);
    border: 1px solid var(--border);
    border-radius: 14px;
    padding: 32px 32px 28px;
    box-shadow: 0 20px 60px rgba(0, 0, 0, 0.25);
    animation: pop 0.15s ease;
  }
  @keyframes pop { from { transform: translateY(8px) scale(0.98); opacity: 0; } to { transform: none; opacity: 1; } }
  .modal-close {
    position: absolute;
    top: 14px; right: 16px;
    width: 30px; height: 30px;
    display: grid; place-items: center;
    background: none;
    border: none;
    border-radius: 6px;
    font-size: 22px;
    line-height: 1;
    color: var(--text-4);
    cursor: pointer;
    transition: background 0.15s, color 0.15s;
  }
  .modal-close:hover { background: var(--bg-subtle); color: var(--text-1); }
  .modal h2 {
    font-family: var(--font-serif);
    font-size: 22px;
    font-weight: 500;
    letter-spacing: -0.02em;
    margin-bottom: 6px;
  }
  .modal-sub { font-size: 13px; color: var(--text-3); margin-bottom: 22px; }
  .modal form { margin-top: 4px; }
  .modal textarea {
    padding: 10px 14px;
    border: 1px solid var(--border);
    border-radius: var(--r-sm);
    font-size: 14px;
    font-family: var(--font-sans);
    background: var(--bg-surface);
    resize: vertical;
    transition: border-color 0.15s;
  }
  .modal textarea:focus {
    outline: none;
    border-color: var(--spec);
    box-shadow: 0 0 0 2px var(--spec-bg);
  }
  .opt { color: var(--text-5); font-weight: 400; }
  .ea-providers { display: flex; flex-direction: column; gap: 10px; }
  .ea-btn {
    display: flex; align-items: center; justify-content: center; gap: 12px;
    height: 44px; padding: 0 16px;
    background: var(--bg-surface); color: var(--text-1);
    border: 1px solid var(--border); border-radius: var(--r-sm);
    font: 500 14px var(--font-sans); cursor: pointer;
    transition: background 0.15s, border-color 0.15s;
  }
  .ea-btn:hover:not(:disabled) { background: var(--bg-subtle); border-color: var(--text-4); }
  .ea-btn:disabled { opacity: 0.6; cursor: default; }
  .ea-btn svg { width: 20px; height: 20px; flex: none; }
  .ea-note { margin-top: 12px; font-size: 12px; color: var(--text-4); line-height: 1.5; }
  .ea-or { display: flex; align-items: center; gap: 12px; margin: 20px 0 16px; font-size: 11.5px; color: var(--text-4); text-transform: uppercase; letter-spacing: 0.06em; }
  .ea-or::before, .ea-or::after { content: ''; flex: 1; height: 1px; background: var(--border); }
  .hp { position: absolute; left: -10000px; width: 1px; height: 1px; overflow: hidden; }
  .ea-wait { display: flex; align-items: center; gap: 10px; font-size: 14px; color: var(--text-2); padding: 12px 0; }
  .modal-success { text-align: center; padding: 12px 0 4px; }
  .success-check {
    width: 52px; height: 52px;
    margin: 0 auto 16px;
    display: grid; place-items: center;
    background: var(--spec-bg, #eef4ff);
    color: var(--spec);
    border-radius: 50%;
    font-size: 26px;
    font-weight: 700;
  }
  .modal-success p { font-size: 14px; color: var(--text-2); line-height: 1.6; margin: 8px 0 22px; }

  /* Right aside */
  .auth-aside {
    flex: 1;
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 48px;
    background: linear-gradient(135deg, var(--bg-page) 0%, var(--bg-subtle) 100%);
  }
  .aside-content { max-width: 480px; }
  .aside-content h2 {
    font-family: var(--font-serif);
    font-size: 32px;
    font-weight: 500;
    letter-spacing: -0.02em;
    line-height: 1.3;
    margin-bottom: 14px;
  }
  .aside-content p {
    font-size: 15px;
    color: var(--text-2);
    line-height: 1.6;
    margin-bottom: 28px;
  }

  .aside-cta {
    margin-top: 34px;
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: 10px;
  }
  .cta-btn {
    display: inline-flex;
    align-items: center;
    gap: 10px;
    padding: 15px 30px;
    background: var(--spec);
    color: #fff;
    font-family: inherit;
    font-size: 15px;
    font-weight: 600;
    letter-spacing: -0.01em;
    border: none;
    border-radius: 10px;
    text-decoration: none;
    cursor: pointer;
    box-shadow: 0 6px 20px var(--spec-bg), 0 1px 2px rgba(0,0,0,0.08);
    transition: transform 0.15s ease, box-shadow 0.15s ease, filter 0.15s ease;
  }
  .cta-btn:hover {
    transform: translateY(-2px);
    filter: brightness(1.05);
    box-shadow: 0 10px 28px var(--spec-bg), 0 2px 6px rgba(0,0,0,0.12);
  }
  .cta-arrow { transition: transform 0.15s ease; }
  .cta-btn:hover .cta-arrow { transform: translateX(3px); }
  .cta-note { font-size: 12px; color: var(--text-4); }

  .aside-features { display: flex; flex-direction: column; gap: 12px; }
  .af {
    display: flex; align-items: center; gap: 12px;
    font-size: 13px; color: var(--text-2);
  }
  .af-ico {
    width: 28px; height: 28px;
    border-radius: 6px;
    display: grid; place-items: center;
    font-family: var(--font-mono);
    font-size: 11px; font-weight: 700;
    color: white;
    flex-shrink: 0;
  }
  .af:nth-child(1) .af-ico { background: var(--spec); }
  .af:nth-child(2) .af-ico { background: var(--runtime); }
  .af:nth-child(3) .af-ico { background: var(--control); }

  /* Tablet */
  @media (max-width: 1024px) {
    .auth-card { width: 380px; padding: 36px 40px; }
    .aside-content h2 { font-size: 26px; }
    .auth-aside { padding: 36px; }
  }

  /* Phone */
  @media (max-width: 768px) {
    .auth-page { flex-direction: column; }
    .auth-card {
      width: 100%;
      border-right: none;
      border-bottom: 1px solid var(--border);
      padding: 32px 24px;
      flex: 1;
    }
    .auth-aside { display: none; }
    .auth-header { margin-bottom: 28px; }
    h1 { font-size: 22px; }
  }

  /* Small phone */
  @media (max-width: 400px) {
    .auth-card { padding: 24px 18px; }
  }
</style>
