/// An org's policy files, kept in A3T's database (`/security/orgs/{org}/…`).
/// In the Studio they show inside each of the org's projects (a ref's project is
/// still `org:<handle>`: the files belong to the org, not to one project).
import { orgPolicy, orgsApi, projectsApi, type OrgPolicyCheck, type OrgPolicyFileDetail, type OrgPolicyFiles, type ReplayResult } from '$lib/services/api';

export type { ReplayResult };
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

/// The org that owns each project you can see (`{ namespace: org }`); projects a
/// user owns are left out: their files are an org's.
export const orgOfProjects = (): Promise<Result<Record<string, string>>> =>
  safely('list projects', async () => {
    const r = await projectsApi.list();
    if (!r.ok) return fail(r.error ?? 'projects could not be read');
    const out: Record<string, string> = {};
    for (const p of ((r.data as any)?.projects ?? []) as any[]) if (p.owner_kind === 'org' && p.owner) out[p.namespace] = p.owner;
    return ok(out);
  });

export const files = (org: string): Promise<Result<OrgPolicyFiles>> =>
  safely(`list ${org}'s policy files`, async () => result(await orgPolicy.files(org), `list ${org}'s policy files`));

export const file = (org: string, path: string): Promise<Result<OrgPolicyFileDetail>> =>
  safely(`load ${path}`, async () => result(await orgPolicy.file(org, path), `load ${path}`));

/// Saved, or why not: the server's reasons are part of the message.
export const save = (org: string, path: string, content: string): Promise<Result<{ version: string; changed: boolean; enforced: boolean }>> =>
  safely(`save ${path}`, async () => {
    const r = await orgPolicy.save(org, path, content);
    // GOV-3: held for a second approver — nothing is saved yet, so say so.
    if (r.ok && (r.data as any)?.pending) return fail((r.data as any).message ?? 'Held for a second approver.');
    if (r.ok) return ok(r.data!);
    const errors = ((r as any).detail?.errors ?? []) as string[];
    return fail(errors.length ? `${r.error}: ${errors.join('; ')}` : (r.error ?? `save ${path} failed`));
  });

export const remove = (org: string, path: string): Promise<Result> =>
  safely(`remove ${path}`, async () => {
    const r = await orgPolicy.remove(org, path);
    if (r.ok && (r.data as any)?.pending) return fail((r.data as any).message ?? 'Held for a second approver.');
    if (r.ok) return ok(undefined);
    const errors = ((r as any).detail?.errors ?? []) as string[];
    return fail(errors.length ? `${r.error}: ${errors.join('; ')}` : (r.error ?? `remove ${path} failed`));
  });

/// The org's enforced set with `drafts` in place and `removed` taken out: what
/// a save, or a removal, would leave.
/// GOV-2: how the org's recent decisions would have gone with this change.
/// A refusal says the server's reasons (the files' errors), not only a status.
export const replay = (org: string, change: { files?: { path: string; content: string }[]; removed?: string[]; switch_on?: string[]; days?: number }): Promise<Result<ReplayResult>> =>
  safely('replay', async () => {
    const r = await orgPolicy.replay(org, change);
    if (r.ok) return ok(r.data as ReplayResult);
    const errors: string[] | undefined = (r as any).detail?.errors;
    return errors?.length ? fail(errors.join('; ')) : result<ReplayResult>(r, 'replay');
  });

export const validate = (org: string, drafts: { path: string; content: string }[], removed: string[] = []): Promise<Result<OrgPolicyCheck>> =>
  safely('validate', async () => result(await orgPolicy.validate(org, drafts, removed), 'validate'));
