// Auth store — JWT-based session management backed by FlowR /auth/* endpoints

import { writable, derived, get } from 'svelte/store';
import * as api from '$lib/services/api';
import { API_BASE } from '$lib/config';

export interface User {
  id: string;
  email: string;
  name: string;
  role: UserRole;
  org_id?: string;
  avatar?: string;
}

export type UserRole = 'superadmin' | 'securityadmin' | 'admin' | 'operator' | 'viewer';

export interface Session {
  user: User;
  access_token: string;
  refresh_token: string;
  expires_in: number;
  /// Set while a SuperAdmin is acting as this user or role: who they really are.
  act?: { sub: string; email: string };
}

const STORAGE_KEY = 'a3t_session';
/// The SuperAdmin's own session, kept while they act as someone else.
const OWN_KEY = 'a3t_session_own';

// Core session store
const sessionStore = writable<Session | null>(null);

// Derived stores
export const isAuthenticated = derived(sessionStore, ($s) => !!$s);
export const currentUser = derived(sessionStore, ($s) => $s?.user ?? null);
export const userRole = derived(sessionStore, ($s) => $s?.user?.role ?? null);
export const accessToken = derived(sessionStore, ($s) => $s?.access_token ?? null);
/// While a SuperAdmin acts as someone: who they really are.
export const actingAs = derived(sessionStore, ($s) => ($s?.act ? { as: $s.user, by: $s.act } : null));

// Initialize from localStorage + sync tokens to API client
export function initAuth(): boolean {
  if (typeof window === 'undefined') return false;
  // Back from an organisation's sign-in: that is the session (or why it failed,
  // which the login page shows).
  const sso = completeSso();
  if (sso?.ok) return true;
  if (sso?.error) { try { sessionStorage.setItem('a3t_sso_error', sso.error); } catch { /* private mode */ } }
  try {
    const stored = localStorage.getItem(STORAGE_KEY);
    if (!stored) return false;
    const session: Session = JSON.parse(stored);
    // Tokens to the API client before the store (see saveSession).
    api.setTokens(session.access_token, session.refresh_token);
    // Handle token expiry — redirect to login
    api.onTokenExpired(() => (session.act ? stopActingAs() : signOut()));
    sessionStore.set(session);
    return true;
  } catch {
    localStorage.removeItem(STORAGE_KEY);
    return false;
  }
}

// Save session to store + localStorage + API client
function saveSession(session: Session) {
  // The API client and storage first, the store last: subscribers (the layout
  // starting the app) call the API the moment the store changes.
  api.setTokens(session.access_token, session.refresh_token);
  api.onTokenExpired(() => (session.act ? stopActingAs() : signOut()));
  if (typeof window !== 'undefined') {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(session));
  }
  sessionStore.set(session);
}

// ── Login — calls POST /auth/login ──

export type LoginResult = { ok: boolean; error?: string; code?: string };

/// The session for a sign-in answer (login or first password).
function signedIn(data: api.LoginResponse) {
  saveSession({
    user: {
      id: data.user.id,
      email: data.user.email,
      name: data.user.name,
      role: data.user.role as UserRole,
      org_id: data.user.org_id,
    },
    access_token: data.access_token,
    refresh_token: data.refresh_token,
    expires_in: data.expires_in,
  });
}

/// `code: 'password_change_required'` — the password was right but temporary:
/// ask for a new one and call `setFirstPassword`.
export async function login(email: string, password: string): Promise<LoginResult> {
  if (!email || !password) return { ok: false, error: 'Email and password required' };
  const res = await api.login(email, password);
  if (res.ok && res.data) { signedIn(res.data); return { ok: true }; }
  return { ok: false, error: res.error || 'Login failed', code: res.detail?.code };
}

/// Replace a temporary password at first sign-in, then sign in with the new one.
export async function setFirstPassword(email: string, password: string, newPassword: string): Promise<LoginResult> {
  const res = await api.firstPassword(email, password, newPassword);
  if (res.ok && res.data) { signedIn(res.data); return { ok: true }; }
  return { ok: false, error: res.error || 'Could not set your password' };
}

// ── Sign in with your organisation (.ai/A3T_ORG_SSO_SPEC.md) ──
// FlowR sends the browser to the organisation's own identity provider (through
// Keycloak) and back here with the A3T token in the URL fragment, which never
// reaches a server.

/// Where to send the browser to sign in through an organisation: a work email
/// (its domain picks the org) or the org's handle.
export function ssoStartUrl(orgOrEmail: string): string {
  const v = orgOrEmail.trim();
  const q = new URLSearchParams(v.includes('@') ? { email: v } : { org: v.toLowerCase() });
  if (typeof window !== 'undefined') q.set('return', window.location.origin);
  return `${API_BASE}/auth/sso/start?${q}`;
}

/// Back from the organisation's sign-in: turn `#sso=…` into the session.
/// `null` when this page load isn't a return from it.
export function completeSso(): { ok: boolean; error?: string } | null {
  if (typeof window === 'undefined' || !/[#&](sso|sso_error)=/.test(window.location.hash)) return null;
  const h = new URLSearchParams(window.location.hash.slice(1));
  history.replaceState(null, '', window.location.pathname + window.location.search);
  if (h.get('sso_error')) return { ok: false, error: h.get('sso_error') ?? 'Sign-in failed' };
  const token = h.get('sso') ?? '';
  try {
    const c = JSON.parse(atob(token.split('.')[1].replace(/-/g, '+').replace(/_/g, '/')));
    saveSession({
      user: { id: c.sub, email: c.email, name: c.name || c.email, role: c.role as UserRole, org_id: c.org },
      access_token: token,
      refresh_token: h.get('sso_refresh') ?? '',
      expires_in: Number(h.get('expires_in') ?? 0),
    });
    return { ok: true };
  } catch {
    return { ok: false, error: 'The sign-in returned an unreadable token.' };
  }
}

// ── Signup — calls POST /auth/signup ──

export async function signup(name: string, email: string, password: string): Promise<{ ok: boolean; error?: string }> {
  if (!name || !email || !password) return { ok: false, error: 'All fields required' };
  if (password.length < 8) return { ok: false, error: 'Password must be at least 8 characters' };
  if (!email.includes('@')) return { ok: false, error: 'Invalid email address' };

  const res = await api.signup(name, email, password);

  if (res.ok && res.data) {
    const session: Session = {
      user: {
        id: res.data.user.id,
        email: res.data.user.email,
        name: res.data.user.name,
        role: res.data.user.role as UserRole,
        org_id: res.data.user.org_id,
      },
      access_token: res.data.access_token,
      refresh_token: res.data.refresh_token,
      expires_in: res.data.expires_in,
    };
    saveSession(session);
    return { ok: true };
  }

  return { ok: false, error: res.error || 'Signup failed' };
}

// ── Sign out ──

export function signOut() {
  sessionStore.set(null);
  orgSecurity.set([]);
  api.clearTokens();
  if (typeof window !== 'undefined') {
    localStorage.removeItem(STORAGE_KEY);
    localStorage.removeItem(OWN_KEY);
  }
}

// ── A SuperAdmin acting as a user or a role ──
// FlowR issues a short-lived token for them; their own session is kept aside and
// put back by Stop (or when that token runs out). The page reloads each way, so
// nothing from one identity lingers in the other's view.

export async function actAs(target: { user_id: string } | { role: string }): Promise<{ ok: boolean; error?: string }> {
  const own = get(sessionStore);
  if (!own || own.act) return { ok: false, error: 'Stop acting as someone else first.' };
  const res = await api.impersonateApi(target);
  if (!res.ok || !res.data) return { ok: false, error: res.error || 'Could not act as them' };
  const d = res.data;
  try { localStorage.setItem(OWN_KEY, JSON.stringify(own)); } catch { return { ok: false, error: 'This browser cannot keep your own session aside.' }; }
  saveSession({
    user: { id: d.user.id, email: d.user.email, name: d.user.name, role: d.user.role as UserRole, org_id: d.user.org_id },
    access_token: d.access_token, refresh_token: '', expires_in: d.expires_in, act: d.act,
  });
  try { localStorage.removeItem('a3t.context'); } catch { /* the project picked belongs to the SuperAdmin */ }
  if (typeof window !== 'undefined') window.location.assign('/');
  return { ok: true };
}

export function stopActingAs() {
  let own: Session | null = null;
  try { own = JSON.parse(localStorage.getItem(OWN_KEY) ?? 'null'); } catch { own = null; }
  try { localStorage.removeItem(OWN_KEY); localStorage.removeItem('a3t.context'); } catch { /* */ }
  if (own) saveSession(own); else signOut();
  if (typeof window !== 'undefined') window.location.assign('/');
}

// Get current session (non-reactive)
export function getSession(): Session | null {
  return get(sessionStore);
}

// ── Role hierarchy check ──

const ROLE_HIERARCHY: Record<UserRole, number> = {
  superadmin: 200,
  // SecurityAdmin is the InfoSec seam, not part of the author chain. Ranked for
  // hasRole() checks; its actual capabilities come from ROLE_PERMISSIONS below.
  securityadmin: 90,
  admin: 100,
  // The developer tier ranks with admin, as on the server (canonicalize).
  operator: 100,
  viewer: 20,
};

export function hasRole(requiredRole: UserRole): boolean {
  const role = get(userRole);
  if (!role) return false;
  return ROLE_HIERARCHY[role] >= ROLE_HIERARCHY[requiredRole];
}

// ── Permission helpers (simple role-based for now) ──

export type Action = 'create' | 'read' | 'update' | 'delete' | 'execute' | 'deploy';
export type ResourceType = 'template' | 'agent' | 'canvas' | 'resource' | 'flow' | 'memory' | 'settings';

const ROLE_PERMISSIONS: Record<UserRole, Record<ResourceType, Action[]>> = {
  superadmin: {
    template: ['create', 'read', 'update', 'delete', 'deploy'],
    agent: ['create', 'read', 'update', 'delete', 'execute'],
    canvas: ['create', 'read', 'update', 'delete', 'deploy'],
    resource: ['create', 'read', 'update', 'delete'],
    flow: ['create', 'read', 'update', 'delete', 'execute'],
    memory: ['create', 'read', 'update', 'delete'],
    settings: ['create', 'read', 'update', 'delete'],
  },
  admin: {
    template: ['create', 'read', 'update', 'delete', 'deploy'],
    agent: ['create', 'read', 'update', 'delete', 'execute'],
    canvas: ['create', 'read', 'update', 'delete', 'deploy'],
    resource: ['create', 'read', 'update', 'delete'],
    flow: ['create', 'read', 'update', 'delete', 'execute'],
    memory: ['read'],
    settings: ['read', 'update'],
  },
  // InfoSec seam: read-only over the platform (so they can navigate + review),
  // plus policy/settings. Cedar is the authority for actual policy edits.
  securityadmin: {
    template: ['read'],
    agent: ['read'],
    canvas: ['read'],
    resource: ['read'],
    flow: ['read'],
    memory: ['read'],
    settings: ['read', 'update'],
  },
  // `operator` is the developer tier: the server gives it Admin, Operator and
  // Viewer (flowr-policy roles::canonicalize), so it authors like an admin —
  // within the projects it can write (canWriteIn), never the InfoSec seam.
  operator: {
    template: ['create', 'read', 'update', 'delete', 'deploy'],
    agent: ['create', 'read', 'update', 'delete', 'execute'],
    canvas: ['create', 'read', 'update', 'delete', 'deploy'],
    resource: ['create', 'read', 'update', 'delete'],
    flow: ['create', 'read', 'update', 'delete', 'execute'],
    memory: ['read'],
    settings: ['read'],
  },
  viewer: {
    template: ['read'],
    agent: ['read'],
    canvas: ['read'],
    resource: ['read'],
    flow: ['read'],
    memory: ['read'],
    settings: [],
  },
};

export function can(action: Action, resource: ResourceType) {
  return derived(userRole, ($role) => {
    if (!$role) return false;
    return ROLE_PERMISSIONS[$role]?.[resource]?.includes(action) ?? false;
  });
}

export function canAny(actions: Action[], resource: ResourceType) {
  return derived(userRole, ($role) => {
    if (!$role) return false;
    const allowed = ROLE_PERMISSIONS[$role]?.[resource] ?? [];
    return actions.some(a => allowed.includes(a));
  });
}

export function checkPermission(action: Action, resource: ResourceType): boolean {
  const role = get(userRole);
  if (!role) return false;
  return ROLE_PERMISSIONS[role]?.[resource]?.includes(action) ?? false;
}

// ── Menu filtering by role ──

import { getMenusForRole } from '$lib/core/auth/menu';

/// The orgs whose security you look after (admin or securityadmin there): they
/// open Security for those orgs. Set when the projects and orgs load.
export const orgSecurity = writable<string[]>([]);

/** Reactive store: sidebar menu groups filtered for the current user's role */
export const menuGroups = derived([userRole, orgSecurity], ([$role, $orgs]) =>
  $role ? getMenusForRole($role, $orgs.length > 0) : []
);

/// SuperAdmin or SecurityAdmin: who may see Security and the Decision log (the
/// server enforces it; this only keeps links to them away from everyone else).
export function isInfosec(): boolean {
  const r = getSession()?.user?.role;
  return r === 'superadmin' || r === 'securityadmin';
}

/// Who opens Security: InfoSec (every org), or an org's admin or security admin
/// (their orgs only).
export function opensSecurity(): boolean {
  return isInfosec() || get(orgSecurity).length > 0;
}
