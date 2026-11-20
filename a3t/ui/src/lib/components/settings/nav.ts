// The Settings pages, grouped: one list for the Settings page and for the
// shell's Items (each leaf is an item; the group is its heading).
//
// UI-1 (20 Nov 2026): no security or identity control lives here unless the
// server enforces it. Sign-in, roles, policies, the decision log and agent keys
// are under Security, Access and each agent's own pages; the mock pages that
// saved settings nothing read (Authentication, Authorization, Policy Engine,
// Audit Log, Monitoring, Compliance, Users, Roles, Identity Providers, API Keys)
// are gone.

export interface Leaf { key: string; label: string; icon: string; ee?: boolean }
export interface Group { label: string; icon: string; ee?: boolean; items: Leaf[] }

export const NAV: Group[] = [
  { label: 'General', icon: 'general', items: [
    { key: 'general.platform', label: 'Platform', icon: 'box' },
    { key: 'general.runtime', label: 'Runtime', icon: 'swap' },
    { key: 'general.appearance', label: 'Appearance', icon: 'palette' },
    { key: 'general.orgs', label: 'Organisations', icon: 'users' },
  ]},
  { label: 'Infrastructure', icon: 'server', items: [
    { key: 'infra.clusters', label: 'Clusters', icon: 'nodes' },
    { key: 'infra.storage', label: 'Storage & Data', icon: 'database' },
    { key: 'infra.backend', label: 'Storage Backend', icon: 'database', ee: true },
    { key: 'infra.platformdb', label: 'Platform Database', icon: 'database', ee: true },
    { key: 'infra.adbembedding', label: 'ADB Embeddings', icon: 'database', ee: true },
    { key: 'infra.adbcache', label: 'ADB Memory Cache', icon: 'database', ee: true },
    { key: 'infra.logs', label: 'Logs & Retention', icon: 'audit' },
    { key: 'infra.registry', label: 'Image Registry', icon: 'box' },
  ]},
  { label: 'Integrations', icon: 'plug', items: [
    { key: 'integrations.notifications', label: 'Notifications', icon: 'bell' },
    { key: 'integrations.webhooks', label: 'Webhooks', icon: 'webhook' },
  ]},
  { label: 'Billing', icon: 'billing', ee: true, items: [
    { key: 'billing.plan', label: 'Plan & Usage', icon: 'chart', ee: true },
    { key: 'billing.payment', label: 'Payment Methods', icon: 'card', ee: true },
    { key: 'billing.invoices', label: 'Invoices', icon: 'receipt', ee: true },
  ]},
  { label: 'Advanced', icon: 'sliders', items: [
    { key: 'advanced.flags', label: 'Feature Flags', icon: 'flag' },
    { key: 'advanced.telemetry', label: 'Telemetry', icon: 'pulse' },
  ]},
];
