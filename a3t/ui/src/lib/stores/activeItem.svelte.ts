// What the user is working on, for the chat: app → selected item → the item's
// config as it stands in the editor.
//
// Every chat in the UI talks to the A3T UI Agent, which keeps the sessions and
// their episodic memory. With each turn the chat sends this as `ui_context`, so
// "this", "here" and "make it shorter" resolve to what is on screen — including
// edits not saved yet, which a get_* tool would not see.
//
// Three levels, each filled by whoever knows it:
//  - the app: the shell (always);
//  - the selected item: the shell from the address, refined by the item view,
//    which knows what the item really is (kind, project, name, id);
//  - the config: the item view — its tab on screen and its config text. The
//    text is compared with the copy taken when the view last loaded or saved
//    it (`rev`), and only an unsaved difference is sent in full.

export interface ActiveItemView {
  kind: string;
  namespace?: string;
  name: string;
  /// A new, unsaved item passes '(new)'.
  id?: string;
  /// The tab or section on screen.
  section?: string;
  /// The config as edited, serialised (YAML or JSON).
  text?: string;
  /// Bump whenever the view (re)loads or saves the config: the text at that
  /// moment is the saved copy that later edits are compared with.
  rev?: number | string;
  /// When the view knows exactly whether its text differs from the stored copy
  /// (the Studio does), it says so and `rev` is not needed.
  dirty?: boolean;
  /// The view accepts the agent's edits: `propose_edit` hands back a complete
  /// new text, which this puts in the editor, unsaved. The chat offers Undo.
  applyEdit?: (text: string) => void;
}

interface Published extends ActiveItemView {
  dirty: boolean;
}

let app = $state<{ type: string; title: string; selectedId: string } | null>(null);
let item = $state<{ owner: symbol; view: Published } | null>(null);

/// The shell, on every navigation.
export function setActiveApp(type: string, title: string, selectedId: string) {
  app = type ? { type, title, selectedId } : null;
}

export function getActiveItem(): Published | null {
  return item?.view ?? null;
}

/// Called once by an item view during init. `get` returns what is on screen,
/// or null when nothing is open; it is re-read whenever what it reads changes.
export function trackActiveItem(get: () => ActiveItemView | null) {
  const owner = Symbol('active-item');
  let baseKey = '';
  let base = '';
  $effect(() => {
    const v = get();
    if (!v) {
      if (item?.owner === owner) item = null;
      return;
    }
    // By id where there is one: a new item's name changes as it is typed.
    const key = `${v.kind}:${v.id ?? `${v.namespace ?? ''}/${v.name}`}:${v.rev ?? ''}`;
    if (key !== baseKey) { baseKey = key; base = v.text ?? ''; }
    item = { owner, view: { ...v, dirty: v.dirty ?? (v.text != null && v.text !== base) } };
  });
  $effect(() => () => {
    if (item?.owner === owner) item = null;
  });
}

/// Secret values never leave the browser in the context: they would land in the
/// agent's prompt and in its episodic memory. Any field whose name says it holds
/// a secret is blanked, at any depth; a `*_ref` names a secret, so it stays.
/// What a secret's value is replaced with.
const HIDDEN = '••• (hidden)';
const SECRET_KEY = /(^|_)(api_?key|secret|token|password|passwd|private_?key|credential)s?$/i;
function redact(v: unknown): unknown {
  if (Array.isArray(v)) return v.map(redact);
  if (v && typeof v === 'object') {
    return Object.fromEntries(Object.entries(v as Record<string, unknown>).map(([k, x]) =>
      [k, SECRET_KEY.test(k) && !/_ref$/i.test(k) && x !== '' && x != null ? HIDDEN : redact(x)]));
  }
  return v;
}
/// JSON is redacted field by field. Anything else (YAML) has `key: value`
/// lines with a secret-looking key blanked.
export function redactSecrets(text: string): string {
  try { return JSON.stringify(redact(JSON.parse(text)), null, 2); } catch { /* not JSON */ }
  return text.replace(/^(\s*-?\s*["']?([\w.-]+)["']?\s*:\s*)(.+)$/gm, (line, head, key, val) =>
    SECRET_KEY.test(key) && !/_ref$/i.test(key) && val.trim() ? `${head}"${HIDDEN}"` : line);
}

function restore(next: unknown, cur: unknown): unknown {
  if (next === HIDDEN) return cur;
  if (Array.isArray(next)) return next.map((x, i) => restore(x, Array.isArray(cur) ? cur[i] : undefined));
  if (next && typeof next === 'object') {
    const c = (cur && typeof cur === 'object' ? cur : {}) as Record<string, unknown>;
    return Object.fromEntries(Object.entries(next as Record<string, unknown>).map(([k, x]) => [k, restore(x, c[k])]));
  }
  return next;
}
/// The agent saw secrets blanked, so a whole document it hands back carries the
/// blank where a secret was. Put the current value back before it is applied —
/// otherwise accepting an edit would overwrite a real key with the placeholder.
export function restoreSecrets(next: string, current: string): string {
  if (!next.includes(HIDDEN)) return next;
  try { return JSON.stringify(restore(JSON.parse(next), JSON.parse(current)), null, 2); } catch { /* not JSON */ }
  const was = new Map<string, string>();
  for (const m of current.matchAll(/^(\s*-?\s*["']?([\w.-]+)["']?\s*:\s*)(.+)$/gm)) was.set(m[2], m[3]);
  return next.replace(/^(\s*-?\s*["']?([\w.-]+)["']?\s*:\s*)"?••• \(hidden\)"?\s*$/gm, (line, head, key) => (was.has(key) ? head + was.get(key) : line));
}

/// Cap on the unsaved text sent with a turn, so a large config cannot crowd
/// out the transcript; the server says when it was cut.
const DRAFT_MAX = 12000;

/// The `ui_context` for a turn from a chat beside an app (the Chat face).
export function activeUiContext(): Record<string, unknown> | undefined {
  const v = item?.view;
  // The app is sent with every turn: it is stored with the turn in the agent's
  // memory (ADB), so later sessions can recall what was said where.
  const where = app ? { app: app.title } : {};
  if (v) {
    // The item's text goes with the turn whether saved or not, so the agent sees
    // what is on screen without needing a tool to read it.
    const t = v.text != null ? redactSecrets(v.text) : null;
    return {
      ...where,
      kind: v.kind,
      name: v.namespace ? `${v.namespace}/${v.name}` : v.name,
      ...(v.section ? { section: v.section } : {}),
      ...(t == null ? {}
        : v.dirty ? { dirty: true, draft_text: t.slice(0, DRAFT_MAX), draft_truncated: t.length > DRAFT_MAX }
        : { text: t.slice(0, DRAFT_MAX), text_truncated: t.length > DRAFT_MAX }),
    };
  }
  if (!app) return undefined;
  // An item the address names but no view has described (yet): its app and id.
  if (app.selectedId) return { ...where, kind: `${app.title} item`, name: app.selectedId };
  // Nothing selected: the app itself.
  return { ...where, kind: 'page', name: app.title };
}
