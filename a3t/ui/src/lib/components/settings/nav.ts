// The Settings pages, grouped: one list for the Settings page and for the
// shell's Items (each leaf is an item; the group is its heading).

export interface Leaf { key: string; label: string; icon: string; ee?: boolean }
export interface Group { label: string; icon: string; ee?: boolean; items: Leaf[] }

export const NAV: Group[] = [
  { label: 'General', icon: 'general', items: [
    { key: 'general.platform', label: 'Platform', icon: 'box' },
    { key: 'general.runtime', label: 'Runtime', icon: 'swap' },
    { key: 'general.appearance', label: 'Appearance', icon: 'palette' },
    { key: 'general.orgs', label: 'Organisations', icon: 'users' },
  ]},
  { label: 'Security', icon: 'shield', ee: true, items: [
    { key: 'security.authn', label: 'Authentication', icon: 'key', ee: true },
    { key: 'security.authz', label: 'Authorization', icon: 'lockcheck', ee: true },
    { key: 'security.policy', label: 'Policy Engine', icon: 'scale', ee: true },
    { key: 'security.audit', label: 'Audit Log', icon: 'audit', ee: true },
    { key: 'security.monitoring', label: 'Monitoring', icon: 'pulse', ee: true },
    { key: 'security.compliance', label: 'Compliance', icon: 'badge', ee: true },
  ]},
  { label: 'Identity & Access', icon: 'users', ee: true, items: [
    { key: 'iam.users', label: 'Users', icon: 'user', ee: true },
    { key: 'iam.roles', label: 'Roles', icon: 'usercog', ee: true },
    { key: 'iam.idp', label: 'Identity Providers', icon: 'plug', ee: true },
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
    { key: 'advanced.apikeys', label: 'API Keys', icon: 'key' },
    { key: 'advanced.flags', label: 'Feature Flags', icon: 'flag' },
    { key: 'advanced.telemetry', label: 'Telemetry', icon: 'pulse' },
  ]},
];
