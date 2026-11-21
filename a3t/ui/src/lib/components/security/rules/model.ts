// A policy rule as the console shows it: one sentence built from parts (who,
// doing, where, effect, conditions, exceptions, why, until), each part a value
// a person picks. The same rule is a YAML entry in one of the org's policy
// files (what is saved) and Cedar (what A3T enforces). Parts marked `today:
// false` are what A3T can't check yet: they are drawn and previewed so the
// design can be tried, and a rule using them can't be saved until A3T passes
// what they need to Cedar (time, address, sign-in, counts, call arguments).
import * as YAML from 'yaml';

export type Effect = 'block' | 'ask' | 'only' | 'log';
export type Callers = 'both' | 'agents' | 'people';

export interface Cond { kind: string; p: Record<string, string> }

export interface Rule {
  effect: Effect;
  roles: string[];            // empty: anyone
  callers: Callers;
  doing: string[];            // permission ids ("flows.delete") or tool globs ("tools.delete_*")
  projects: string[];         // empty: the whole org
  conds: Cond[];              // all must hold (Block/Ask: when; Only: allowed only when)
  unless: Cond[];             // exceptions
  otherwise: 'block' | 'ask'; // Only when…: what happens otherwise
  askFrom: string;            // who answers an approval: '' = the person the agent acts for
  timeout: string;            // approval timeout ("4h")
  reason: string;
  until: string;              // yyyy-mm-dd, '' = no end
}

export const blankRule = (): Rule => ({
  effect: 'block', roles: [], callers: 'both', doing: [], projects: [], conds: [], unless: [],
  otherwise: 'ask', askFrom: '', timeout: '1d', reason: '', until: '',
});

// ── Condition catalogue ──
export interface CondDef {
  kind: string; group: string; label: string; today: boolean;
  /// Parameters with their default; `opts` makes it a pick list.
  params: { k: string; label: string; def: string; opts?: string[]; mono?: boolean }[];
  /// The words in the sentence.
  say: (p: Record<string, string>) => string;
  /// Cedar (preview): a boolean expression.
  cedar: (p: Record<string, string>) => string;
  /// What A3T has to pass to Cedar for it (not today).
  needs?: string;
}

const DAYS: Record<string, string> = { 'mon-fri': 'weekdays', 'sat-sun': 'weekends', 'mon-sun': 'every day' };
const DAYNUM: Record<string, string> = { 'mon-fri': '[1,2,3,4,5]', 'sat-sun': '[6,7]', 'mon-sun': '[1,2,3,4,5,6,7]' };
const mins = (hhmm: string) => { const [h, m] = hhmm.split(':').map(Number); return (h || 0) * 60 + (m || 0); };
const list = (s: string) => s.split(',').map((x) => x.trim()).filter(Boolean);
const q = (s: string) => JSON.stringify(s);

export const GROUPS = ['When', 'How often', 'From where', 'Who exactly', 'On what', "What's in the call"];

export const CONDS: CondDef[] = [
  // When
  { kind: 'hours', group: 'When', label: 'Time of day and days', today: false, needs: 'context.time (org timezone)',
    params: [{ k: 'days', label: 'Days', def: 'mon-fri', opts: ['mon-fri', 'sat-sun', 'mon-sun'] }, { k: 'from', label: 'From', def: '09:00' }, { k: 'to', label: 'To', def: '18:00' }, { k: 'tz', label: 'Timezone', def: 'Europe/Berlin' }],
    say: (p) => `${DAYS[p.days] ?? p.days} ${p.from}–${p.to} ${p.tz.split('/').pop()?.replace('_', ' ')} time`,
    cedar: (p) => `${DAYNUM[p.days] ?? '[]'}.contains(context.time.weekday) && context.time.minute >= ${mins(p.from)} && context.time.minute < ${mins(p.to)}` },
  { kind: 'dates', group: 'When', label: 'Between two dates (a freeze)', today: false, needs: 'context.now',
    params: [{ k: 'from', label: 'From', def: '2026-12-20' }, { k: 'to', label: 'To', def: '2027-01-02' }],
    say: (p) => `between ${p.from} and ${p.to}`,
    cedar: (p) => `context.now >= datetime(${q(p.from)}) && context.now < datetime(${q(p.to)})` },
  { kind: 'after_deploy', group: 'When', label: 'Soon after a deploy', today: false, needs: 'context.since_deploy_s',
    params: [{ k: 'min', label: 'Minutes', def: '30' }],
    say: (p) => `within ${p.min} minutes of a deploy`, cedar: (p) => `context.since_deploy_s < ${Number(p.min) * 60}` },
  // How often
  { kind: 'rate', group: 'How often', label: 'At most N calls in a period', today: false, needs: 'context.counts (kept by A3T)',
    params: [{ k: 'n', label: 'At most', def: '5' }, { k: 'per', label: 'Per', def: 'day', opts: ['minute', 'hour', 'day'] }, { k: 'by', label: 'Counted per', def: 'person', opts: ['person', 'agent', 'key', 'project', 'org'] }],
    say: (p) => `at most ${p.n} a ${p.per} per ${p.by}`, cedar: (p) => `context.counts.rule_per_${p.per} < ${p.n}` },
  { kind: 'refusals', group: 'How often', label: 'After repeated refusals (lockout)', today: false, needs: 'context.counts.refused',
    params: [{ k: 'n', label: 'Refused', def: '3' }, { k: 'mins', label: 'Within (min)', def: '10' }, { k: 'block', label: 'Then block (min)', def: '30' }],
    say: (p) => `after ${p.n} refusals within ${p.mins} minutes (for ${p.block} minutes)`,
    cedar: (p) => `context.counts.refused_${p.mins}m >= ${p.n} && context.counts.last_refusal_s < ${Number(p.block) * 60}` },
  { kind: 'first_use', group: 'How often', label: 'First time a tool is used', today: false, needs: 'context.counts.ever',
    params: [], say: () => 'the first time they use that tool', cedar: () => 'context.counts.ever == 0' },
  { kind: 'tokens', group: 'How often', label: 'Model tokens spent today', today: false, needs: 'context.usage',
    params: [{ k: 'n', label: 'Over', def: '1000000' }],
    say: (p) => `once over ${Number(p.n).toLocaleString()} tokens today`, cedar: (p) => `context.usage.tokens_today > ${p.n}` },
  // From where
  { kind: 'ip', group: 'From where', label: 'IP address range', today: false, needs: 'context.source_ip',
    params: [{ k: 'cidr', label: 'Range', def: '10.20.0.0/16', mono: true }],
    say: (p) => `from ${p.cidr}`, cedar: (p) => `context.source_ip.isInRange(ip(${q(p.cidr)}))` },
  { kind: 'network', group: 'From where', label: 'A named network', today: false, needs: 'context.source_ip + networks/ files',
    params: [{ k: 'name', label: 'Network', def: 'office', opts: ['office', 'vpn'] }],
    say: (p) => `from the ${p.name} network`, cedar: (p) => `context.networks.contains(${q(p.name)})` },
  { kind: 'via', group: 'From where', label: 'Through (console, VS Code…)', today: false, needs: 'context.channel',
    params: [{ k: 'via', label: 'Through', def: 'console', opts: ['console', 'vscode', 'mcp', 'api-key', 'chat', 'trigger'] }],
    say: (p) => `through ${p.via === 'api-key' ? 'an API key' : p.via === 'vscode' ? 'VS Code' : p.via}`, cedar: (p) => `context.channel == ${q(p.via)}` },
  // Who exactly
  { kind: 'group', group: 'Who exactly', label: "In a group from the org's provider", today: false, needs: 'principal.groups',
    params: [{ k: 'name', label: 'Group', def: 'oncall', mono: true }],
    say: (p) => `in group ${p.name}`, cedar: (p) => `principal has groups && principal.groups.contains(${q(p.name)})` },
  { kind: 'person', group: 'Who exactly', label: 'A named person', today: false, needs: 'principal.email',
    params: [{ k: 'email', label: 'Email', def: 'jane@acme.test', mono: true }],
    say: (p) => p.email, cedar: (p) => `principal has email && principal.email == ${q(p.email)}` },
  { kind: 'mfa', group: 'Who exactly', label: 'Signed in with a second factor', today: false, needs: 'context.auth',
    params: [], say: () => 'signed in with a second factor', cedar: () => 'context.auth.second_factor' },
  { kind: 'org_idp', group: 'Who exactly', label: "Signed in through the org's provider", today: false, needs: 'context.auth',
    params: [], say: () => "signed in through the org's provider", cedar: () => 'context.auth.org_idp' },
  { kind: 'session_age', group: 'Who exactly', label: 'Session younger than N hours', today: false, needs: 'context.auth',
    params: [{ k: 'h', label: 'Hours', def: '8' }],
    say: (p) => `signed in within the last ${p.h} hours`, cedar: (p) => `context.auth.age_s < ${Number(p.h) * 3600}` },
  { kind: 'acting_as', group: 'Who exactly', label: 'A SuperAdmin acting as them', today: false, needs: 'context.auth',
    params: [], say: () => 'a SuperAdmin is acting as them', cedar: () => 'context.auth has impersonated_by' },
  // On what
  { kind: 'tools', group: 'On what', label: 'Tools by name or pattern', today: true,
    params: [{ k: 'tools', label: 'Tools', def: 'delete_*', mono: true }],
    say: (p) => `on tools ${list(p.tools).join(', ')}`,
    cedar: (p) => `resource is Tool && resource has name && (${list(p.tools).map((t) => `resource.name like ${q(t)}`).join(' || ')})` },
  { kind: 'models_not_in', group: 'On what', label: 'A model not on a list', today: true,
    params: [{ k: 'models', label: 'Allowed models', def: 'a3t/claude-cli-max', mono: true }],
    say: (p) => `with a model other than ${list(p.models).join(', ')}`,
    cedar: (p) => `resource is Llm && !(resource has name && [${list(p.models).map(q).join(', ')}].contains(resource.name))` },
  { kind: 'models_in', group: 'On what', label: 'One of these models', today: true,
    params: [{ k: 'models', label: 'Models', def: 'openai/gpt-4o', mono: true }],
    say: (p) => `with ${list(p.models).join(', ')}`,
    cedar: (p) => `resource is Llm && resource has name && [${list(p.models).map(q).join(', ')}].contains(resource.name)` },
  { kind: 'destructive', group: 'On what', label: 'Destructive tools', today: false, needs: 'a rule field (Cedar has it)',
    params: [], say: () => 'on destructive tools', cedar: () => 'resource is Tool && resource has destructive && resource.destructive' },
  // What's in the call
  { kind: 'untrusted', group: "What's in the call", label: 'After reading untrusted input', today: false, needs: 'context.untrusted_seen',
    params: [], say: () => 'after reading untrusted input in that turn', cedar: () => 'context.untrusted_seen' },
  { kind: 'pii', group: "What's in the call", label: 'Personal data in the arguments', today: false, needs: 'context.args_pii',
    params: [], say: () => 'when the call carries personal data', cedar: () => 'context.args_pii' },
  { kind: 'arg_over', group: "What's in the call", label: 'A number argument over N', today: false, needs: 'context.args',
    params: [{ k: 'arg', label: 'Argument', def: 'amount', mono: true }, { k: 'n', label: 'Over', def: '1000' }],
    say: (p) => `when ${p.arg} is over ${p.n}`, cedar: (p) => `context.args has ${p.arg} && context.args.${p.arg} > ${p.n}` },
];
export const condDef = (k: string) => CONDS.find((c) => c.kind === k);
export const newCond = (k: string): Cond => ({ kind: k, p: Object.fromEntries((condDef(k)?.params ?? []).map((x) => [x.k, x.def])) });
export const condSay = (c: Cond) => condDef(c.kind)?.say(c.p) ?? c.kind;

// ── Words ──
export const ROLE_WORD = (r: string) => (r.endsWith('s') ? r : `${r}s`);
const join = (xs: string[], last = 'and') => (xs.length <= 1 ? xs.join('') : `${xs.slice(0, -1).join(', ')} ${last} ${xs[xs.length - 1]}`);

export function whoWords(r: Rule): string {
  const roles = r.roles.length ? join(r.roles.map(ROLE_WORD), 'or') : '';
  if (r.callers === 'agents') return roles ? `agents acting for ${roles}` : 'any agent';
  if (r.callers === 'people') return roles ? `${roles} (people, not agents)` : 'any person';
  return roles ? `${roles}, and agents acting for them` : 'anyone';
}

export function sentence(r: Rule, label: (id: string) => string): string {
  if (!r.doing.length) return '';
  const who = whoWords(r);
  const what = join(r.doing.map((d) => label(d).toLowerCase()), 'or');
  const where = r.projects.length ? ` in ${join(r.projects)}` : ' anywhere in the org';
  const conds = r.conds.map(condSay);
  const unless = r.unless.length ? ` Not for anyone ${join(r.unless.map(condSay), 'or')}.` : '';
  const until = r.until ? ` Until ${r.until}.` : '';
  const Who = who[0].toUpperCase() + who.slice(1);
  const ask = `waits ${r.timeout ? `up to ${r.timeout} ` : ''}for ${r.askFrom ? `an ${r.askFrom}` : 'the person the agent acts for'} to approve`;
  let s = '';
  if (r.effect === 'block') s = `${who === 'anyone' ? 'Nobody' : Who} may ${who === 'anyone' ? '' : 'not '}${what}${where}${conds.length ? ` ${join(conds)}` : ''}.`;
  else if (r.effect === 'log') s = `When ${who} ${what}${where}${conds.length ? ` ${join(conds)}` : ''}, it is logged as "would be refused"; nothing is refused.`;
  else if (r.effect === 'ask') s = `When ${who} ${what}${where}${conds.length ? ` ${join(conds)}` : ''}, the call ${ask}.`;
  else s = `${Who} may ${what}${where} only ${conds.length ? join(conds) : '(add a condition)'}. Otherwise the call ${r.otherwise === 'ask' ? ask : 'is refused'}.`;
  return s + unless + until;
}

/// Who it does NOT cover, so "Operators" isn't read as "Operators and up".
export function notCovered(r: Rule, allRoles: string[]): string {
  const out: string[] = [];
  if (r.roles.length) {
    const others = allRoles.filter((x) => !r.roles.includes(x));
    if (others.length) out.push(join(others.map(ROLE_WORD), 'and'));
  }
  if (r.callers === 'agents') out.push('people calling themselves');
  if (r.callers === 'people') out.push('agents');
  if (r.projects.length) out.push('other projects');
  return out.length ? `Not covered: ${join(out)}.` : '';
}

/// What A3T can't check yet, for this rule.
export function notYet(r: Rule): string[] {
  const out: string[] = [];
  for (const c of [...r.conds, ...r.unless]) { const d = condDef(c.kind); if (d && !d.today) out.push(`${d.label} (needs ${d.needs})`); }
  if (r.effect === 'only') out.push('"Allow only when" (the compiler writes forbid … unless)');
  if (r.unless.length) out.push('exceptions (unless)');
  if (r.until) out.push('an end date (needs context.now)');
  if (r.callers === 'people') out.push('"people only"');
  if (r.callers === 'agents' && r.roles.length) out.push('"agents acting for a role" (today: any agent, or a role)');
  if (r.askFrom) out.push('choosing who approves (today: the person the agent acts for)');
  return [...new Set(out)];
}

// ── YAML (the policy file entry) ──
const condYaml = (c: Cond) => (Object.keys(c.p).length ? { [c.kind]: { ...c.p } } : c.kind);

export function toYamlRule(r: Rule): any {
  const out: any = {};
  const who: any = {};
  if (r.roles.length) who.roles = [...r.roles];
  if (r.callers === 'agents') who.agents = r.roles.length ? 'acting' : 'any';
  if (r.callers === 'people') who.people_only = true;
  if (Object.keys(who).length) out.who = who;
  const today = r.conds.filter((c) => condDef(c.kind)?.today);
  const later = r.conds.filter((c) => !condDef(c.kind)?.today);
  if (r.effect === 'only') {
    out.allowed_only_when = r.conds.map(condYaml);
    out.otherwise = r.otherwise === 'ask' ? { needs_approval: true, ...(r.askFrom ? { from: r.askFrom } : {}), timeout: r.timeout } : 'block';
    out.on = [...r.doing];
  } else {
    out[r.effect === 'ask' ? 'needs_approval' : 'cannot'] = [...r.doing];
    if (r.effect === 'ask' && r.timeout) out.timeout = r.timeout;
    if (r.effect === 'ask' && r.askFrom) out.approver = r.askFrom;
    const on: any = {};
    for (const c of today) {
      if (c.kind === 'tools') on.tools = list(c.p.tools);
      if (c.kind === 'models_in') on.models = { in: list(c.p.models) };
      if (c.kind === 'models_not_in') on.models = { not_in: list(c.p.models) };
    }
    if (Object.keys(on).length) out.on = on;
    if (later.length) out.when = later.map(condYaml);
  }
  if (r.unless.length) out.unless = r.unless.map(condYaml);
  if (r.until) out.until = r.until;
  out.reason = r.reason;
  return out;
}

export function fileYaml(name: string, description: string, r: Rule, rest: any[] = []): string {
  const doc: any = { kind: 'a3t/policy', name, description: description || r.reason };
  doc.applies_to = r.projects.length ? { projects: [...r.projects] } : { org: true };
  doc.rules = [...rest, toYamlRule(r)];
  return YAML.stringify(doc, { lineWidth: 100 });
}

/// A file's rules as the console's model; a rule it can't read is `null` (shown as written).
export function rulesOf(y: any): (Rule | null)[] {
  const projects: string[] = Array.isArray(y?.applies_to?.projects) ? y.applies_to.projects.map(String) : [];
  return (Array.isArray(y?.rules) ? y.rules : []).map((e: any) => {
    try {
      const r = blankRule();
      r.projects = projects;
      r.roles = Array.isArray(e?.who?.roles) ? e.who.roles.map(String) : [];
      if (e?.who?.agents === 'any' || e?.who?.agents === 'acting') r.callers = 'agents';
      if (e?.who?.people_only) r.callers = 'people';
      const ask = Array.isArray(e?.needs_approval);
      r.effect = e?.allowed_only_when ? 'only' : ask ? 'ask' : 'block';
      r.doing = (e?.allowed_only_when ? e.on : ask ? e.needs_approval : e?.cannot ?? []).map(String);
      if (e?.timeout) r.timeout = String(e.timeout);
      if (e?.approver) r.askFrom = String(e.approver);
      r.reason = String(e?.reason ?? '');
      if (e?.until) r.until = String(e.until);
      const asCond = (x: any): Cond => typeof x === 'string' ? { kind: x, p: {} } : { kind: Object.keys(x)[0], p: Object.fromEntries(Object.entries(Object.values(x)[0] as any ?? {}).map(([k, v]) => [k, String(v)])) };
      if (e?.on?.tools) r.conds.push({ kind: 'tools', p: { tools: e.on.tools.join(', ') } });
      if (e?.on?.models?.in) r.conds.push({ kind: 'models_in', p: { models: e.on.models.in.join(', ') } });
      if (e?.on?.models?.not_in) r.conds.push({ kind: 'models_not_in', p: { models: e.on.models.not_in.join(', ') } });
      for (const c of e?.when ?? e?.allowed_only_when ?? []) r.conds.push(asCond(c));
      for (const c of e?.unless ?? []) r.unless.push(asCond(c));
      if (e?.otherwise === 'block') r.otherwise = 'block';
      if (!r.doing.length) return null;
      return r;
    } catch { return null; }
  });
}

// ── Cedar (preview; A3T compiles the real thing at save) ──
const TOOL_ACTIONS = '[Action::"tool:invoke", Action::"mcp:call"]';
export function cedarPreview(id: string, r: Rule): string {
  const head = r.doing.some((d) => d.startsWith('models.')) ? 'action == Action::"llm:call"' : `action in ${TOOL_ACTIONS}`;
  const when: string[] = [];
  if (r.roles.length) when.push(`principal has level && ${JSON.stringify(r.roles)}.contains(principal.level)   // exact level`);
  if (r.callers === 'agents') when.push('principal is Agent');
  if (r.callers === 'people') when.push('principal is User');
  for (const d of r.doing) {
    if (d.startsWith('tools.')) when.push(`resource is Tool && resource has name && resource.name like ${q(d.slice(6))}`);
    else if (!d.startsWith('models.')) when.push(`/* ${d}: its tools and actions, from the catalogue */`);
  }
  if (r.projects.length) when.push(`/* placed on ${r.projects.join(', ')} */`);
  if (r.until) when.push(`context.now < datetime(${q(r.until)})`);
  const conds = r.conds.map((c) => condDef(c.kind)?.cedar(c.p) ?? c.kind);
  const exc = r.unless.map((c) => condDef(c.kind)?.cedar(c.p) ?? c.kind);
  let unless: string[] = [];
  if (r.effect === 'only') unless = [conds.length ? `(${conds.join(' &&\n   ')})` : 'false'];
  else when.push(...conds);
  unless.push(...exc);
  const approval = r.effect === 'ask' || (r.effect === 'only' && r.otherwise === 'ask');
  const ann = [`@id(${q(id)})`, `@reason(${q(r.reason || '…')})`];
  if (approval) ann.push(`@approval(${q(`${r.askFrom || 'on_behalf_of'}:${r.timeout}`)})`);
  if (r.effect === 'log') ann.push('@shadow("true")   // logged, never refused');
  return `${ann.join('\n')}\nforbid(principal, ${head}, resource)\nwhen {\n  ${when.join(' &&\n  ') || 'true'}\n}${unless.length ? `\nunless {\n  ${unless.join(' ||\n  ')}\n}` : ''};`;
}

// ── Patterns ──
export interface Pattern { id: string; title: string; what: string; rule: Partial<Rule> }
const C = newCond;
export const PATTERNS: Pattern[] = [
  { id: 'office', title: 'Office hours only', what: 'Deletes only on weekdays 09–18; otherwise someone approves.',
    rule: { effect: 'only', roles: ['Operator'], doing: ['flows.delete'], conds: [C('hours')], otherwise: 'ask', askFrom: 'Admin', timeout: '4h', reason: 'Deletes happen in office hours, or with an Admin\'s OK.' } },
  { id: 'freeze', title: 'Change freeze', what: 'No deploys between two dates, except on-call.',
    rule: { effect: 'block', doing: ['production.deploy'], conds: [C('dates')], unless: [C('group')], reason: 'Year-end change freeze.' } },
  { id: 'lockout', title: 'Lock out after refusals', what: 'Refused 3× in 10 min: blocked for 30 min.',
    rule: { effect: 'block', doing: ['tools.*'], conds: [C('refusals')], reason: 'Too many refused calls in a short time.' } },
  { id: 'rate', title: 'Rate limit', what: 'At most N calls an hour per person.',
    rule: { effect: 'block', doing: ['runs.run'], conds: [{ kind: 'rate', p: { n: '100', per: 'hour', by: 'person' } }], reason: 'At most 100 runs an hour per person.' } },
  { id: 'models', title: 'Approved models only', what: 'Only listed models; others are refused.',
    rule: { effect: 'block', doing: ['models.use'], conds: [C('models_not_in')], reason: 'Only approved models.' } },
  { id: 'agent-delete', title: 'Agents ask before deleting', what: 'An agent waits for a person before any delete.',
    rule: { effect: 'ask', callers: 'agents', doing: ['tools.delete_*'], timeout: '4h', reason: 'A person confirms what an agent deletes.' } },
  { id: 'network', title: 'Company network only', what: 'Changes only from the office or VPN.',
    rule: { effect: 'only', doing: ['flows.change'], conds: [C('network')], otherwise: 'block', reason: 'Changes come from the company network.' } },
  { id: 'mfa', title: 'Second factor for risky calls', what: 'Destructive tools need a second factor.',
    rule: { effect: 'only', doing: ['tools.delete_*'], conds: [C('mfa')], otherwise: 'block', reason: 'Deleting needs a second factor.' } },
  { id: 'untrusted', title: 'Untrusted input guard', what: 'After web or email input, sending waits for a person.',
    rule: { effect: 'ask', callers: 'agents', doing: ['tools.send_*'], conds: [C('untrusted')], timeout: '1h', reason: 'Sending after reading untrusted input needs a person.' } },
  { id: 'amount', title: 'Big amounts', what: 'Payments over 1,000 need a finance approver.',
    rule: { effect: 'ask', doing: ['tools.pay_*'], conds: [C('arg_over')], askFrom: 'finance', timeout: '1d', reason: 'Large payments need finance.' } },
  { id: 'expires', title: 'Access that expires', what: 'A rule that switches itself off on a date.',
    rule: { effect: 'block', roles: ['Viewer'], doing: ['flows.run'], until: '2026-12-31', reason: 'Contractors run flows only until the end of the year.' } },
  { id: 'tokens', title: 'Spending cap', what: 'Model calls stop after N tokens a day.',
    rule: { effect: 'block', callers: 'agents', doing: ['models.use'], conds: [C('tokens')], reason: 'Daily token budget reached.' } },
];
export const fromPattern = (p: Pattern): Rule => ({ ...blankRule(), ...structuredClone(p.rule) } as Rule);

export const slug = (s: string) => s.toLowerCase().replace(/[^a-z0-9]+/g, '-').replace(/^-|-$/g, '').slice(0, 50) || 'new-rule';
