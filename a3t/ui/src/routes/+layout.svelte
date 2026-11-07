<script lang="ts">
  import '../app.css';
  import type { Snippet } from 'svelte';
  import { page } from '$app/stores';
  import { goto } from '$app/navigation';
  import Shell from '$lib/shell/Shell.svelte';
  import { trackOpen, setPins, markPinsLoaded } from '$lib/stores/apps.svelte.ts';
  import { getActivePanel } from '$lib/stores/panels.svelte.ts';
  import { untrack } from 'svelte';
  import { navigateTo, initFromUrl, setupPopState } from '$lib/stores/panels.svelte.ts';
  import { initContext } from '$lib/stores/context.svelte.ts';
  import PublishDialog from '$lib/components/shared/PublishDialog.svelte';
  import { initAuth, isAuthenticated } from '$lib/stores/auth';
  import { initEditorContext } from '$lib/editor/context';
  import { fetchPins } from '$lib/services/api';
  import { onMount } from 'svelte';

  let { children }: { children: Snippet } = $props();

  // Pins are server-owned and per user; load once so every surface reads one copy.
  onMount(() => {
    if (!authed) return;
    fetchPins().then((p) => { setPins(p); markPinsLoaded(); }).catch(() => {});
  });

  // Register the current app in the taskbar. Done here rather than at each call
  // site so the rail, the palette, a deep link and a back/forward all count.
  $effect(() => {
    const p = getActivePanel();
    if (!p) return;
    // untrack: trackOpen reads the open-app list it also writes, which would make
    // this effect retrigger itself and trip Svelte's update-depth guard.
    untrack(() => trackOpen(p.type, p.title));
  });

  let authChecked = $state(false);
  let authed = $state(false);

  const publicPaths = ['/login', '/signup'];

  onMount(() => {
    const hasSession = initAuth();
    authChecked = true;
    authed = hasSession;

    if (!hasSession && !publicPaths.includes(window.location.pathname)) {
      goto('/login');
      return;
    }

    if (hasSession) {
      // Restore from URL, else land on Chat — Dashboard is hidden from the menu,
      // so defaulting there would open a page the rail cannot get back to.
      // The project comes first: the panels read the rest of the same URL.
      initContext();
      const restored = initFromUrl();
      if (!restored) {
        navigateTo('chat', 'AI Chat');
      }
      setupPopState();
      // Initialize editor context (registry + ajv) in background
      initEditorContext().then(ctx => {
        if (ctx.error) {
          console.warn('[layout] Editor context init warning:', ctx.error);
        } else {
          console.log(`[layout] Editor context ready: ${ctx.functions.size} functions loaded`);
        }
      });
    }

    const unsub = isAuthenticated.subscribe(v => {
      authed = v;
      if (v && authChecked && publicPaths.includes(window.location.pathname)) {
        navigateTo('chat', 'AI Chat');
      } else if (!v && authChecked && !publicPaths.includes(window.location.pathname)) {
        goto('/login');
      }
    });
    return unsub;
  });

  // Derive if current route is public
  let isPublicRoute = $derived(publicPaths.some(p => $page.url.pathname.startsWith(p)));
</script>

<svelte:head>
  <title>A3T — Agent Platform</title>
</svelte:head>

{#if isPublicRoute}
  <!-- Public pages (login/signup) render without the shell -->
  {@render children()}
{:else if authed}
  <!-- Authenticated shell -->
  <Shell />
  <PublishDialog />
  {@render children()}
{:else if authChecked}
  <!-- Redirect in progress -->
  <div class="loading-screen">
    <span class="loading-logo">A3T</span>
    <span class="loading-text">Redirecting...</span>
  </div>
{:else}
  <!-- Checking auth -->
  <div class="loading-screen">
    <span class="loading-logo">A3T</span>
    <span class="loading-text">Loading...</span>
  </div>
{/if}

<style>
  .loading-screen {
    height: 100vh;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 12px;
    background: var(--bg-page);
  }
  .loading-logo {
    font-family: var(--font-serif);
    font-size: 32px;
    font-weight: 600;
    color: var(--text-1);
  }
  .loading-text {
    font-size: 13px;
    color: var(--text-4);
  }
</style>
