/**
 * Menu configuration with role-based access control
 *
 * Access rules:
 *   superadmin — sees all menu items (platform-level access)
 *   admin      — sees all menu items
 *   (per-item roles below; Security and the Decision log: SuperAdmin and SecurityAdmin only)
 */

import type { UserRole, MenuGroup } from './types';

const ALL_ROLES: UserRole[] = ['superadmin', 'securityadmin', 'admin', 'operator', 'viewer'];
const OPERATOR_PLUS: UserRole[] = ['superadmin', 'admin', 'operator'];
/// Platform InfoSec: Security and the Decision log are theirs alone (the server
/// refuses anyone else too).
const INFOSEC: UserRole[] = ['superadmin', 'securityadmin'];

// Chat and Projects on top; agents and what they are built from; what they
// know and keep; security and audit; runtime and logs; the Studio and
// Settings at the bottom. The console's main users are security and
// governance teams — developers build in VS Code with Claude, over MCP.
export const MENU_CONFIG: MenuGroup[] = [
  {
    label: 'Home',
    roles: ALL_ROLES,
    items: [
      { type: 'chat',       label: 'AI Chat',    tag: null, icon: 'chat',    roles: ALL_ROLES },
      { type: 'projects',   label: 'Projects',   tag: null, icon: 'project', roles: ALL_ROLES },
    ],
  },
  {
    label: 'Agents',
    roles: ALL_ROLES,
    items: [
      { type: 'agents',     label: 'Agents',     tag: 'pods',  icon: 'agent',    roles: ALL_ROLES },
      { type: 'templates',  label: 'Templates',  tag: 'sw/v1', icon: 'grid',     roles: ALL_ROLES },
      { type: 'triggers',   label: 'Triggers',   tag: null,    icon: 'trigger',  roles: ALL_ROLES },
      { type: 'canvas',     label: 'Canvas',     tag: 'crd',   icon: 'canvas',   roles: ALL_ROLES },
      { type: 'resources',  label: 'Resources',  tag: null,    icon: 'cabinet',  roles: ALL_ROLES },
    ],
  },
  {
    label: 'Knowledge & data',
    roles: ALL_ROLES,
    items: [
      { type: 'datarooms',  label: 'Files', tag: null, icon: 'room',   roles: ALL_ROLES },
      { type: 'memory',     label: 'Memory',     tag: null, icon: 'memory', roles: ALL_ROLES },
      { type: 'knowledge',  label: 'Knowledge',  tag: null, icon: 'kg',     roles: ALL_ROLES },
    ],
  },
  {
    label: 'Security & audit',
    roles: ALL_ROLES,
    items: [
      { type: 'security',   label: 'Security',     tag: 'ee', icon: 'shield', roles: INFOSEC },
      { type: 'logs',       label: 'Decision log', tag: null, icon: 'decision', roles: INFOSEC },
    ],
  },
  {
    label: 'Runtime & logs',
    roles: ALL_ROLES,
    items: [
      { type: 'runtime',    label: 'Runtimes',   tag: null, icon: 'runtime', roles: ALL_ROLES },
      { type: 'executions', label: 'Executions', tag: null, icon: 'runs',    roles: ALL_ROLES },
      // The A3T error log (UX ST-1): the flow lets Admin and SuperAdmin read it.
      { type: 'errors',     label: 'Errors',     tag: null, icon: 'alert',   roles: ['superadmin', 'admin'] },
    ],
  },
  {
    // Pinned to the foot of the menu, always in view: the Studio (for when
    // VS Code is not at hand; /db-console still opens it pinned to databases)
    // and Settings.
    label: 'Settings',
    roles: ALL_ROLES,
    pinned: true,
    items: [
      { type: 'flow-editor', label: 'Studio',   tag: null, icon: 'studio',   roles: ALL_ROLES },
      { type: 'settings',    label: 'Settings', tag: null, icon: 'settings', roles: ALL_ROLES },
    ],
  },
];

/**
 * Filter menu groups and items for the given role.
 * Removes empty groups after filtering.
 */
export function getMenusForRole(role: UserRole): MenuGroup[] {
  return MENU_CONFIG
    .filter(group => group.roles.includes(role))
    .map(group => ({
      ...group,
      items: group.items.filter(item => item.roles.includes(role)),
    }))
    .filter(group => group.items.length > 0);
}
