// AQL (Agent Query Language) as a tree of verbs, for <SchemaTree of="aql" />.
// Follows grammar/aql.pest and spec/AQL_SPEC_v0.13.md in a3t-ai/AQL (release 0.13.0).
// Every example is a complete statement, or a script of statements separated by ";". Check them
// against the real parser before publishing (aql_parser::parse, or parse_script for scripts).
import type { SchemaNode } from './schema-tree';

// Building blocks shared by several verbs.

const memories = (types: [string, string][]): SchemaNode[] =>
	types.map(([k, d]) => ({ k, t: 'memory', d, ex: '' }));

const MEMORY: Record<string, string> = {
	EPISODIC: 'What happened: events, incidents, decisions and their outcomes.',
	SEMANTIC: 'What is known: records about things, and the links between them.',
	PROCEDURAL: 'How to do things: runbooks and strategies the agent runs.',
	TOOLS: 'What can be used: the tools the agent may call, with their rankings.',
	WORKING: 'The current session: what the agent is holding right now.',
	ALL: 'Every memory type at once. For RECALL and FORGET only.',
};
const pick = (...names: string[]): [string, string][] => names.map((n) => [n, MEMORY[n]]);

const where = (verb: string): SchemaNode => ({
	k: 'WHERE', t: 'predicate',
	d: 'Conditions on fields: field, operator, value. Join them with AND and OR, group them with brackets. Fields can be dotted, like metadata.pod.',
	ex: `${verb} WHERE pod = "payments-api" AND resolved = true`,
	c: [
		{ k: '=  !=  <  <=  >  >=', t: 'operator', d: 'Compare a field with a value. Strings in double quotes; numbers, true, false and null as they are.', ex: `RECALL FROM EPISODIC WHERE resolution_time > 300` },
		{ k: 'BETWEEN a AND b', t: 'operator', d: 'A value in a range, both ends included. Also how to read a span of time.', ex: `RECALL FROM EPISODIC WHERE created_at BETWEEN "2026-11-01T00:00:00Z" AND "2026-11-08T00:00:00Z"` },
		{ k: 'IN ( … )', t: 'operator', d: 'The field equals any value in the list.', ex: `RECALL FROM EPISODIC WHERE pod IN ("payments-api", "checkout")` },
		{ k: 'CONTAINS  STARTS_WITH  ENDS_WITH', t: 'operator', d: 'Match part of a text field.', ex: `RECALL FROM SEMANTIC WHERE key STARTS_WITH "person:"` },
		{ k: 'AND  OR  ( )', t: 'logic', d: 'Combine conditions. Brackets make the grouping explicit.', ex: `RECALL FROM EPISODIC WHERE (pod = "a" OR pod = "b") AND resolved = true` },
		{ k: '{name}  $name', t: 'value', d: 'A variable instead of a literal: bound by the request, or by an earlier stage of a PIPELINE.', ex: `RECALL FROM EPISODIC WHERE url = {url} LIMIT 10` },
	],
});

const like: SchemaNode = {
	k: 'LIKE "text" | $var', t: 'predicate',
	d: 'Similarity: the records closest in meaning to some text or a vector, found by vector and word search together. Use it instead of WHERE.',
	ex: `RECALL FROM SEMANTIC LIKE $question CONFIDENCE 0.7 LIMIT 5`,
	c: [
		{ k: 'CONFIDENCE c', t: 'modifier', d: 'At least this similar, from 0 to 1. Without it you get the best matches, however weak.', ex: `RECALL FROM PROCEDURAL LIKE "pod OOMKilled restart" CONFIDENCE 0.6` },
	],
};

const recent: SchemaNode = {
	k: 'WINDOW LAST n | duration', t: 'modifier',
	d: 'The newest items: the last n, or everything from the last 30s, 5m, 1h… Works on anything that has a time.',
	ex: `RECALL FROM WORKING WINDOW LAST 30s`,
	c: [
		{ k: 'LAST n', t: 'option', d: 'The n most recent items.', ex: `RECALL FROM WORKING WINDOW LAST 10` },
		{ k: 'LAST duration', t: 'option', d: 'Everything from the last span of time: a number and ms, s, m, h or d.', ex: `RECALL FROM EPISODIC WHERE pod = "payments-api" WINDOW LAST 1h` },
	],
};

const shape: SchemaNode = {
	k: 'shape the result', t: 'modifiers',
	d: 'Order, cut and trim what comes back, so the LLM gets a few useful rows instead of everything.',
	ex: `RECALL FROM EPISODIC WHERE outcome = "success"
  ORDER BY confidence DESC LIMIT 5
  RETURN incident_id, action, confidence`,
	c: [
		{ k: 'ORDER BY field [ASC | DESC]', t: 'modifier', d: 'Sort by one field. Ascending unless you say DESC. With LIMIT, also how to take the top n by a field.', ex: `RECALL FROM WORKING ORDER BY attention_weight DESC LIMIT 3` },
		{ k: 'LIMIT n', t: 'modifier', d: 'Return at most n records.', ex: `RECALL FROM EPISODIC WHERE pod = "payments-api" LIMIT 10` },
		{ k: 'RETURN fields', t: 'modifier', d: 'Return only these fields, or * for all of them. Keeps the context small.', ex: `RECALL FROM SEMANTIC WHERE key = "service:payments-api" RETURN owner, memory_limit` },
	],
};

const summarise: SchemaNode = {
	k: 'summarise', t: 'modifiers',
	d: 'Turn many records into a few numbers. Often the LLM needs a signal, not fifty rows.',
	ex: `RECALL FROM EPISODIC WHERE strategy = "tech_news_premium"
  AGGREGATE COUNT(*) AS uses, AVG(ctr) AS avg_ctr
  HAVING uses > 10 AND avg_ctr < 0.01`,
	c: [
		{ k: 'AGGREGATE f(field) AS name', t: 'modifier', d: 'Compute COUNT, AVG, SUM, MIN or MAX over the matched records. COUNT(*) counts them. AS names the result.', ex: `RECALL FROM EPISODIC WHERE root_cause = "OOMKilled"
  AGGREGATE COUNT(*) AS incidents, AVG(resolution_time) AS avg_minutes` },
		{ k: 'GROUP BY fields', t: 'modifier', d: 'With AGGREGATE: one result row per group instead of one overall.', ex: `RECALL FROM EPISODIC WHERE outcome = "won"
  AGGREGATE COUNT(*) AS wins GROUP BY url` },
		{ k: 'HAVING condition', t: 'modifier', d: 'Filter on the aggregated values. Needs AGGREGATE.', ex: `RECALL FROM EPISODIC WHERE outcome = "won"
  AGGREGATE COUNT(*) AS wins GROUP BY url
  HAVING wins > 5` },
	],
};

const linkSelect: SchemaNode[] = [
	{ k: 'TYPE "t"', t: 'option', d: 'One link type, by name. Without TYPE, every type.', ex: `RECALL FROM SEMANTIC WHERE key = "concept:k8s-oom" FOLLOW LINKS TYPE "triggers"` },
	{ k: 'TYPE ("a", "b")', t: 'option', d: 'Several link types.', ex: `RECALL FROM SEMANTIC WHERE key = "person:ada" FOLLOW LINKS TYPE ("works_at", "based_in")` },
	{ k: 'WEIGHT w', t: 'option', d: 'Only links with at least this weight.', ex: `RECALL FROM PROCEDURAL WHERE pattern_id = "oom-fix" WITH LINKS TYPE "applied_to" WEIGHT 0.8` },
];

const links: SchemaNode = {
	k: 'read the graph', t: 'modifiers',
	d: 'Records are connected by LINKs. These modifiers show the links behind a record, or walk along them to other records, even in another memory type. Links are read in both directions.',
	ex: `RECALL FROM SEMANTIC WHERE key = "concept:k8s-oom"
  FOLLOW LINKS TYPE "triggers"
  RETURN pattern_id, steps, confidence`,
	c: [
		{ k: 'WITH LINKS', t: 'modifier', d: 'Return each record with its evidence: its links of each type, and their weights.', ex: `RECALL FROM PROCEDURAL WHERE pattern_id = "oom-fix" WITH LINKS`, c: linkSelect },
		{
			k: 'FOLLOW LINKS', t: 'modifier', d: 'Walk along links and return the records at the other end. Starting in SEMANTIC and following "triggers" can return PROCEDURAL records. Links to a record\'s aliases count too.',
			ex: `RECALL FROM SEMANTIC WHERE key = "org:acme" FOLLOW LINKS TYPE "works_at"`,
			c: [...linkSelect, { k: 'DEPTH n', t: 'option', d: 'How many hops to walk. One by default.', ex: `RECALL FROM SEMANTIC WHERE key = "person:ada" FOLLOW LINKS TYPE "knows" DEPTH 2` }],
		},
	],
};

const history: SchemaNode = {
	k: 'read the past', t: 'modifiers',
	d: 'UPDATE keeps the old version of a record. These modifiers read memory as it was, not just as it is. A time is a timestamp or a variable.',
	ex: `RECALL FROM SEMANTIC WHERE key = "person:ada" AS OF "2026-10-01T00:00:00Z"`,
	c: [
		{ k: 'AS OF time', t: 'modifier', d: 'What memory held at that moment.', ex: `RECALL FROM SEMANTIC WHERE key = "product:x1" AS OF $quarter_end RETURN price` },
		{ k: 'WITH HISTORY', t: 'modifier', d: 'Every version, current and replaced, oldest first.', ex: `RECALL FROM PROCEDURAL WHERE pattern_id = "deploy" WITH HISTORY` },
		{ k: 'WITH HISTORY BETWEEN a AND b', t: 'modifier', d: 'Only the versions written in that span.', ex: `RECALL FROM SEMANTIC WHERE key = "person:ada" WITH HISTORY BETWEEN "2026-01-01T00:00:00Z" AND "2026-06-30T00:00:00Z"` },
	],
};

const whose = (read: boolean): SchemaNode => ({
	k: 'whose memory', t: 'modifiers',
	d: read
		? 'Every statement works in a namespace: its own NAMESPACE, else the USE in force. A read may list several namespaces.'
		: 'A write works in exactly one namespace: its own NAMESPACE, else the USE in force.',
	ex: read ? `RECALL FROM SEMANTIC LIKE $q NAMESPACE ("crm", "kb") LIMIT 5` : `STORE INTO EPISODIC (event = "called", customer = "person:ada") NAMESPACE "crm"`,
	c: [
		{ k: 'NAMESPACE "ns"', t: 'modifier', d: 'Whose memory: an agent, a project, a branch. A server may require it on every statement.', ex: `RECALL FROM EPISODIC WHERE pod = "payments-api" NAMESPACE "ops"` },
		...(read ? [{ k: 'NAMESPACE ("a", "b")', t: 'modifier', d: 'One read across several namespaces, ranked together. Reads only: writes, links and topics are in one namespace.', ex: `RECALL FROM SEMANTIC LIKE "refund policy" NAMESPACE ("crm", "kb") LIMIT 5` }] : []),
		{ k: 'IN SCOPE "s"', t: 'modifier', d: 'A shared group inside the namespace, such as one team.', ex: read ? `RECALL FROM SEMANTIC IN SCOPE "team:billing" WHERE kind = "customer"` : `STORE INTO SEMANTIC (key = "note:renewal", text = "renewal due") IN SCOPE "team:billing"` },
	],
});

const ttl: SchemaNode = { k: 'TTL d', t: 'modifier', d: 'Forget this memory automatically after a duration: 30s, 15m, 12h, 7d.', ex: `STORE INTO EPISODIC (event = "login", user_id = "u123") TTL 7d` };

const payload: SchemaNode = {
	k: '( field = value, … )', t: 'payload', r: true,
	d: 'The record, as field = value pairs in brackets. Fields are free: the agent decides what to keep. Values can be strings, numbers, true, false, null, lists or variables.',
	ex: `STORE INTO EPISODIC (incident_id = "inc-001", pod = "payments-api", resolved = true, resolution_time = 420)`,
};

const ends = (verb: string, type: string): SchemaNode[] => [
	{ k: 'FROM memory WHERE …', t: 'clause', r: true, d: 'The records the links start at. One memory type, not ALL.', ex: `${verb} FROM PROCEDURAL WHERE pattern_id = "oom-fix" INTO EPISODIC WHERE incident_id = "inc-001" ${type}` },
	{ k: 'INTO memory WHERE …', t: 'clause', r: true, d: 'The records the links end at. It can be a different memory type.', ex: `${verb} FROM SEMANTIC WHERE key = "concept:auth" INTO PROCEDURAL WHERE pattern_id = "rotate-keys" ${type}` },
];

const nsPair = (verb: string, what: string): SchemaNode => ({
	k: 'FROM "ns" INTO "ns"', t: 'clause', r: true,
	d: `${what} The two names must differ. USE doesn't apply: these verbs always name their namespaces.`,
	ex: `${verb} FROM "crm" INTO "crm-try"`,
});

export const aqlSchema: SchemaNode[] = [
	// ── Memory ─────────────────────────────────────────────────────────────
	{
		k: 'RECALL', t: 'read verb',
		d: 'Every read. Exact lookups, filters, similarity, the tools for a task, the session\'s latest state: one verb, and the engine picks the access path from what you ask.',
		ex: `RECALL FROM EPISODIC WHERE outcome = "success"
  ORDER BY confidence DESC LIMIT 5`,
		c: [
			{
				k: 'FROM memory', t: 'clause', r: true, d: 'Which memory to read. ALL reads every type at once: "tell me everything about this URL".',
				ex: `RECALL FROM ALL WHERE url = "example.com/news"`,
				c: memories(pick('EPISODIC', 'SEMANTIC', 'PROCEDURAL', 'TOOLS', 'WORKING', 'ALL')),
			},
			{ ...where('RECALL FROM EPISODIC'), d: 'Conditions on fields. A key is just a field: WHERE key = "person:ada" is an exact lookup. No WHERE reads everything, within LIMIT.' },
			like, recent, shape, summarise, links, history, whose(true),
		],
	},
	{
		k: 'STORE', t: 'write verb',
		d: 'Write a new memory. A write names one memory type: the agent decides whether this is an event, knowledge or a procedure.',
		ex: `STORE INTO EPISODIC (event = "login", user_id = "u123") TTL 7d`,
		c: [
			{
				k: 'INTO memory', t: 'clause', r: true, d: 'Exactly one memory type. STORE INTO ALL is an error.',
				ex: `STORE INTO PROCEDURAL (pattern_id = "oom-fix", steps = "raise limit, restart", confidence = 0.6)`,
				c: memories(pick('EPISODIC', 'SEMANTIC', 'PROCEDURAL', 'TOOLS', 'WORKING')),
			},
			payload, ttl, whose(false),
		],
	},
	{
		k: 'UPDATE', t: 'write verb',
		d: 'Change fields of records that exist. The old version is kept as history, so AS OF and WITH HISTORY can still read it. A value is a field: there are no separate facts.',
		ex: `UPDATE INTO SEMANTIC WHERE key = "person:ada" (age = 36)`,
		c: [
			{ k: 'INTO memory', t: 'clause', r: true, d: 'One memory type, not ALL.', ex: `UPDATE INTO PROCEDURAL WHERE pattern_id = "oom-fix" (confidence = 0.94, success_count = 8)` },
			{ ...where('UPDATE INTO EPISODIC'), r: true, d: 'Which records to change. Required: an UPDATE never touches everything.', ex: `UPDATE INTO EPISODIC WHERE incident_id = "inc-001" (resolved = true)`, c: undefined },
			{ ...payload, d: 'The fields to set. Fields you leave out keep their values; set one to null to clear it.', ex: `UPDATE INTO SEMANTIC WHERE key = "person:ada" (title = null, team = "research")` },
			{
				k: '(aliases = [ … ])', t: 'field',
				d: 'Two keys turn out to name one thing. Give the survivor the other key as an alias, then FORGET the duplicate: reads, links and stored links by the alias all reach the survivor. Nothing is rewritten. (This replaces MERGE ENTITY.)',
				ex: `UPDATE INTO SEMANTIC WHERE key = "person:ada" (aliases = ["person:ada-lovelace"]) NAMESPACE "crm";
FORGET FROM SEMANTIC WHERE key = "person:ada-lovelace" NAMESPACE "crm"`,
			},
			ttl, whose(false),
		],
	},
	{
		k: 'FORGET', t: 'write verb',
		d: 'Erase memories. A forgotten record leaves a tombstone, so no read can still see it.',
		ex: `FORGET FROM WORKING WHERE stale = true`,
		c: [
			{ k: 'FROM memory', t: 'clause', r: true, d: 'One memory type, or ALL to forget a record wherever it is.', ex: `FORGET FROM ALL WHERE user_id = "u123" NAMESPACE "crm"`, c: memories(pick('EPISODIC', 'SEMANTIC', 'PROCEDURAL', 'TOOLS', 'WORKING', 'ALL')) },
			{ ...where('FORGET FROM EPISODIC'), d: 'Which records to forget.', c: undefined },
			{
				k: 'no WHERE', t: 'option',
				d: 'Forgets everything in the memory named. It must name its NAMESPACE, so it can never happen by accident. FORGET FROM ALL with a NAMESPACE drops the whole namespace, which is also how a branch is dropped.',
				ex: `FORGET FROM ALL NAMESPACE "crm-try"`,
			},
			whose(false),
		],
	},
	{
		k: 'LINK', t: 'graph verb',
		d: 'Connect records with a typed, weighted link: the evidence behind a procedure, an entity\'s relationships. Relationships are links: "Ada works at Acme" is a LINK. On a link that exists, LINK sets its weight.',
		ex: `LINK FROM SEMANTIC WHERE key = "person:ada"
  INTO SEMANTIC WHERE key = "org:acme"
  TYPE "works_at"`,
		c: [
			...ends('LINK', 'TYPE "applied_to"'),
			{ k: 'TYPE "name"', t: 'clause', r: true, d: 'What the link means. Several links of one TYPE give several objects.', ex: `LINK FROM EPISODIC WHERE incident_id = "inc-001" INTO EPISODIC WHERE incident_id = "inc-005" TYPE "recurring"` },
			{ k: 'WEIGHT w', t: 'option', d: 'How strong the link is, from 0 to 1. Run LINK again with a new WEIGHT to change it.', ex: `LINK FROM PROCEDURAL WHERE pattern_id = "oom-fix" INTO EPISODIC WHERE incident_id = "inc-001" TYPE "applied_to" WEIGHT 0.97` },
			{ k: 'NAMESPACE "ns"', t: 'modifier', d: 'Both ends are in this namespace.', ex: `LINK FROM SEMANTIC WHERE key = "person:ada" INTO SEMANTIC WHERE key = "org:acme" TYPE "works_at" NAMESPACE "crm"` },
		],
	},
	{
		k: 'UNLINK', t: 'graph verb',
		d: 'Remove links between records. The records themselves stay.',
		ex: `UNLINK FROM SEMANTIC WHERE key = "person:ada" INTO SEMANTIC WHERE key = "org:acme" TYPE "works_at"`,
		c: [
			...ends('UNLINK', 'TYPE "applied_to"'),
			{ k: 'TYPE "name"', t: 'option', d: 'Remove only links of this type. Without it, every link between those records goes.', ex: `UNLINK FROM SEMANTIC WHERE key = "a" INTO SEMANTIC WHERE key = "b"` },
			{ k: 'NAMESPACE "ns"', t: 'modifier', d: 'Both ends are in this namespace.', ex: `UNLINK FROM SEMANTIC WHERE key = "person:ada" INTO SEMANTIC WHERE key = "org:acme" TYPE "works_at" NAMESPACE "crm"` },
		],
	},

	// ── Namespaces ─────────────────────────────────────────────────────────
	{
		k: 'BRANCH', t: 'namespace verb',
		d: 'A new namespace that reads through its base at a point in time, plus its own writes. Nothing is copied, and writes stay in the branch. Use it to try something against real memory.',
		ex: `BRANCH FROM "crm" INTO "crm-try"`,
		c: [
			nsPair('BRANCH', 'FROM is the base; INTO is the new branch. A branch can be branched again.'),
			{ k: 'AS OF time', t: 'option', d: 'Branch from the base as it was at that moment, not as it is now.', ex: `BRANCH FROM "crm" INTO "crm-before-import" AS OF "2026-11-01T09:00:00Z"` },
			{ k: 'drop a branch', t: 'note', d: 'A branch is a namespace, so FORGET drops it. Dropping a namespace that still has branches is refused, naming them.', ex: `FORGET FROM ALL NAMESPACE "crm-try"` },
		],
	},
	{
		k: 'FORK', t: 'namespace verb',
		d: 'BRANCH with a full copy. The fork no longer reads through its base, so later changes to the base, or dropping it, don\'t reach it. It remembers where it came from, so it can still be merged either way.',
		ex: `FORK FROM "crm" INTO "crm-copy"`,
		c: [
			nsPair('FORK', 'FROM is copied into the new namespace INTO.'),
			{ k: 'AS OF time', t: 'option', d: 'Copy the records as they were at that moment.', ex: `FORK FROM "crm" INTO "crm-nov-1" AS OF "2026-11-01T00:00:00Z"` },
		],
	},
	{
		k: 'MERGE', t: 'namespace verb',
		d: 'Apply the changes FROM made since the two parted to INTO, in one write. Merging a base into its branch moves the branch forward (what rebase did). Two namespaces with no common base can\'t be merged.',
		ex: `MERGE FROM "crm-try" INTO "crm"`,
		c: [
			nsPair('MERGE', 'FROM is the side whose changes are applied; INTO receives them.'),
			{
				k: 'PREFER SOURCE | TARGET', t: 'option',
				d: 'When both sides changed the same record, MERGE fails and lists the conflicts. PREFER says which side wins instead: SOURCE is FROM, TARGET is INTO.',
				ex: `MERGE FROM "crm-try" INTO "crm" PREFER SOURCE`,
			},
			{ k: 'rebase', t: 'note', d: 'Merge the base into the branch to bring in its newer changes before you test again.', ex: `MERGE FROM "crm" INTO "crm-try"` },
		],
	},

	// ── Flow ───────────────────────────────────────────────────────────────
	{
		k: 'PIPELINE', t: 'flow verb',
		d: 'Several reads in one request, in order, within a time budget, ending in one context for the LLM. The last stage may write, to keep what was learned.',
		ex: `PIPELINE bid TIMEOUT 80ms
  RECALL FROM TOOLS WHERE task = "bidding" LIMIT 3
  | RECALL FROM SEMANTIC WHERE key = {url}
  | RECALL FROM EPISODIC WHERE url = {url} LIMIT 10`,
		c: [
			{ k: 'name', t: 'option', d: 'An optional name, for logs and traces.', ex: `PIPELINE triage RECALL FROM WORKING WINDOW LAST 60s | RECALL FROM EPISODIC WHERE pod = {pod} LIMIT 5` },
			{ k: 'TIMEOUT d', t: 'option', d: 'The time budget for the whole pipeline. TIMEOUT exists only here.', ex: `PIPELINE TIMEOUT 200ms RECALL FROM WORKING WINDOW LAST 60s | RECALL FROM EPISODIC WHERE pod = {pod} LIMIT 5` },
			{
				k: 'stage | stage', t: 'clause', r: true,
				d: 'RECALLs separated by |. {name} uses a value from an earlier stage. Each stage may name its own NAMESPACE.',
				ex: `PIPELINE enrich
  RECALL FROM SEMANTIC WHERE key = {customer} NAMESPACE "crm"
  | STORE INTO EPISODIC (customer = {customer}, seen = true) NAMESPACE "ops"`,
			},
			{
				k: 'write last', t: 'option',
				d: 'The last stage may be a STORE or UPDATE: read the experience, then write back what was learned (what REFLECT … THEN did).',
				ex: `PIPELINE learn
  RECALL FROM EPISODIC WHERE incident_id = {current}
  | RECALL FROM PROCEDURAL WHERE pattern_id = {matched}
  | STORE INTO PROCEDURAL (pattern_id = "learned", steps = "step1, step2", confidence = 0.6)`,
			},
		],
	},
	{
		k: 'SUBSCRIBE', t: 'flow verb',
		d: 'Follow changes as they happen: messages on a topic, or a live query whose result updates as memory changes. A subscription starts now.',
		ex: `SUBSCRIBE TOPIC "orders" WHERE status = "paid" NAMESPACE "shop"`,
		c: [
			{ k: 'TOPIC "name"', t: 'clause', d: 'Messages PUBLISHed to this topic. WHERE filters them.', ex: `SUBSCRIBE TOPIC "orders" WHERE status = "paid" NAMESPACE "shop"` },
			{ k: 'RECALL …', t: 'clause', d: 'A live query (what WATCH was): any RECALL, delivered again whenever its result changes.', ex: `SUBSCRIBE RECALL FROM SEMANTIC WHERE kind = "customer" LIMIT 20 NAMESPACE "crm"` },
			{ k: 'delivery', t: 'note', d: 'Acknowledgements, resuming from a position and how often a live query re-runs are the server\'s API and settings, not AQL.', ex: '' },
		],
	},
	{
		k: 'UNSUBSCRIBE', t: 'flow verb',
		d: 'Stop a subscription or a live query.',
		ex: `UNSUBSCRIBE "sub-7f3a"`,
		c: [{ k: 'handle', t: 'clause', r: true, d: 'The id returned when the subscription was created: a string, a variable or a name.', ex: `UNSUBSCRIBE $sub` }],
	},
	{
		k: 'PUBLISH', t: 'flow verb',
		d: 'Send a message to a topic, for every subscriber.',
		ex: `PUBLISH TOPIC "orders" (order_id = "o-1", status = "paid") TTL 7d NAMESPACE "shop"`,
		c: [
			{ k: 'TOPIC "name"', t: 'clause', r: true, d: 'Where to publish. Topics are in one namespace.', ex: `PUBLISH TOPIC "alerts" (pod = "payments-api", level = "high")` },
			{ ...payload, ex: `PUBLISH TOPIC "orders" (order_id = "o-1", status = "paid")` },
			{ ...ttl, d: 'How long the message is kept for subscribers.', ex: `PUBLISH TOPIC "orders" (order_id = "o-1", status = "paid") TTL 7d` },
		],
	},

	// ── Session ────────────────────────────────────────────────────────────
	{
		k: 'USE', t: 'session',
		d: 'Name the namespace once. Statements after it that name no namespace work there; a statement\'s own NAMESPACE still wins.',
		ex: `USE "crm";
RECALL FROM SEMANTIC WHERE key = "person:ada";
STORE INTO EPISODIC (event = "called", customer = "person:ada")`,
		c: [
			{
				k: 'script', t: 'clause',
				d: 'Statements separated by ; are one request. Its answer is the last statement\'s. A failing statement stops the script and is named by position; the ones before it stay done.',
				ex: `USE "ops";
STORE INTO EPISODIC (event = "deploy", service = "payments-api");
RECALL FROM EPISODIC WHERE service = "payments-api" ORDER BY created_at DESC LIMIT 3`,
			},
			{ k: 'session', t: 'note', d: 'A connection that keeps state (MCP, Flight) keeps its USE between requests. A plain HTTP request uses a script.', ex: '' },
			{ k: 'own NAMESPACE wins', t: 'note', d: 'USE only fills in a missing namespace. BRANCH, FORK and MERGE always name theirs.', ex: `USE "crm";
RECALL FROM EPISODIC WHERE pod = "x" NAMESPACE "ops"` },
		],
	},
];
