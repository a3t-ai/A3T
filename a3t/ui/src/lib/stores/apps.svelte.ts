import * as L from './layout.svelte.ts';

// Shell state: which apps have a list, open apps, server pins, and the rail and
// context-list functions older callers use. The layout itself (Menu ·
// Items|Chat · Details, pins, pop-ins) lives in layout.svelte.ts.
//
// "Active apps" is taskbar-shaped: an app opened stays in the list until it is
// explicitly closed. It is deliberately NOT keep-alive — switching apps remounts
// the page and it restores from the server or localStorage, which is how every
// page in this console already works.

const LS_OPEN = 'a3t.shell.openApps';
const LS_CTX_W = 'a3t.shell.contextW';

export interface OpenApp {
  type: string;
  title: string;
}

function load<T>(key: string, fallback: T): T {
  if (typeof window === 'undefined') return fallback;
  try {
    const v = localStorage.getItem(key);
    return v === null ? fallback : (JSON.parse(v) as T);
  } catch {
    return fallback;
  }
}

function save(key: string, value: unknown) {
  if (typeof window === 'undefined') return;
  try {
    localStorage.setItem(key, JSON.stringify(value));
  } catch {
    /* private mode / blocked storage — the shell still works, it just forgets */
  }
}

// ── Which apps have a context list, and what it is called ──
//
// An app absent from this table has no list, so its context column collapses
// rather than showing an empty pane. `source` is resolved by ContextList.
/// `focuses` records whether the app's page actually acts on a selected item
/// (it reads `props.selectedId`). Where it does not, the list still navigates to
/// the app and stays useful as an overview, but it cannot yet jump to the item —
/// the pane says so rather than looking broken. Canvas is absent because there is
/// no list tool for it on the server.
export const APP_CONTEXT: Record<string, { label: string; source: string; focuses: boolean }> = {
  chat: { label: 'Conversations', source: 'sessions', focuses: true },
  projects: { label: 'Projects', source: 'projects', focuses: true },
  agents: { label: 'Agents', source: 'agents', focuses: true },
  templates: { label: 'Templates', source: 'templates', focuses: true },
  // Flow Studio has its own explorer (projects → flows, resources, triggers…).
  // Resources lists its FIXED subtype categories, not instances: the page is a
  // per-category browser, so the pane offers exactly the categories it filters to.
  // Every resource and flow, grouped by kind; the one picked opens in Details.
  resources: { label: 'Resources', source: 'resources', focuses: true },
  triggers: { label: 'Triggers', source: 'triggers', focuses: true },
  executions: { label: 'Executions', source: 'executions', focuses: true },
  runtime: { label: 'Hosts & Runtimes', source: 'runtimes', focuses: true },
  canvas: { label: 'Canvases', source: 'canvases', focuses: true },
  // Memory by agent; the one picked opens its memory in Details.
  memory: { label: 'Agents', source: 'memory-agents', focuses: true },
  // Storage resources (S3, MinIO, GCS, Azure, FTP); the one picked opens in Details.
  datarooms: { label: 'Storage', source: 'storage', focuses: true },
  // Knowledge-graph databases; the one picked opens in Details.
  knowledge: { label: 'Knowledge graphs', source: 'kg-dbs', focuses: true },
  // Fixed sections, like Resources' categories.
  security: { label: 'Security', source: 'security-sections', focuses: true },
  // Views of the decision log; the selected project scopes them.
  logs: { label: 'Decisions', source: 'decision-views', focuses: true },
  // Every Settings page is an item; the form opens in Details.
  settings: { label: 'Settings', source: 'settings-sections', focuses: true },
};

export function hasContext(type: string | undefined | null): boolean {
  return !!type && type in APP_CONTEXT;
}

export function contextLabel(type: string | undefined | null): string {
  return (type && APP_CONTEXT[type]?.label) || 'Context';
}

// ── State ──

let openApps = $state<OpenApp[]>(load<OpenApp[]>(LS_OPEN, []));
let contextW = $state<number>(load<number>(LS_CTX_W, 260));

export const RAIL_W = 104;
export const CONTEXT_MIN = 180;
export const CONTEXT_MAX = 460;

// ── Active apps (taskbar) ──

export function getOpenApps(): OpenApp[] {
  return openApps;
}

/// Record an app as open. Idempotent, and refreshes the title so a renamed menu
/// entry does not leave a stale label in the list.
export function trackOpen(type: string, title: string) {
  if (!type) return;
  const i = openApps.findIndex((a) => a.type === type);
  if (i >= 0) {
    if (openApps[i].title !== title) {
      openApps = openApps.map((a) => (a.type === type ? { ...a, title } : a));
      save(LS_OPEN, openApps);
    }
    return;
  }
  openApps = [...openApps, { type, title }];
  save(LS_OPEN, openApps);
}

/// Close an app. Returns the app that should be shown instead when the closed one
/// was active — the caller navigates, since navigation lives in the panel store.
export function closeApp(type: string): OpenApp | null {
  const i = openApps.findIndex((a) => a.type === type);
  if (i < 0) return null;
  openApps = openApps.filter((a) => a.type !== type);
  save(LS_OPEN, openApps);
  if (openApps.length === 0) return null;
  // Prefer the neighbour to the left, matching editor-tab behaviour.
  return openApps[Math.max(0, i - 1)] ?? null;
}

export function isOpen(type: string): boolean {
  return openApps.some((a) => a.type === type);
}

// ── Pinning ──
//
// The shell's layout store owns the pins now (Menu · Items|Chat · Details).
// These keep their old names so callers written for the rail and the context
// list go on working: the rail is the Menu stack, the context list is the Items
// face of the middle stack.

export function isRailPinned(): boolean {
  return L.isPinned('menu');
}

export function toggleRailPinned() {
  L.togglePin('menu');
}

export function isContextPinned(): boolean {
  return L.docked().mid;
}

export function toggleContextPinned() {
  L.togglePin('mid');
}

export function isContextDismissed(): boolean {
  return false;
}

/// The middle stack is popped in over the page.
export function isContextOpen(): boolean {
  return L.getOverlay() === 'mid';
}

/// Show the app's list: the Items face, popped in when it is not docked.
export function openContextPane() {
  L.showMid('items');
}

export function closeContextPane() {
  if (L.getOverlay() === 'mid') L.closeOverlay();
}

/// An item was picked from the list: a popped-in list steps aside for Details.
export function settleContextPane() {
  L.revealDetails();
}

/// Whether the app's list can be seen right now.
export function contextVisible(type: string | undefined | null, _hasSelection: boolean): boolean {
  return hasContext(type) && L.isVisible('mid') && L.getFace() === 'items';
}

// ── Pins ──
//
// Server-owned, per user, so a pin follows you across devices exactly as the
// conversations themselves do. Loaded once per session and kept here so every
// surface (home page, context list) reads one copy.
let pins = $state<Record<string, string[]>>({});
let pinsLoaded = false;

export function getPins(kind: string): string[] {
  return pins[kind] ?? [];
}

export function isPinned(kind: string, id: string): boolean {
  return (pins[kind] ?? []).includes(id);
}

export function setPins(next: Record<string, string[]>) {
  pins = next ?? {};
}

export function pinsAreLoaded(): boolean {
  return pinsLoaded;
}

export function markPinsLoaded() {
  pinsLoaded = true;
}

// ── Context list freshness ──
//
// The list loads when the app changes; it has no way to know that the app just
// created something (a first chat turn writes a new session row). A revision
// counter the page bumps is cheaper and more precise than polling.
let contextRev = $state(0);

export function contextRev_(): number {
  return contextRev;
}

/// The context changed (project, platform, something published or exposed):
/// every page and list reloads.
export function bumpContext() {
  contextRev += 1;
}

// An Items list changed under it (a chat turn wrote a conversation, a runtime
// changed state): only the Items list re-reads. Pages must not reload on this —
// a reload replaced what they hold, unsaved edits included (a Template's spec
// was lost after every chat turn).
let itemsRev = $state(0);
export function itemsRev_(): number {
  return itemsRev;
}
export function bumpItems() {
  itemsRev += 1;
}

// ── First message of a brand-new conversation ──
//
// A bare `/chat` shows a centred composer and creates NOTHING; the session only
// exists once you send. But sending has to navigate to `/chat/<id>`, which
// remounts the page — so the typed message is handed over here rather than lost
// in the unmount. Module state, never persisted: it is in flight for one tick.
let pendingFirstMessage: string | null = null;

export function setPendingMessage(text: string) {
  pendingFirstMessage = text;
}

/// Read once and clear — a handover, not a store.
export function takePendingMessage(): string | null {
  const t = pendingFirstMessage;
  pendingFirstMessage = null;
  return t;
}

export function getContextW(): number {
  return contextW;
}

export function setContextW(w: number) {
  contextW = Math.min(CONTEXT_MAX, Math.max(CONTEXT_MIN, Math.round(w)));
  save(LS_CTX_W, contextW);
}
