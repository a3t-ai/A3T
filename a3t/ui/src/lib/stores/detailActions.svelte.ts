// Actions of the item open in the Details pane (save, open in Studio, close).
//
// One bar per stack: an item view shown in the pane hands its actions to the
// Details stack bar instead of drawing a second toolbar under it. The view
// registers while it is open and clears on close; the shell's bar draws them.

export interface DetailActions {
  onsave?: () => void;
  onopenineditor?: () => void;
  onclose?: () => void;
  /// Further actions, shown as plain buttons before Save.
  extra?: { label: string; run: () => void }[];
}

let current = $state<{ owner: symbol; actions: DetailActions } | null>(null);

export function getDetailActions(): DetailActions | null {
  return current?.actions ?? null;
}

/// Register a view's actions; returns the function that clears them, and only
/// them, so a view closing late never clears the next one's.
export function setDetailActions(actions: DetailActions): () => void {
  const owner = Symbol('detail-actions');
  current = { owner, actions };
  return () => {
    if (current?.owner === owner) current = null;
  };
}

// The Items list's Refresh, drawn in the Items | Chat stack bar.
let itemsRefresh = $state<{ owner: symbol; run: () => void; busy: boolean } | null>(null);

export function getItemsRefresh(): { run: () => void; busy: boolean } | null {
  return itemsRefresh;
}

/// Register the list's refresh; returns the clear function (only clears its own).
export function setItemsRefresh(run: () => void, busy: boolean): () => void {
  const owner = Symbol('items-refresh');
  itemsRefresh = { owner, run, busy };
  return () => {
    if (itemsRefresh?.owner === owner) itemsRefresh = null;
  };
}

// Whether an item header is showing the actions (DetailActionsBar). When none
// is, the Details stack bar shows them instead, so they are never lost.
let hosts = $state(0);
export function actionsHosted(): boolean {
  return hosts > 0;
}
export function hostActions(): () => void {
  hosts++;
  let done = false;
  return () => { if (!done) { done = true; hosts--; } };
}
