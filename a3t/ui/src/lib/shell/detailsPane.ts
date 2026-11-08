// The Details pane as a host for item views.
//
// Pages open a selected item in a slide-over (SlidePanel, and Templates' own).
// On their own they cover the whole window from the right. Inside the shell an
// item belongs in the Details pane, so the shell provides the pane's element
// through context and a slide-over fits itself to it: same place and size as
// the pane, no backdrop, no slide across the window.

import { getContext, setContext } from 'svelte';

const KEY = Symbol('a3t.detailsPane');

export type PaneHost = () => HTMLElement | undefined;

/// Called by the shell around the pages it puts in Details.
export function provideDetailsPane(host: PaneHost) {
  setContext(KEY, host);
}

/// Called by a slide-over during init: the pane to fit, or null outside the shell.
export function detailsPane(): PaneHost | null {
  return getContext<PaneHost | undefined>(KEY) ?? null;
}

/// Svelte action: keep `node` (position: fixed) exactly over the pane, following
/// it as stacks are pinned, expanded or dragged. Does nothing without a host.
export function fitToPane(node: HTMLElement, host: PaneHost | null) {
  if (!host) return {};
  let raf = 0;
  const place = () => {
    const el = host();
    if (!el) return;
    const r = el.getBoundingClientRect();
    Object.assign(node.style, {
      top: `${r.top}px`, left: `${r.left}px`, width: `${r.width}px`, height: `${r.height}px`,
      right: 'auto', bottom: 'auto',
    });
  };
  const schedule = () => { cancelAnimationFrame(raf); raf = requestAnimationFrame(place); };
  place();
  const ro = new ResizeObserver(schedule);
  const el = host();
  if (el) ro.observe(el);
  window.addEventListener('resize', schedule);
  return {
    destroy() {
      cancelAnimationFrame(raf);
      ro.disconnect();
      window.removeEventListener('resize', schedule);
    },
  };
}
