/// An org's policy file (policies/, roles/, access/ YAML), kept in A3T's
/// database and edited here like any other YAML: as code in the center, checked
/// against the org's other enforced files on the right (Validate), every saved
/// version below (Versions). Its ref: `{ project: 'org:<handle>', kind: 'policy',
/// name: '<dir>/<name>.yaml' }`. A save is refused (with the reasons) if the
/// org's enforced files wouldn't work with it.
import * as orgPolicy from '../adapters/orgPolicy';
import { fail, ok } from '../items';
import type { Doc } from '../workspace';
import type { ItemKind } from './types';

export type PolicyDir = 'policies' | 'roles' | 'access' | 'egress';
const PATH = /^(policies|roles|access|egress)\/[A-Za-z0-9_-]{1,80}\.ya?ml$/;

/// What a new file starts as: a working example of its kind, to change.
/// `project`: the project it is made in (from a project's Policies folder).
export const STARTERS: Record<PolicyDir, (name: string, org: string, project?: string) => string> = {
  policies: (name, org, project) => `kind: a3t/policy
name: ${name}
description: What this rule stops, in one sentence.
# Where it applies: the whole org, or some of its projects.
applies_to: ${project ? `{ projects: [${project}] }` : '{ org: true }'}
rules:
  # Who it binds (leave "who" out for everyone), what they cannot do, and why.
  - who: { roles: [Operator, Viewer] }
    cannot: [flows.delete]
    reason: Only Admins delete flows in ${org}.
`,
  roles: (name) => `kind: a3t/role
name: ${name}
description: What this role is for.
# Start from a baseline role (Admin, Operator, Viewer) and take permissions away.
extends: Operator
remove: [memory.change]
`,
  access: (_name, org, project) => `kind: a3t/access
project: ${project ?? `${org}-<project>`}
description: Who holds which role on this project.
# Sign-in groups and named people: role (a baseline role or one of the org's roles/ files).
groups:
  ${org}-support: Operator
users:
  someone@example.com: Viewer
`,
  egress: (name, org) => `kind: a3t/egress
description: Where ${org}'s flows and agents may call out to.
# With this file on, only these hosts are reached (calls out of A3T: http, remote MCP servers, outgoing webhooks).
hosts:
  - api.github.com
  - "*.slack.com"
# Hosts or ranges on ${org}'s own network that may be reached although private (never needed for public hosts).
private: []
`,
};

const SINGULAR: Record<PolicyDir, string> = { policies: 'policy', roles: 'role', access: 'access', egress: 'egress' };
const loadedText = (doc: Doc, text: string, data: any) => { doc.text = text; doc.saved = text; doc.data = data; doc.error = undefined; };

export const policyKind: ItemKind = {
  id: 'policy', label: 'Policy file', icon: '§',
  match: (it) => it.kind === 'policy',
  capabilities: ['edit', 'create', 'delete'],
  format: 'yaml',
  views: {
    center: 'code', code: 'code',
    right: [{ id: 'policy-check', title: 'Validate' }],
    bottom: [{ id: 'policy-versions', title: 'Versions' }],
  },
  console: { section: 'security', title: 'Security' },
  /// `type`: the folder (`policies`, `roles`, `access`), and after a `:` the
  /// project it is made in, which the starter names (`access:acme-p1`).
  create(project, n, type = 'policies') {
    const [folder, inProject] = type.split(':');
    const dir = (['policies', 'roles', 'access', 'egress'].includes(folder) ? folder : 'policies') as PolicyDir;
    const base = inProject && dir === 'access' ? inProject : `new-${SINGULAR[dir]}${n > 1 ? `-${n}` : ''}`;
    return { ref: { project, kind: 'policy', name: `${dir}/${base}.yaml` }, text: STARTERS[dir](base, orgPolicy.orgOf(project), inProject || undefined), type: dir };
  },
  checkName: (n) => (PATH.test(n) ? '' : 'A path: policies/<name>.yaml, roles/<name>.yaml, access/<name>.yaml or egress/<name>.yaml (letters, digits, - and _).'),
  async load(doc) {
    const org = orgPolicy.orgOf(doc.ref.project);
    if (!org) { doc.error = `${doc.ref.project} is not an org`; return; }
    const r = await orgPolicy.file(org, doc.ref.name);
    if (!r.ok) { doc.error = r.error; return; }
    if (!r.data.file) { doc.error = `${doc.ref.name} was removed (its versions are kept: see Versions)`; doc.data = r.data; return; }
    loadedText(doc, r.data.file.content, r.data);
  },
  async push(doc) {
    const org = orgPolicy.orgOf(doc.ref.project);
    if (!org) return fail(`${doc.ref.project} is not an org`);
    if (doc.isNew && !PATH.test(doc.ref.name)) return fail(`Name it: ${policyKind.checkName!(doc.ref.name)}`);
    const r = await orgPolicy.save(org, doc.ref.name, doc.text ?? '');
    if (!r.ok) return r;
    doc.saved = doc.text;
    const back = await orgPolicy.file(org, doc.ref.name);
    if (back.ok) doc.data = back.data;
    return ok({ ...doc.ref });
  },
  dirty: (doc) => !!doc.isNew || (doc.loaded && (doc.text ?? '') !== (doc.saved ?? '')),
  /// Every saved version, newest first; the one saved now is "active".
  async versions(doc) {
    const r = await orgPolicy.file(orgPolicy.orgOf(doc.ref.project), doc.ref.name);
    if (!r.ok) return r;
    const saves = r.data.history.filter((h) => h.action === 'create' || h.action === 'save').map((h) => h.version);
    return ok({ versions: [...new Set(saves)], active: r.data.file?.version ?? '' });
  },
  async loadVersion(doc, version) {
    const r = await orgPolicy.file(orgPolicy.orgOf(doc.ref.project), doc.ref.name);
    if (!r.ok) return r;
    const h = r.data.history.find((x) => x.version === version && (x.action === 'create' || x.action === 'save'));
    return h ? ok(h.content) : fail(`${doc.ref.name} has no version ${version}`);
  },
  remove: (it) => orgPolicy.remove(orgPolicy.orgOf(it.project), it.name),
};
