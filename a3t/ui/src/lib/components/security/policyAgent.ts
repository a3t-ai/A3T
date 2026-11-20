// GOV-1: a rule described in words, written as a policy file by
// agent_a3t_policy (an A3T agent), at design time. FlowR checks it against the
// org's other files (and the agent is asked again with what was wrong); the
// security admin replays it against recent decisions, reads it, and saves it
// themselves — with a second approver where the org asks for one. Nothing the
// agent writes is enforced until then.
import { askAgentJson } from '$lib/components/triggers/mapperApi';
import { orgPolicy, securityCatalogue } from '$lib/services/api';

export const POLICY_AGENT = 'agent_a3t_policy';

export interface PolicyDraft {
  path: string; yaml: string; explanation: string; assumptions: string[];
  /// FlowR's check: ok, or what is wrong.
  ok: boolean; errors: string[]; tries: number;
}

const PATH = /^policies\/[a-z0-9][a-z0-9-]{0,62}\.yaml$/;

export async function draftPolicy(
  org: string,
  description: string,
  ctx: { projects: string[]; files: string[]; roles: string[] },
  signal?: AbortSignal,
): Promise<{ ok: true; draft: PolicyDraft } | { ok: false; error: string }> {
  const cat = await securityCatalogue();
  const catalogue = cat.ok ? (cat.data?.permissions ?? []).map((p: any) => ({ id: p.id, label: p.label, ...(p.enforced_by ? { console_only: true } : {}) })) : [];
  let errors: string[] = [];
  let last: PolicyDraft | null = null;
  for (let tries = 1; tries <= 3; tries++) {
    const ask = { org, description, catalogue, roles: ctx.roles, projects: ctx.projects, files: ctx.files,
      ...(last ? { previous: { path: last.path, yaml: last.yaml }, check_errors: errors } : {}) };
    const r = await askAgentJson(POLICY_AGENT, ask, signal);
    if (!r.ok) return r;
    const j = r.json;
    if (typeof j.yaml !== 'string' || !j.yaml.trim()) return { ok: false, error: 'It answered without a policy file.' };
    const path = typeof j.path === 'string' && PATH.test(j.path) ? j.path : `policies/${String(j.path ?? 'new-rule').replace(/^.*\//, '').replace(/\.ya?ml$/, '').toLowerCase().replace(/[^a-z0-9-]/g, '-').slice(0, 60) || 'new-rule'}.yaml`;
    const v = await orgPolicy.validate(org, [{ path, content: j.yaml }]);
    errors = v.ok ? (v.data?.ok ? [] : (v.data?.errors ?? ['it does not check'])) : [v.error ?? 'the check failed'];
    last = { path, yaml: j.yaml, explanation: String(j.explanation ?? ''), assumptions: (j.assumptions ?? []).map(String), ok: errors.length === 0, errors, tries };
    if (last.ok) break;
  }
  return { ok: true, draft: last! };
}
