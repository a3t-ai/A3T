// Request early access by signing in with Google, LinkedIn or GitHub.
//
// The provider (through Keycloak) vouches for the visitor's name and email, and
// that is all early access captures. It never grants access: the sign-in runs
// in its OWN realm, `a3t-early-access`, which FlowR does not trust — a token
// from it is rejected by the API outright. The platform realm (`a3t`) is not
// involved, so no platform account is created; admitting someone stays a
// separate, deliberate step.
//
// Browser side only: authorization code + PKCE against a public client
// (`a3t-early-access`), `kc_idp_hint` to go straight to the provider. The tokens are
// read once for the identity and then dropped — the visitor is NOT signed in.
import { API_BASE } from '$lib/config';

export type Provider = 'google' | 'linkedin' | 'github';

/// Keycloak's alias for each provider (LinkedIn's OIDC provider is
/// `linkedin-openid-connect` in Keycloak 22+). Override per environment with
/// VITE_KC_IDP_<NAME> if the realm uses other aliases.
const IDP_ALIAS: Record<Provider, string> = {
  google: (import.meta.env.VITE_KC_IDP_GOOGLE as string) || 'google',
  linkedin: (import.meta.env.VITE_KC_IDP_LINKEDIN as string) || 'linkedin-openid-connect',
  github: (import.meta.env.VITE_KC_IDP_GITHUB as string) || 'github',
};
export const PROVIDER_LABEL: Record<Provider, string> = { google: 'Google', linkedin: 'LinkedIn', github: 'GitHub' };

const CLIENT_ID = (import.meta.env.VITE_KC_EA_CLIENT as string) || 'a3t-early-access';
const REALM = (import.meta.env.VITE_KC_EA_REALM as string) || 'a3t-early-access';
const STORE = 'a3t.ea.pkce';

/// The early-access realm on the same Keycloak as the API: console.<x> →
/// keycloak.<x>/realms/a3t-early-access. Deliberately NOT the realm FlowR trusts.
export function realmUrl(): string {
  const pin = import.meta.env.VITE_KC_EA_REALM_URL as string | undefined;
  if (pin) return pin.replace(/\/$/, '');
  const base = API_BASE || (typeof window !== 'undefined' ? window.location.origin : '');
  return base.replace('://console.', '://keycloak.').replace(/\/$/, '') + `/realms/${REALM}`;
}

/// Is sign-in with a provider set up here? The realm's discovery document is
/// public and CORS-enabled; until the realm exists it is a 404, and sending a
/// visitor there would strand them on Keycloak's "Page not found".
let available: Promise<boolean> | null = null;
export function providersAvailable(): Promise<boolean> {
  available ??= fetch(`${realmUrl()}/.well-known/openid-configuration`)
    .then((r) => r.ok)
    .catch(() => false);
  return available;
}

const redirectUri = () => `${window.location.origin}/login`;

function b64url(bytes: ArrayBuffer | Uint8Array): string {
  const a = bytes instanceof Uint8Array ? bytes : new Uint8Array(bytes);
  let s = '';
  for (const b of a) s += String.fromCharCode(b);
  return btoa(s).replace(/\+/g, '-').replace(/\//g, '_').replace(/=+$/, '');
}
const random = (n = 32) => b64url(crypto.getRandomValues(new Uint8Array(n)));

/// Leave for the provider. Comes back to /login with ?code&state.
export async function startEarlyAccess(provider: Provider): Promise<void> {
  const verifier = random(48);
  const state = random(16);
  const challenge = b64url(await crypto.subtle.digest('SHA-256', new TextEncoder().encode(verifier)));
  try { sessionStorage.setItem(STORE, JSON.stringify({ verifier, state, provider })); } catch { /* private mode: the return will say so */ }
  const q = new URLSearchParams({
    client_id: CLIENT_ID,
    response_type: 'code',
    scope: 'openid email profile',
    redirect_uri: redirectUri(),
    state,
    code_challenge: challenge,
    code_challenge_method: 'S256',
    kc_idp_hint: IDP_ALIAS[provider],
  });
  window.location.assign(`${realmUrl()}/protocol/openid-connect/auth?${q}`);
}

export type Identity = { name: string; email: string; provider: Provider; verified: boolean };

/// On /login: if this is the provider coming back, finish it. Returns null when
/// the URL is not an early-access return, so the page behaves as before.
export async function finishEarlyAccess(): Promise<{ identity?: Identity; error?: string } | null> {
  const url = new URL(window.location.href);
  const code = url.searchParams.get('code');
  const state = url.searchParams.get('state');
  const kcError = url.searchParams.get('error');
  if (!state || (!code && !kcError)) return null;
  let saved: { verifier: string; state: string; provider: Provider } | null = null;
  try { saved = JSON.parse(sessionStorage.getItem(STORE) ?? 'null'); } catch { saved = null; }
  if (!saved || saved.state !== state) return null;      // not ours
  sessionStorage.removeItem(STORE);
  // Clean the address bar: the code is single-use and should not linger.
  history.replaceState(null, '', url.pathname);
  if (kcError) {
    const why = url.searchParams.get('error_description');
    return { error: kcError === 'access_denied' ? 'Sign-in was cancelled.' : `Sign-in failed: ${why || kcError}` };
  }
  const res = await fetch(`${realmUrl()}/protocol/openid-connect/token`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/x-www-form-urlencoded' },
    body: new URLSearchParams({
      grant_type: 'authorization_code',
      client_id: CLIENT_ID,
      code: code!,
      redirect_uri: redirectUri(),
      code_verifier: saved.verifier,
    }),
  }).catch(() => null);
  if (!res || !res.ok) return { error: `Could not finish signing in with ${PROVIDER_LABEL[saved.provider]}. Please try again.` };
  const tok = await res.json().catch(() => null);
  const claims = decodeJwt(tok?.id_token);
  if (!claims?.email) return { error: `${PROVIDER_LABEL[saved.provider]} did not share an email address, which we need to reach you.` };
  return {
    identity: {
      name: claims.name || [claims.given_name, claims.family_name].filter(Boolean).join(' ') || claims.preferred_username || '',
      email: claims.email,
      provider: saved.provider,
      verified: claims.email_verified !== false,
    },
  };
  // `tok` goes out of scope here: nothing is stored, the visitor is not signed in.
}

function decodeJwt(t: unknown): any {
  if (typeof t !== 'string') return null;
  try {
    const p = t.split('.')[1].replace(/-/g, '+').replace(/_/g, '/');
    return JSON.parse(decodeURIComponent(escape(atob(p + '='.repeat((4 - (p.length % 4)) % 4)))));
  } catch { return null; }
}
