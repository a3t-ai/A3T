<script lang="ts">
  import { getPanels } from '$lib/stores/panels.svelte.ts';

  import ChatPage from '$lib/components/pages/ChatPage.svelte';
  import ChatPageAI from '$lib/components/pages/ChatPageAI.svelte';

  // The AI-SDK chat is the default; the previous implementation stays reachable
  // while it beds in. Flip with:  localStorage.setItem('a3t.chat.legacy','true')
  let useLegacyChat = $state(false);
  try { useLegacyChat = localStorage.getItem('a3t.chat.legacy') === 'true'; } catch {}
  import TemplatesList from '$lib/components/pages/TemplatesList.svelte';
  import AgentsList from '$lib/components/pages/AgentsList.svelte';
  import ResourcesList from '$lib/components/pages/ResourcesList.svelte';
  import TriggersList from '$lib/components/pages/TriggersList.svelte';
  import CanvasList from '$lib/components/pages/CanvasList.svelte';
  import InputsList from '$lib/components/pages/InputsList.svelte';
  import FlowEditor from '$lib/components/pages/FlowEditor.svelte';
  import ConsoleEditor from '$lib/components/pages/ConsoleEditor.svelte';
  import DbConsole from '$lib/components/pages/DbConsole.svelte';
  import KnowledgePage from '$lib/components/pages/KnowledgePage.svelte';
  import MemoryPage from '$lib/components/pages/MemoryPage.svelte';
  import DataRoomsPage from '$lib/components/pages/DataRoomsPage.svelte';
  import TestBench from '$lib/components/pages/TestBench.svelte';
  import EarlyAccessRequests from '$lib/components/pages/EarlyAccessRequests.svelte';
  import ErrorsPage from '$lib/components/pages/ErrorsPage.svelte';
  import SettingsPage from '$lib/components/pages/SettingsPage.svelte';
  import SecurityPage from '$lib/components/pages/SecurityPage.svelte';
  import ProjectsPage from '$lib/components/pages/ProjectsPage.svelte';
  import ExecutionsPage from '$lib/components/pages/ExecutionsPage.svelte';
  import LogsPage from '$lib/components/pages/LogsPage.svelte';
  import RuntimePage from '$lib/components/pages/RuntimePage.svelte';
  import Placeholder from '$lib/components/pages/Placeholder.svelte';

  /// Pages that open a newly picked item in place (they watch `selectedId`).
  const IN_PLACE = new Set(['resources', 'agents', 'templates', 'executions', 'canvas', 'projects', 'triggers']);

  let panels = $derived(getPanels());
  let activePanel = $derived(panels.length > 0 ? panels[panels.length - 1] : null);
</script>

<div class="screen-container">
  {#if !activePanel}
    <div class="empty-state">
      <h2>Welcome to A3T</h2>
      <p>Select a section from the sidebar to get started.</p>
    </div>
  {:else}
    <!-- Remount the page when the app changes or New is pressed — not when
         another item is picked. A pick used to remount (and reload) the whole
         page, which drew it before the item's view covered it: the flicker.
         These list pages open a newly picked item in place; the rest (Runtime,
         Settings, Security, Chat, Studio…) read the selection on mount, so a pick
         still remounts them. -->
    {#key `${activePanel.type}:${IN_PLACE.has(activePanel.type) ? '' : (activePanel.props?.selectedId ?? '')}:${activePanel.props?.create ?? ''}`}
    {#if activePanel.type === 'dashboard'}
      <Placeholder type="dashboard" />
    {:else if activePanel.type === 'chat'}
      {#if useLegacyChat}<ChatPage />{:else}<ChatPageAI />{/if}
    {:else if activePanel.type === 'templates' || activePanel.type === 'template-detail'}
      <TemplatesList />
    {:else if activePanel.type === 'agents' || activePanel.type === 'agent-detail'}
      <AgentsList />
    {:else if activePanel.type === 'resources'}
      <ResourcesList />
    {:else if activePanel.type === 'triggers'}
      <TriggersList />
    {:else if activePanel.type === 'canvas' || activePanel.type === 'canvas-detail'}
      <CanvasList />
    {:else if activePanel.type === 'inputs'}
      <InputsList />
    {:else if activePanel.type === 'flow-editor'}
      <FlowEditor />
    {:else if activePanel.type === 'console-editor'}
      <ConsoleEditor />
    {:else if activePanel.type === 'db-console'}
      <DbConsole />
    {:else if activePanel.type === 'datarooms'}
      <DataRoomsPage />
    {:else if activePanel.type === 'knowledge'}
      <KnowledgePage />
    {:else if activePanel.type === 'memory'}
      <MemoryPage />
    {:else if activePanel.type === 'test-bench'}
      <TestBench />
    {:else if activePanel.type === 'early-access'}
      <EarlyAccessRequests />
    {:else if activePanel.type === 'errors'}
      <ErrorsPage />
    {:else if activePanel.type === 'settings'}
      <SettingsPage />
    {:else if activePanel.type === 'security'}
      <SecurityPage />
    {:else if activePanel.type === 'projects'}
      <ProjectsPage />
    {:else if activePanel.type === 'executions'}
      <ExecutionsPage />
    {:else if activePanel.type === 'runtime' || activePanel.type === 'reconciliation'}
      <RuntimePage />
    {:else if activePanel.type === 'logs'}
      <LogsPage />
    {:else}
      <Placeholder type={activePanel.type} />
    {/if}
    {/key}
  {/if}
</div>

<style>
  .screen-container {
    display: flex;
    flex-direction: column;
    height: 100%;
    overflow: hidden;
  }
  .empty-state {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    flex: 1;
    color: var(--text-4);
    gap: 8px;
  }
  .empty-state h2 {
    font-family: var(--font-serif);
    font-size: 28px;
    color: var(--text-3);
    font-weight: 500;
  }
  .empty-state p { font-size: 14px; }
</style>
