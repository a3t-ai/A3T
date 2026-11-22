/// A3T's roles, in one table (ID-1): `a3t/policies/roles.yaml`, served by
/// FlowR at `/security/roles`. The console shows role names and what each role
/// does from here, so they always match what the server enforces.
import { securityRoles } from '$lib/services/api';

export type RoleScope = 'cedar' | 'platform' | 'org' | 'project';
export type RoleRow = { id: string; scope: RoleScope; label: string; what: string; carries?: string[]; access?: string; rank?: number };

let rows = $state<RoleRow[]>([]);
let asked = false;

/// Load the table once (anyone signed in may read it). Readers below are
/// called from templates and `$derived`, where starting a request (which marks
/// the status bar busy) is not allowed: the request starts just after.
export function loadRoles(): void {
  if (asked) return;
  asked = true;
  queueMicrotask(() => void securityRoles().then((r) => {
    if (r.ok && Array.isArray(r.data?.roles)) rows = r.data!.roles as RoleRow[];
    else asked = false;
  }));
}

export function roles(scope: RoleScope): RoleRow[] {
  loadRoles();
  return rows.filter((r) => r.scope === scope);
}

/// A role's name, e.g. ('org', 'securityadmin') → "Org security admin".
export function roleLabel(scope: RoleScope, id: string, fallback = id): string {
  loadRoles();
  return rows.find((r) => r.scope === scope && r.id === id)?.label ?? fallback;
}

/// What a role does, in one sentence.
export function roleWhat(scope: RoleScope, id: string, fallback = ''): string {
  loadRoles();
  return rows.find((r) => r.scope === scope && r.id === id)?.what ?? fallback;
}
