// The item a chat conversation has open for editing with the agent.
//
// A card in the chat (a flow, agent, template…) opens the item in the editing
// pane: the user and the agent co-edit it — the agent sees the unsaved buffer
// and the section on screen, and its revisions land in the editor. The pane is
// drawn by the shell in Details (popped in over it, or pinned), not inside the
// chat, so it survives the conversation moving between Details and the middle
// stack. This store is how the two talk: the chat opens items and feeds
// revisions in; the pane reports its section and buffer, and hands back the
// messages it wants sent ("I saved…", "About this flow: …").
//
// One item at a time, owned by one conversation. Only the chat showing that
// conversation reads or writes it, and the pane shows only beside it.

import { initialSection, type ArtifactKind } from '$lib/components/chat/artifacts';

export interface ChatArtifact {
  kind: ArtifactKind;
  namespace: string;
  name: string;
  text: string;
  id?: string;
}

export interface ArtifactRevision {
  text: string;
  seq: number;
  /// Set when the agent edited the buffer (`propose_edit`) rather than saving a
  /// new version: applied to the editor unsaved, with Undo.
  edit?: string;
}

interface State {
  session: string;
  artifact: ChatArtifact;
  revision: ArtifactRevision | null;
  section: string;
  edit: { text: string; dirty: boolean } | null;
  /// Pinned: the pane takes Details and the conversation moves beside it.
  /// Unpinned: it pops in over Details.
  pinned: boolean;
}

let state = $state<State | null>(null);

export function getChatArtifact(): State | null {
  return state;
}

/// The open item, if it belongs to this conversation.
export function artifactFor(session: string | null | undefined): State | null {
  return session && state?.session === session ? state : null;
}

/// Open an item from a conversation's card. Opening another item of the same
/// conversation keeps the pin, so a pinned pane stays where the user put it.
export function openChatArtifact(session: string, artifact: ChatArtifact) {
  const keepPin = state?.session === session && state.pinned;
  state = { session, artifact, revision: null, section: initialSection(artifact.kind), edit: null, pinned: keepPin };
}

export function closeChatArtifact() {
  state = null;
}

export function setArtifactRevision(rev: ArtifactRevision) {
  if (state) state.revision = rev;
}

export function setArtifactSection(section: string) {
  if (state) state.section = section;
}

export function setArtifactEdit(edit: { text: string; dirty: boolean }) {
  if (state) state.edit = edit;
}

export function setArtifactPinned(pinned: boolean) {
  if (state) state.pinned = pinned;
}

// ── From the pane to its conversation ──
//
// `applied`: send this as a turn (the pane saved something the agent must know).
// `ask`: put this in the composer for the user to finish.
export interface PaneMessage {
  session: string;
  kind: 'applied' | 'ask';
  text: string;
  seq: number;
}
let outbox = $state<PaneMessage | null>(null);
let outSeq = 0;

export function sendFromPane(kind: 'applied' | 'ask', text: string) {
  if (!state) return;
  outbox = { session: state.session, kind, text, seq: ++outSeq };
}

/// The pending message for this conversation; the chat takes it once.
export function paneMessageFor(session: string | null | undefined): PaneMessage | null {
  return session && outbox?.session === session ? outbox : null;
}

export function takePaneMessage(seq: number) {
  if (outbox?.seq === seq) outbox = null;
}
