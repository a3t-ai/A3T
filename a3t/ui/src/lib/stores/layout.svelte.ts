// The shell's layout: three stacks side by side under the top bar.
//
//   Menu · Items|Chat · Details
//
// Each stack pins in or out as a whole. A pinned stack takes its place in the
// row; an unpinned one waits where it was and pops in over the page from there:
// the menu behind its shoulder arrow, Items|Chat behind a button beside the
// Details heading, Details behind an edge tab on the right. One of Items|Chat and Details always stays
// pinned, so the page is never empty. Expand gives one stack the page and turns
// the other into a pop-in without changing the pins.
//
// The middle stack has two faces that share one column: Items (the app's list)
// and Chat (the one thread). The message box lives only in Chat.
//
// Pins, widths and the face are per browser (localStorage); the open pop-in,
// the expanded stack and the palette are transient.

export type Stack = 'menu' | 'mid' | 'details';
export type MidFace = 'items' | 'chat';

const LS = 'a3t.shell.layout.v1';

export const LIMITS = {
  menu: [72, 260],
  mid: [320, 880],
  chatPop: [340, 820],
  detailsPop: [380, 1000],
} as const;
export const DEFAULTS = { menu: 104, mid: 440, chatPop: 460, detailsPop: 560 };
type WidthKey = keyof typeof DEFAULTS;

/// Labels show in the menu from this width; below it the menu is icons only.
export const MENU_LABELS_AT = 150;

interface Saved {
  pins: Record<Stack, boolean>;
  w: Record<WidthKey, number>;
  face: MidFace;
  chat: string;
}

function load(): Saved {
  const base: Saved = { pins: { menu: true, mid: true, details: true }, w: { ...DEFAULTS }, face: 'items', chat: '' };
  if (typeof window === 'undefined') return base;
  try {
    const raw = localStorage.getItem(LS);
    if (!raw) return base;
    const v = JSON.parse(raw) as Partial<Saved>;
    return {
      pins: { ...base.pins, ...(v.pins ?? {}) },
      w: { ...base.w, ...(v.w ?? {}) },
      face: v.face === 'chat' ? 'chat' : 'items',
      chat: typeof v.chat === 'string' ? v.chat : '',
    };
  } catch {
    return base;
  }
}

const saved = load();
// The invariant also holds for whatever an older or hand-edited value says.
if (!saved.pins.mid && !saved.pins.details) saved.pins.details = true;

let pins = $state<Record<Stack, boolean>>(saved.pins);
let widths = $state<Record<WidthKey, number>>(saved.w);
let face = $state<MidFace>(saved.face);
/// The Chat face's conversation ('' = the landing composer). One thread for
/// everything, whichever app is open beside it.
let chatSession = $state<string>(saved.chat);
/// The stack shown as a pop-in right now, if any.
let overlay = $state<Stack | null>(null);
/// The stack given the whole page by Expand, if any.
let focus = $state<'mid' | 'details' | null>(null);
let paletteOpen = $state(false);
/// A phone-width window: no room for stacks side by side. The menu is reached
/// from the Apps button and only Details is docked; the saved pins are kept for
/// when the window is wide again.
export const NARROW_PX = 720;
let narrow = $state(false);
if (typeof window !== 'undefined' && window.matchMedia) {
  const mq = window.matchMedia(`(max-width: ${NARROW_PX}px)`);
  narrow = mq.matches;
  mq.addEventListener?.('change', (e) => { narrow = e.matches; overlay = null; });
}
let toast = $state('');
let toastTimer: ReturnType<typeof setTimeout> | null = null;

function persist() {
  if (typeof window === 'undefined') return;
  try {
    localStorage.setItem(LS, JSON.stringify({ pins, w: widths, face, chat: chatSession }));
  } catch {
    /* blocked storage: the layout still works, it just forgets */
  }
}

export const STACK_NAME: Record<Stack, string> = { menu: 'Menu', mid: 'Items | Chat', details: 'Details' };

// ── Reads ──

export function isPinned(s: Stack): boolean { return pins[s]; }
export function isNarrow(): boolean { return narrow; }
/// The menu is in the row: pinned, and the window has room for it.
export function menuDocked(): boolean { return pins.menu && !narrow; }
export function getFace(): MidFace { return face; }
export function getOverlay(): Stack | null { return overlay; }
export function getFocus(): 'mid' | 'details' | null { return focus; }
export function width(k: WidthKey): number { return widths[k]; }
export function isPaletteOpen(): boolean { return paletteOpen; }
export function getToast(): string { return toast; }
export function getChatSession(): string { return chatSession; }

/// Whether Items|Chat and Details are laid out in the row (not pop-ins).
/// Expand overrides the pins without changing them.
export function docked(): { mid: boolean; details: boolean } {
  if (narrow) return focus === 'mid' ? { mid: true, details: false } : { mid: false, details: true };
  if (focus === 'mid') return { mid: true, details: false };
  if (focus === 'details') return { mid: false, details: true };
  return { mid: pins.mid, details: pins.details };
}

/// Whether a stack can be seen right now, docked or popped in.
export function isVisible(s: Stack): boolean {
  if (s === 'menu') return menuDocked() || overlay === 'menu';
  return docked()[s] || overlay === s;
}

// ── Writes ──

export function say(message: string) {
  toast = message;
  if (toastTimer) clearTimeout(toastTimer);
  toastTimer = setTimeout(() => { toast = ''; }, 2200);
}

export function togglePin(s: Stack) {
  focus = null;
  // Pinning the stack that is popped in keeps it where it is, now docked.
  const wasOverlay = overlay === s;
  overlay = null;
  if (wasOverlay && !pins[s]) {
    pins = { ...pins, [s]: true };
    persist();
    say(`${STACK_NAME[s]} pinned in`);
    return;
  }
  const next = { ...pins, [s]: !pins[s] };
  if (!next.mid && !next.details) {
    const other: Stack = s === 'mid' ? 'details' : 'mid';
    next[other] = true;
    pins = next;
    persist();
    say(`One of Items | Chat and Details stays pinned, so ${STACK_NAME[other]} is pinned back in`);
    return;
  }
  pins = next;
  persist();
  say(next[s] ? `${STACK_NAME[s]} pinned in` : `${STACK_NAME[s]} unpinned: ${s === 'menu' ? 'it pops in from the left' : s === 'mid' ? 'its arrow is on the left edge of Details' : 'its arrow is on the right edge'}`);
}

export function toggleExpand(s: 'mid' | 'details') {
  overlay = null;
  focus = focus === s ? null : s;
  say(focus ? `${STACK_NAME[s]} expanded: the other stack pops in when needed` : 'Restored');
}

export function openOverlay(s: Stack) { overlay = s; }
export function closeOverlay() { overlay = null; }

export function setFace(f: MidFace) {
  face = f;
  persist();
}

/// Show the middle stack on a face: switch to it, and pop the stack in when it
/// is not docked.
export function showMid(f: MidFace) {
  setFace(f);
  if (!docked().mid) overlay = 'mid';
}

/// Ctrl J: bring Items|Chat into view, or swap its face when it already is.
export function toggleFace() {
  if (!isVisible('mid')) { overlay = 'mid'; return; }
  setFace(face === 'chat' ? 'items' : 'chat');
}

/// Something was selected: make sure Details is where the reader can see it.
export function revealDetails() {
  if (!docked().details) overlay = 'details';
  else if (overlay === 'mid') overlay = null;
}

/// Switch the Chat face to a conversation; '' starts a new one.
export function setChatSession(id: string) {
  if (id === chatSession) return;
  chatSession = id;
  persist();
}

export function setWidth(k: WidthKey, v: number) {
  const [lo, hi] = LIMITS[k];
  widths = { ...widths, [k]: Math.round(Math.max(lo, Math.min(hi, v))) };
  persist();
}
export function resetWidth(k: WidthKey) { setWidth(k, DEFAULTS[k]); }

export function openPalette() { paletteOpen = true; overlay = null; }
export function closePalette() { paletteOpen = false; }
export function togglePalette() { paletteOpen ? closePalette() : openPalette(); }

/// Esc: close whatever floats, innermost first.
export function escape(): boolean {
  if (paletteOpen) { paletteOpen = false; return true; }
  if (overlay) { overlay = null; return true; }
  return false;
}
