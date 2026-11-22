// A policy rule as the console shows it: one sentence built from parts (who,
// doing, where, effect, conditions, exceptions, why, until), each part a value
// a person picks. The same rule is a YAML entry in one of the org's policy
// files (what is saved) and Cedar (what A3T enforces, compiled by the server
// and shown as it compiled it). Counted conditions (rate, lockout, first use,
// tokens) are checked by A3T against what it counts, in a Block or Ask rule.
import * as YAML from 'yaml';

export type Effect = 'block' | 'ask' | 'only' | 'log';
export type Callers = 'both' | 'agents' | 'people';

export interface Cond { kind: string; p: Record<string, string> }

export interface Rule {
  effect: Effect;
  roles: string[];            // empty: anyone
  callers: Callers;
  agents: string[];           // named agents (ids); empty: none named
  doing: string[];            // permission ids ("flows.delete") or tool globs ("tools.delete_*")
  projects: string[];         // empty: the whole org
  conds: Cond[];              // all must hold (Block/Ask: when; Only: allowed only when)
  unless: Cond[];             // exceptions
  otherwise: 'block' | 'ask'; // Only when…: what happens otherwise
  askFrom: string;            // who answers: '' = the person the agent acts for; a role, an org role, or group:<name>
  timeout: string;            // approval timeout ("4h")
  reason: string;
  until: string;              // yyyy-mm-dd, '' = no end
}

export const blankRule = (): Rule => ({
  effect: 'block', roles: [], callers: 'both', agents: [], doing: [], projects: [], conds: [], unless: [],
  otherwise: 'ask', askFrom: '', timeout: '1d', reason: '', until: '',
});

// ── Condition catalogue ──
export interface CondDef {
  kind: string; group: string; label: string;
  /// Counted by A3T (rate, lockout, first use, tokens): only in a Block or Ask rule's conditions.
  counted?: boolean;
  /// Parameters with their default; `opts` makes it a pick list.
  params: { k: string; label: string; def: string; opts?: string[]; mono?: boolean }[];
  /// The words in the sentence.
  say: (p: Record<string, string>) => string;
}

const DAYS: Record<string, string> = { 'mon-fri': 'weekdays', 'sat-sun': 'weekends', 'mon-sun': 'every day' };
const list = (s: string) => s.split(',').map((x) => x.trim()).filter(Boolean);

export const GROUPS = ['When', 'How often', 'From where', 'Who exactly', 'On what', "What's in the call"];

export const CONDS: CondDef[] = [
  // When
  { kind: 'hours', group: 'When', label: 'Time of day and days',
    params: [{ k: 'days', label: 'Days', def: 'mon-fri', opts: ['mon-fri', 'sat-sun', 'mon-sun'] }, { k: 'from', label: 'From', def: '09:00' }, { k: 'to', label: 'To', def: '18:00' }],
    say: (p) => `${DAYS[p.days] ?? p.days} ${p.from}–${p.to} (org time)` },
  { kind: 'dates', group: 'When', label: 'Between two dates (a freeze)',
    params: [{ k: 'from', label: 'From', def: '2026-12-20' }, { k: 'to', label: 'To', def: '2027-01-02' }],
    say: (p) => `between ${p.from} and ${p.to}` },
  { kind: 'after_deploy', group: 'When', label: 'Soon after a deploy',
    params: [{ k: 'min', label: 'Minutes', def: '30' }],
    say: (p) => `within ${p.min} minutes of a deploy` },
  // How often
  { kind: 'rate', group: 'How often', counted: true, label: 'At most N calls in a period',
    params: [{ k: 'n', label: 'At most', def: '5' }, { k: 'per', label: 'Per', def: 'day', opts: ['minute', 'hour', 'day'] }, { k: 'by', label: 'Counted per', def: 'person', opts: ['person', 'agent', 'project', 'org'] }],
    say: (p) => `at most ${p.n} a ${p.per} per ${p.by}` },
  { kind: 'refusals', group: 'How often', counted: true, label: 'After repeated refusals (lockout)',
    params: [{ k: 'n', label: 'Refused', def: '3' }, { k: 'mins', label: 'Within (min)', def: '10' }, { k: 'block', label: 'Then block (min)', def: '30' }],
    say: (p) => `after ${p.n} refusals within ${p.mins} minutes (for ${p.block} minutes)` },
  { kind: 'first_use', group: 'How often', counted: true, label: 'First time a tool is used',
    params: [], say: () => 'the first time they use that tool' },
  { kind: 'tokens', group: 'How often', counted: true, label: 'Model tokens spent today',
    params: [{ k: 'n', label: 'Over', def: '1000000' }],
    say: (p) => `once over ${Number(p.n).toLocaleString()} tokens today` },
  // From where
  { kind: 'ip', group: 'From where', label: 'IP address range',
    params: [{ k: 'cidr', label: 'Range', def: '10.20.0.0/16', mono: true }],
    say: (p) => `from ${p.cidr}` },
  { kind: 'network', group: 'From where', label: 'A named network',
    params: [{ k: 'name', label: 'Network (networks/ file)', def: 'office', mono: true }],
    say: (p) => `from the ${p.name} network` },
  { kind: 'via', group: 'From where', label: 'Through (console, VS Code…)',
    params: [{ k: 'via', label: 'Through', def: 'console', opts: ['console', 'vscode', 'mcp', 'api-key', 'trigger'] }],
    say: (p) => `through ${p.via === 'api-key' ? 'an API key' : p.via === 'vscode' ? 'VS Code' : p.via}` },
  // Who exactly
  { kind: 'group', group: 'Who exactly', label: "In a group from the org's provider",
    params: [{ k: 'name', label: 'Group', def: 'oncall', mono: true }],
    say: (p) => `in group ${p.name}` },
  { kind: 'person', group: 'Who exactly', label: 'A named person',
    params: [{ k: 'email', label: 'Email', def: 'jane@acme.test', mono: true }],
    say: (p) => p.email },
  { kind: 'mfa', group: 'Who exactly', label: 'Signed in with a second factor',
    params: [], say: () => 'signed in with a second factor' },
  { kind: 'org_idp', group: 'Who exactly', label: "Signed in through the org's provider",
    params: [], say: () => "signed in through the org's provider" },
  { kind: 'session_age', group: 'Who exactly', label: 'Session younger than N hours',
    params: [{ k: 'h', label: 'Hours', def: '8' }],
    say: (p) => `signed in within the last ${p.h} hours` },
  { kind: 'acting_as', group: 'Who exactly', label: 'A SuperAdmin acting as them',
    params: [], say: () => 'a SuperAdmin is acting as them' },
  // On what
  { kind: 'tools', group: 'On what', label: 'Tools by name or pattern',
    params: [{ k: 'tools', label: 'Tools', def: 'delete_*', mono: true }],
    say: (p) => `on tools ${list(p.tools).join(', ')}` },
  { kind: 'models_not_in', group: 'On what', label: 'A model not on a list',
    params: [{ k: 'models', label: 'Allowed models', def: 'a3t/claude-cli-max', mono: true }],
    say: (p) => `with a model other than ${list(p.models).join(', ')}` },
  { kind: 'models_in', group: 'On what', label: 'One of these models',
    params: [{ k: 'models', label: 'Models', def: 'openai/gpt-4o', mono: true }],
    say: (p) => `with ${list(p.models).join(', ')}` },
  { kind: 'destructive', group: 'On what', label: 'Destructive tools',
    params: [], say: () => 'on destructive tools' },
  // What's in the call
  { kind: 'untrusted', group: "What's in the call", label: 'After reading untrusted input',
    params: [], say: () => 'after reading untrusted input in that turn' },
  { kind: 'pii', group: "What's in the call", label: 'Personal data in the arguments',
    params: [], say: () => 'when the call carries personal data' },
  { kind: 'arg_over', group: "What's in the call", label: 'A number argument over N',
    params: [{ k: 'arg', label: 'Argument', def: 'amount', mono: true }, { k: 'n', label: 'Over', def: '1000' }],
    say: (p) => `when ${p.arg} is over ${p.n}` },
];
export const condDef = (k: string) => CONDS.find((c) => c.kind === k);
export const newCond = (k: string): Cond => ({ kind: k, p: Object.fromEntries((condDef(k)?.params ?? []).map((x) => [x.k, x.def])) });
export const condSay = (c: Cond) => condDef(c.kind)?.say(c.p) ?? c.kind;

// ── Words ──
export const ROLE_WORD = (r: string) => (r.endsWith('s') ? r : `${r}s`);
const join = (xs: string[], last = 'and') => (xs.length <= 1 ? xs.join('') : `${xs.slice(0, -1).join(', ')} ${last} ${xs[xs.length - 1]}`);

export function whoWords(r: Rule): string {
  const roles = r.roles.length ? join(r.roles.map(ROLE_WORD), 'or') : '';
  const named = r.agents.length ? `agent${r.agents.length > 1 ? 's' : ''} ${join(r.agents)}` : '';
  if (named) return roles ? `${roles}, and ${named}` : named;
  if (r.callers === 'agents') return roles ? `agents acting for ${roles}` : 'any agent';
  if (r.callers === 'people') return roles ? `${roles} (people, not agents)` : 'any person';
  return roles ? `${roles}, and agents acting for them` : 'anyone';
}

/// Who answers an approval, in words.
export const approverWords = (a: string) => !a ? 'the person the agent acts for'
  : a.startsWith('group:') ? `someone in group ${a.slice(6)}`
  : a === 'securityadmin' ? 'an org security admin' : a === 'admin' ? 'an org admin' : `an ${a}`;

export function sentence(r: Rule, label: (id: string) => string): string {
  if (!r.doing.length) return '';
  const who = whoWords(r);
  const what = join(r.doing.map((d) => label(d).toLowerCase()), 'or');
  const where = r.projects.length ? ` in ${join(r.projects)}` : ' anywhere in the org';
  const conds = r.conds.map(condSay);
  const unless = r.unless.length ? ` Not for anyone ${join(r.unless.map(condSay), 'or')}.` : '';
  const until = r.until ? ` Until ${r.until}.` : '';
  const Who = who[0].toUpperCase() + who.slice(1);
  const ask = `waits ${r.timeout ? `up to ${r.timeout} ` : ''}for ${approverWords(r.askFrom)} to approve`;
  let s = '';
  if (r.effect === 'block') s = `${who === 'anyone' ? 'Nobody' : Who} may ${who === 'anyone' ? '' : 'not '}${what}${where}${conds.length ? ` ${join(conds)}` : ''}.`;
  else if (r.effect === 'log') s = `When ${who} ${what}${where}${conds.length ? ` ${join(conds)}` : ''}, it is logged as "would be refused"; nothing is refused.`;
  else if (r.effect === 'ask') s = `When ${who} ${what}${where}${conds.length ? ` ${join(conds)}` : ''}, the call ${ask}.`;
  else s = `${Who} may ${what}${where} only ${conds.length ? join(conds) : '(add a condition)'}. Otherwise ${r.otherwise === 'ask' ? `an agent's call ${ask}, and a person's own call is refused` : 'the call is refused'}.`;
  return s + unless + until;
}

/// Who it does NOT cover, so "Operators" isn't read as "Operators and up".
export function notCovered(r: Rule, allRoles: string[]): string {
  const out: string[] = [];
  if (r.roles.length) {
    const others = allRoles.filter((x) => !r.roles.includes(x));
    if (others.length) out.push(join(others.map(ROLE_WORD), 'and'));
  }
  if (r.callers === 'agents' && !r.agents.length) out.push('people calling themselves');
  if (r.callers === 'people') out.push('agents');
  if (r.effect === 'ask') out.push("people calling themselves (an approval is for agents' calls)");
  if (r.projects.length) out.push('other projects');
  return out.length ? `Not covered: ${join(out)}.` : '';
}

/// What stops the rule from being saved as it is (the server says the same).
export function problems(r: Rule): string[] {
  const out: string[] = [];
  const counted = (cs: Cond[]) => cs.filter((c) => condDef(c.kind)?.counted).map((c) => condDef(c.kind)!.label);
  if (r.effect === 'only' && counted(r.conds).length) out.push(`${join(counted(r.conds))} is counted by A3T: use it in a Blocks or Asks rule`);
  if (counted(r.unless).length) out.push(`${join(counted(r.unless))} can't be an exception`);
  if (r.effect === 'log' && counted(r.conds).length) out.push('a rule that only logs can\'t count');
  if (r.effect === 'only' && !r.conds.length) out.push('"Allows only when" needs at least one condition');
  if (r.askFrom.startsWith('group:') && r.askFrom.length < 7) out.push('name the approving group');
  return out;
}

// ── YAML (the policy file entry) ──
const condYaml = (c: Cond) => (Object.keys(c.p).length ? { [c.kind]: { ...c.p } } : c.kind);

export function toYamlRule(r: Rule): any {
  const out: any = {};
  const who: any = {};
  if (r.roles.length) who.roles = [...r.roles];
  if (r.agents.length) who.agents = [...r.agents];
  else if (r.callers === 'agents') who.agents = r.roles.length ? 'acting' : 'any';
  if (r.callers === 'people' && !r.agents.length) who.people_only = true;
  if (Object.keys(who).length) out.who = who;
  const ask = r.effect === 'ask' || (r.effect === 'only' && r.otherwise === 'ask');
  if (r.effect === 'only') {
    out.only = [...r.doing];
    out.allowed_only_when = r.conds.map(condYaml);
    out.otherwise = r.otherwise;
  } else {
    out[r.effect === 'ask' ? 'needs_approval' : 'cannot'] = [...r.doing];
    if (r.conds.length) out.when = r.conds.map(condYaml);
  }
  if (ask && r.timeout) out.timeout = r.timeout;
  if (ask && r.askFrom) out.approver = r.askFrom;
  if (r.effect === 'log') out.log_only = true;
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
      if (Array.isArray(e?.who?.agents)) { r.agents = e.who.agents.map(String); r.callers = 'agents'; }
      if (e?.who?.agents === 'any' || e?.who?.agents === 'acting') r.callers = 'agents';
      if (e?.who?.people_only) r.callers = 'people';
      const only = Array.isArray(e?.only);
      const ask = Array.isArray(e?.needs_approval);
      r.effect = only ? 'only' : ask ? 'ask' : e?.log_only ? 'log' : 'block';
      r.doing = (only ? e.only : ask ? e.needs_approval : e?.cannot ?? []).map(String);
      if (e?.timeout) r.timeout = String(e.timeout);
      if (e?.approver) r.askFrom = String(e.approver);
      r.reason = String(e?.reason ?? '');
      if (e?.until) r.until = String(e.until);
      const asCond = (x: any): Cond => typeof x === 'string' ? { kind: x, p: {} } : { kind: Object.keys(x)[0], p: Object.fromEntries(Object.entries(Object.values(x)[0] as any ?? {}).map(([k, v]) => [k, Array.isArray(v) ? v.join(', ') : String(v)])) };
      if (e?.on?.tools) r.conds.push({ kind: 'tools', p: { tools: e.on.tools.join(', ') } });
      if (e?.on?.models?.in) r.conds.push({ kind: 'models_in', p: { models: e.on.models.in.join(', ') } });
      if (e?.on?.models?.not_in) r.conds.push({ kind: 'models_not_in', p: { models: e.on.models.not_in.join(', ') } });
      for (const c of (only ? e?.allowed_only_when : e?.when) ?? []) r.conds.push(asCond(c));
      for (const c of e?.unless ?? []) r.unless.push(asCond(c));
      r.otherwise = e?.otherwise === 'ask' ? 'ask' : 'block';
      if (!r.doing.length) return null;
      return r;
    } catch { return null; }
  });
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
    rule: { effect: 'ask', doing: ['tools.pay_*'], conds: [C('arg_over')], askFrom: 'group:finance', timeout: '1d', reason: 'Large payments need finance.' } },
  { id: 'expires', title: 'Access that expires', what: 'A rule that switches itself off on a date.',
    rule: { effect: 'block', roles: ['Viewer'], doing: ['flows.run'], until: '2026-12-31', reason: 'Contractors run flows only until the end of the year.' } },
  { id: 'tokens', title: 'Spending cap', what: 'Model calls stop after N tokens a day.',
    rule: { effect: 'block', callers: 'agents', doing: ['models.use'], conds: [C('tokens')], reason: 'Daily token budget reached.' } },
];
export const fromPattern = (p: Pattern): Rule => ({ ...blankRule(), ...structuredClone(p.rule) } as Rule);

export const slug = (s: string) => s.toLowerCase().replace(/[^a-z0-9]+/g, '-').replace(/^-|-$/g, '').slice(0, 50) || 'new-rule';
