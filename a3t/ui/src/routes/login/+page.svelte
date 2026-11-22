<script lang="ts">
  import { goto } from '$app/navigation';
  import { login, setFirstPassword, isAuthenticated, ssoStartUrl, completeSso } from '$lib/stores/auth';
  import { API_BASE } from '$lib/config';
  import { onMount } from 'svelte';
  import { startEarlyAccess, finishEarlyAccess, providersAvailable, PROVIDER_LABEL, type Provider, type Identity } from '$lib/core/auth/earlyAccess';

  let email = $state('');
  let password = $state('');
  /// ID-9: SuperAdmin and SecurityAdmin also give their authenticator code.
  let otp = $state('');
  let wantOtp = $state(false);
  let error = $state('');
  let loading = $state(false);

  // Sign in through your organisation's own identity provider. Checked first, so
  // an unknown organisation is said here, not on a bare error page.
  let ssoOpen = $state(false);
  let orgId = $state('');
  let ssoLeaving = $state(false);
  let ssoError = $state('');
  async function ssoGo(e: Event) {
    e.preventDefault();
    const v = orgId.trim();
    if (!v) return;
    error = ''; ssoError = ''; ssoLeaving = true;
    const url = ssoStartUrl(v);
    const res = await fetch(url, { redirect: 'manual' }).catch(() => null);
    if (res && (res.type === 'opaqueredirect' || (res.status >= 300 && res.status < 400))) { window.location.href = url; return; }
    ssoLeaving = false;
    if (!res) { ssoError = 'The sign-in service can\'t be reached. Try again in a moment.'; return; }
    if (res.status === 404) {
      ssoError = v.includes('@')
        ? `No organisation signs in with single sign-on for ${v.split('@')[1]}. Use your email and password above, or ask your administrator.`
        : `No organisation called "${v}" signs in with single sign-on. Check the name, or use your work email.`;
      return;
    }
    const j = await res.json().catch(() => ({}));
    ssoError = j.error ?? 'Single sign-on is not available right now.';
  }

  // Forgot password: Keycloak emails a link to set a new one. The answer is the
  // same whether or not the account exists.
  let forgotOpen = $state(false);
  let forgotEmail = $state('');
  let forgotBusy = $state(false);
  let forgotSent = $state('');
  let forgotError = $state('');
  function openForgot() { forgotOpen = true; forgotEmail = email; forgotSent = ''; forgotError = ''; error = ''; }
  async function sendReset(e: Event) {
    e.preventDefault();
    const v = forgotEmail.trim();
    if (!v.includes('@')) { forgotError = 'Enter the email you sign in with.'; return; }
    forgotBusy = true; forgotError = '';
    const res = await fetch(`${API_BASE}/auth/forgot`, {
      method: 'POST', headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ email: v, return: window.location.origin }),
    }).catch(() => null);
    forgotBusy = false;
    if (!res) { forgotError = 'The sign-in service can\'t be reached. Try again in a moment.'; return; }
    const j = await res.json().catch(() => ({}));
    if (!res.ok) { forgotError = j.error ?? 'Could not send the link. Try again later.'; return; }
    forgotSent = v;
  }

  onMount(() => {
    // Back from the organisation's sign-in?
    const sso = completeSso();
    if (sso && !sso.ok) error = sso.error ?? 'Sign-in failed';
    try { const e = sessionStorage.getItem('a3t_sso_error'); if (e) { error = e; sessionStorage.removeItem('a3t_sso_error'); } } catch { /* private mode */ }
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
    const result = await login(email, password, otp);
    loading = false;
    if (result.ok) {
      goto('/');
    } else if (result.code === 'password_change_required') {
      // The password was right but temporary: choose your own before going in.
      choosing = true; newPassword = ''; confirmPassword = ''; chooseError = '';
    } else if (result.code === 'mfa_required') {
      error = result.error || 'This account signs in with its authenticator code.';
    } else {
      error = result.error || 'Login failed';
      // A wrong password and a missing code look the same: offer the code field.
      if (!wantOtp) wantOtp = true;
    }
  }

  // ── First sign-in: an admin gave a temporary password; choose your own. ──
  let choosing = $state(false);
  let newPassword = $state('');
  let confirmPassword = $state('');
  let chooseError = $state('');
  let chooseBusy = $state(false);
  async function chooseSubmit(e: Event) {
    e.preventDefault();
    chooseError = '';
    if (newPassword.length < 8) { chooseError = 'Use at least 8 characters.'; return; }
    if (newPassword !== confirmPassword) { chooseError = "The two passwords don't match."; return; }
    if (newPassword === password) { chooseError = 'Choose a password different from the temporary one.'; return; }
    chooseBusy = true;
    const r = await setFirstPassword(email, password, newPassword);
    chooseBusy = false;
    if (r.ok) { password = ''; newPassword = ''; confirmPassword = ''; goto('/'); }
    else chooseError = r.error ?? 'Could not set your password.';
  }
  function chooseCancel() { choosing = false; password = ''; newPassword = ''; confirmPassword = ''; chooseError = ''; }

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

    {#if choosing}
      <h1>Choose your password</h1>
      <p class="auth-sub">Your account has a temporary password. Choose your own to finish setting it up; you'll use it from now on.</p>
      {#if chooseError}<div class="auth-error">{chooseError}</div>{/if}
      <form onsubmit={chooseSubmit}>
        <div class="field">
          <label for="cp-email">Email</label>
          <input id="cp-email" type="email" value={email} readonly autocomplete="username" />
        </div>
        <div class="field">
          <label for="cp-new">New password</label>
          <!-- svelte-ignore a11y_autofocus -->
          <input id="cp-new" type="password" bind:value={newPassword} oninput={() => (chooseError = '')} placeholder="At least 8 characters" required autocomplete="new-password" autofocus />
        </div>
        <div class="field">
          <label for="cp-confirm">Confirm new password</label>
          <input id="cp-confirm" type="password" bind:value={confirmPassword} oninput={() => (chooseError = '')} placeholder="The same again" required autocomplete="new-password" />
        </div>
        <button type="submit" class="auth-submit" disabled={chooseBusy}>
          {#if chooseBusy}<span class="spinner"></span> Setting your password...{:else}Set password and sign in{/if}
        </button>
      </form>
      <div class="choose-back"><button type="button" class="link-btn" onclick={chooseCancel}>Back to sign in</button></div>
    {:else if forgotOpen}
      <h1>Reset your password</h1>
      <p class="auth-sub">We'll email you a link to set a new one.</p>
      {#if forgotSent}
        <div class="auth-ok">If <b>{forgotSent}</b> has an account, a link is on its way. It works for 30 minutes.</div>
        <button type="button" class="auth-submit" onclick={() => (forgotOpen = false)}>Back to sign in</button>
      {:else}
        {#if forgotError}<div class="auth-error">{forgotError}</div>{/if}
        <form onsubmit={sendReset}>
          <div class="field">
            <label for="forgot-email">Email</label>
            <!-- svelte-ignore a11y_autofocus -->
            <input id="forgot-email" type="email" bind:value={forgotEmail} placeholder="you@company.com" required autocomplete="email" autofocus />
          </div>
          <button type="submit" class="auth-submit" disabled={forgotBusy}>
            {#if forgotBusy}<span class="spinner"></span> Sending…{:else}Send reset link{/if}
          </button>
        </form>
        <p class="auth-hint">Sign in with your organisation? Reset your password there — A3T never holds it.</p>
        <button type="button" class="link-btn back" onclick={() => (forgotOpen = false)}>← Back to sign in</button>
      {/if}
    {:else}
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
          <div class="label-row">
            <label for="password">Password</label>
            <button type="button" class="link-btn small" onclick={openForgot}>Forgot password?</button>
          </div>
          <input id="password" type="password" bind:value={password} placeholder="Password" required autocomplete="current-password" />
        </div>
        {#if wantOtp}
          <div class="field">
            <div class="label-row">
              <label for="otp">Authenticator code</label>
              <a class="link-btn small" href={`${API_BASE}/auth/account`} target="_blank" rel="noopener">Set up an authenticator</a>
            </div>
            <input id="otp" inputmode="numeric" pattern="[0-9]*" maxlength="8" bind:value={otp} placeholder="SuperAdmin and SecurityAdmin: the 6-digit code" autocomplete="one-time-code" />
          </div>
        {:else}
          <button type="button" class="link-btn small" onclick={() => (wantOtp = true)}>I use an authenticator code</button>
        {/if}
        <button type="submit" class="auth-submit" disabled={loading}>
          {#if loading}
            <span class="spinner"></span> Signing in...
          {:else}
            Sign in
          {/if}
        </button>
      </form>

      <div class="sso-or"><span>or</span></div>
      {#if !ssoOpen}
        <button type="button" class="sso-submit" onclick={() => { ssoOpen = true; ssoError = ''; }}>
          <svg width="15" height="15" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" aria-hidden="true"><circle cx="5" cy="8" r="3"/><path d="M8 8h7M12.5 8v2.5M14.5 8v2"/></svg>
          Sign in with single sign-on
        </button>
      {:else}
        <form class="sso" onsubmit={ssoGo}>
          <div class="field">
            <label for="org">Work email or organisation</label>
            <!-- svelte-ignore a11y_autofocus -->
            <input id="org" bind:value={orgId} placeholder="you@yourcompany.com" autocomplete="email" autofocus class:bad={!!ssoError} oninput={() => (ssoError = '')} />
            {#if ssoError}<div class="field-error">{ssoError}</div>{:else}<div class="field-hint">You'll sign in on your organisation's own page.</div>{/if}
          </div>
          <div class="sso-actions">
            <button type="submit" class="sso-submit primary" disabled={ssoLeaving || !orgId.trim()}>
              {#if ssoLeaving}<span class="spinner dark"></span> Opening your organisation…{:else}Continue{/if}
            </button>
            <button type="button" class="link-btn" onclick={() => { ssoOpen = false; ssoError = ''; }}>Cancel</button>
          </div>
        </form>
      {/if}
    {/if}

    <!-- On a phone the side panel (and its big button) is hidden: the same button here. -->
    <div class="phone-cta">
      <button type="button" class="cta-btn" onclick={openForm}>Request Early Access <span class="cta-arrow">→</span></button>
      <span class="cta-note">Invite-only during private beta.</span>
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
      <p>A3T runs your AI agents behind an API, gives each one a memory that lasts, and governs what every agent and person may do.</p>
      <div class="aside-features">
        <div class="af"><span class="af-ico">A</span> Agents — called over an API; versioned, branched and cloned like code</div>
        <div class="af"><span class="af-ico">M</span> Memory — working, episodic, semantic and procedural, kept per agent</div>
        <div class="af"><span class="af-ico">G</span> Governance — sign-in per organisation, a role per project, every decision logged</div>
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

  .link-btn {
    color: var(--spec);
    font-weight: 500;
    text-decoration: none;
  }
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
    .label-row { display: flex; align-items: baseline; justify-content: space-between; }
  .link-btn.small { font-size: 12px; }
  .link-btn.back { margin-top: 14px; font-size: 13px; }
  .auth-ok { margin-bottom: 16px; padding: 10px 12px; border-radius: 8px; background: #ecf6f0; border: 1px solid #b9dcc6; color: #1f5b3a; font-size: 13px; line-height: 1.5; }
  .auth-hint { margin: 14px 0 0; font-size: 12px; color: var(--text-4, #8a8578); line-height: 1.5; }
  .field-hint { margin-top: 4px; font-size: 12px; color: var(--text-4, #8a8578); }
  .field-error { margin-top: 6px; font-size: 12.5px; color: #b42318; line-height: 1.45; }
  .field input.bad { border-color: #e3a29a; }
  .sso-submit { display: inline-flex; align-items: center; justify-content: center; gap: 8px; }
  .sso-submit.primary { background: var(--text-1, #1c1b19); border-color: var(--text-1, #1c1b19); color: #fff; }
  .sso-submit.primary:hover:not(:disabled) { opacity: .9; }
  .sso-actions { display: flex; align-items: center; gap: 14px; }
  .sso-actions .sso-submit { flex: 1; }
  .phone-cta { display: none; }
  @media (max-width: 768px) {
    .phone-cta { display: flex; flex-direction: column; align-items: flex-start; gap: 8px; margin-top: 28px; padding-top: 22px; border-top: 1px solid var(--border, #e5e1d8); }
    .phone-cta .cta-btn { width: 100%; justify-content: center; }
  }
  .sso-or { display: flex; align-items: center; gap: 10px; margin: 18px 0 6px; color: var(--text-3, #8a8578); font-size: 12px; }
  .sso-or::before, .sso-or::after { content: ''; flex: 1; height: 1px; background: var(--border, #e5e1d8); }
  .sso .field { margin-bottom: 10px; }
  .sso-submit { width: 100%; padding: 10px; border-radius: 8px; border: 1px solid var(--border, #d9d4c8); background: var(--bg-surface, #fff); color: var(--text-1, #222); font: inherit; font-weight: 600; cursor: pointer; }
  .sso-submit:hover:not(:disabled) { border-color: var(--text-2, #555); }
  .sso-submit:disabled { opacity: .55; cursor: default; }
  .choose-back { margin-top: 14px; text-align: center; }
</style>
