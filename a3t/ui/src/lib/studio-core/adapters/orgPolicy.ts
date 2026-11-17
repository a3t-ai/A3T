/// An org's policy files, kept in A3T's database (`/security/orgs/{org}/…`).
/// In the Studio an org's files sit under `org:<handle>`, beside the projects.
import { orgPolicy, orgsApi, type OrgPolicyCheck, type OrgPolicyFileDetail, type OrgPolicyFiles } from '$lib/services/api';
import { fail, ok, type Result } from '../items';
import { result, safely } from './util';

export const ORG_PREFIX = 'org:';
export const orgOf = (project: string) => (project.startsWith(ORG_PREFIX) ? project.slice(ORG_PREFIX.length) : '');

/// The orgs whose policy files you manage (an org's admins and security admins,
/// SuperAdmin and SecurityAdmin), as the orgs flow says.
export const managedOrgs = (): Promise<Result<string[]>> =>
  safely('list orgs', async () => {
    const r = await orgsApi.list();
    if (!r.ok) return fail(r.error ?? 'orgs could not be read');
    return ok(((r.data?.orgs ?? []) as any[]).filter((o) => o.can_audit).map((o) => o.handle as string).sort());
  });

export const files = (org: string): Promise<Result<OrgPolicyFiles>> =>
  safely(`list ${org}'s policy files`, async () => result(await orgPolicy.files(org), `list ${org}'s policy files`));

export const file = (org: string, path: string): Promise<Result<OrgPolicyFileDetail>> =>
  safely(`load ${path}`, async () => result(await orgPolicy.file(org, path), `load ${path}`));

/// Saved, or why not: the server's reasons are part of the message.
export const save = (org: string, path: string, content: string): Promise<Result<{ version: string; changed: boolean; enforced: boolean }>> =>
  safely(`save ${path}`, async () => {
    const r = await orgPolicy.save(org, path, content);
    if (r.ok) return ok(r.data!);
    const errors = ((r as any).detail?.errors ?? []) as string[];
    return fail(errors.length ? `${r.error}: ${errors.join('; ')}` : (r.error ?? `save ${path} failed`));
  });

export const remove = (org: string, path: string): Promise<Result> =>
  safely(`remove ${path}`, async () => {
    const r = await orgPolicy.remove(org, path);
    if (r.ok) return ok(undefined);
    const errors = ((r as any).detail?.errors ?? []) as string[];
    return fail(errors.length ? `${r.error}: ${errors.join('; ')}` : (r.error ?? `remove ${path} failed`));
  });

export const validate = (org: string, drafts: { path: string; content: string }[]): Promise<Result<OrgPolicyCheck>> =>
  safely('validate', async () => result(await orgPolicy.validate(org, drafts), 'validate'));
