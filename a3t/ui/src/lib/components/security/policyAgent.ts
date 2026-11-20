// GOV-1: a rule described in words, written as policy files by
// agent_a3t_policy (an A3T agent), at design time. FlowR checks them together
// against the org's other files (and the agent is asked again with what was
// wrong); the security admin replays them against recent decisions, reads them,
// and saves them themselves — with a second approver where the org asks for
// one. Nothing the agent writes is enforced until then.
import { askAgentJson } from '$lib/components/triggers/mapperApi';
import { orgPolicy, securityCatalogue } from '$lib/services/api';

export const POLICY_AGENT = 'agent_a3t_policy';

export interface DraftFile { path: string; yaml: string; explanation: string }
export interface PolicyDraft {
  files: DraftFile[]; assumptions: string[];
  /// FlowR's check of all of them together: ok, or what is wrong.
  ok: boolean; errors: string[]; tries: number;
}

const PATH = /^policies\/[a-z0-9][a-z0-9-]{0,62}\.yaml$/;
const pathOf = (p: unknown, i: number) => typeof p === 'string' && PATH.test(p) ? p
  : `policies/${String(p ?? `new-rule-${i + 1}`).replace(/^.*\//, '').replace(/\.ya?ml$/, '').toLowerCase().replace(/[^a-z0-9-]/g, '-').slice(0, 60) || `new-rule-${i + 1}`}.yaml`;

/// The files in an answer: `{files: [...]}`, or one file, or a bare list.
function filesOf(j: any): DraftFile[] {
  const list: any[] = Array.isArray(j) ? j : Array.isArray(j?.files) ? j.files : j?.yaml ? [j] : [];
  return list.filter((f) => typeof f?.yaml === 'string' && f.yaml.trim())
    .map((f, i) => ({ path: pathOf(f.path, i), yaml: f.yaml, explanation: String(f.explanation ?? '') }));
}

export async function checkDraft(org: string, files: DraftFile[]): Promise<string[]> {
  const v = await orgPolicy.validate(org, files.map((f) => ({ path: f.path, content: f.yaml })));
  return v.ok ? (v.data?.ok ? [] : (v.data?.errors ?? ['it does not check'])) : [v.error ?? 'the check failed'];
}

export async function draftPolicy(
  org: string,
  description: string,
  ctx: { projects: string[]; files: string[]; roles: string[] },
  signal?: AbortSignal,
): Promise<{ ok: true; draft: PolicyDraft } | { ok: false; error: string }> {
  const cat = await securityCatalogue();
  const catalogue = cat.ok ? (cat.data?.permissions ?? []).map((p: any) => ({ id: p.id, label: p.label, ...(p.enforced_by ? { console_only: true } : {}) })) : [];
  let last: PolicyDraft | null = null;
  for (let tries = 1; tries <= 3; tries++) {
    const ask = { org, description, catalogue, roles: ctx.roles, projects: ctx.projects, files: ctx.files,
      ...(last ? { previous: last.files.map((f) => ({ path: f.path, yaml: f.yaml })), check_errors: last.errors } : {}) };
    const r = await askAgentJson(POLICY_AGENT, ask, signal);
    if (!r.ok) return r;
    const files = filesOf(r.json);
    if (!files.length) return { ok: false, error: 'It answered without a policy file.' };
    const errors = await checkDraft(org, files);
    last = { files, assumptions: (r.json?.assumptions ?? []).map(String), ok: errors.length === 0, errors, tries };
    if (last.ok) break;
  }
  return { ok: true, draft: last! };
}
