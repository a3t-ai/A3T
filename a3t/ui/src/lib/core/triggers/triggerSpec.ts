// A trigger as YAML: what fires it (`on`), and where each firing goes (`send`),
// with the mapping from the event to each target's input. The Triggers page and
// the Studio edit the same text; FlowR stores the trigger it describes.
//
//   name: refunds
//   project: default
//   description: Refund requests from the shop
//   enabled: true
//   on:
//     webhook: { path: /shop/refund, method: POST }
//   send:
//     - flow: default/flow_default_refund
//       input: '${ {order: .body.order_id, amount: .body.amount} }'
//     - agent: agent_default_support
//       message: '${ "Refund asked for order " + (.body.order_id | tostring) }'
//       conversation: '${ .body.customer_id }'
//     - url: https://hooks.example.com/refunds
//
// Every expression is jq over the event: { body, message, source, trigger, time }.
// `body` is the webhook's JSON body, the topic message's fields, or for a schedule
// { trigger: cron, fired_at, minute }.
import * as YAML from 'yaml';

export type SourceKind = 'webhook' | 'cron' | 'adb_topic' | 'event';

export interface StoredTarget {
  type: 'workflow' | 'a3t_agent' | 'webhook' | 'agent';
  namespace?: string; name?: string; input_mapping?: string | null;
  agent_id?: string; message?: string | null; conversation?: string | null;
  url?: string; headers?: Record<string, string> | null;
}
export interface StoredTrigger {
  id?: string; name: string; namespace: string; description?: string; enabled: boolean;
  config: { type: SourceKind; [k: string]: any };
  targets: StoredTarget[];
}

/// What each source puts in `.body`, for the people writing mappings.
export const BODY_OF: Record<SourceKind, string> = {
  webhook: 'the JSON body the caller POSTs',
  cron: '{ trigger: "cron", trigger_id, fired_at, minute }',
  adb_topic: "the message's fields, plus _topic and _offset",
  event: 'the event: { specversion, id, type, source, data, … }',
};

const clean = (o: Record<string, any>) => Object.fromEntries(Object.entries(o).filter(([, v]) => v !== undefined && v !== null && v !== ''));

/// The YAML for a trigger as FlowR stores it.
export function toYaml(t: StoredTrigger): string {
  const c = t.config ?? { type: 'webhook' };
  const on: Record<string, any> =
    c.type === 'webhook' ? { webhook: clean({ path: c.path, method: c.method ?? 'POST', message: c.message_template }) }
    : c.type === 'cron' ? { cron: clean({ expression: c.expression, timezone: c.timezone, message: c.message }) }
    : c.type === 'adb_topic' ? { adb_topic: clean({ resource: c.resource, topic: c.topic, filter: c.filter, message: c.message_template }) }
    : { event: clean({ source: c.source, filter: c.filter, message: c.message }) };
  const send = (t.targets ?? []).map((g) =>
    g.type === 'workflow' ? clean({ flow: `${g.namespace}/${g.name}`, input: g.input_mapping })
    : g.type === 'a3t_agent' ? clean({ agent: g.agent_id, message: g.message, conversation: g.conversation })
    : g.type === 'webhook' ? clean({ url: g.url, headers: g.headers })
    : clean({ inbox: g.agent_id }));
  const doc = clean({ name: t.name, project: t.namespace, description: t.description, enabled: t.enabled !== false, on, send });
  return YAML.stringify(doc, { lineWidth: 0 });
}

export interface Parsed { trigger: StoredTrigger | null; issues: string[] }

/// The trigger a YAML text describes, or what is wrong with it.
export function fromYaml(text: string): Parsed {
  let d: any;
  try { d = YAML.parse(text); } catch (e) { return { trigger: null, issues: [`Not YAML: ${(e as Error).message}`] }; }
  const issues: string[] = [];
  if (!d || typeof d !== 'object') return { trigger: null, issues: ['Empty.'] };
  if (!d.name || typeof d.name !== 'string') issues.push('name: give it a name.');
  const on = d.on ?? {};
  const kinds = Object.keys(on).filter((k) => ['webhook', 'cron', 'adb_topic', 'event'].includes(k));
  if (kinds.length !== 1) issues.push('on: exactly one of webhook, cron, adb_topic or event.');
  const kind = (kinds[0] ?? 'webhook') as SourceKind;
  const s = on[kind] ?? {};
  let config: StoredTrigger['config'];
  if (kind === 'webhook') {
    if (!s.path) issues.push('on.webhook.path: the path it listens on, e.g. /shop/refund.');
    config = clean({ type: 'webhook', path: s.path ? (String(s.path).startsWith('/') ? s.path : `/${s.path}`) : '', method: s.method ?? 'POST', message_template: s.message }) as any;
  } else if (kind === 'cron') {
    if (!s.expression) issues.push('on.cron.expression: when it fires, e.g. "*/5 * * * *".');
    config = clean({ type: 'cron', expression: s.expression, timezone: s.timezone, message: s.message ?? '' }) as any;
    if (!config.message) config.message = '';
  } else if (kind === 'adb_topic') {
    if (!s.resource || !s.topic) issues.push('on.adb_topic: resource (an ADB resource) and topic.');
    config = clean({ type: 'adb_topic', resource: s.resource, topic: s.topic, filter: s.filter, message_template: s.message }) as any;
  } else {
    if (!s.source) issues.push('on.event.source: the event type, e.g. flow.completed.');
    config = clean({ type: 'event', source: s.source, filter: s.filter, message: s.message }) as any;
  }
  const send: any[] = Array.isArray(d.send) ? d.send : [];
  if (!send.length) issues.push('send: at least one target (flow, agent or url).');
  const targets: StoredTarget[] = [];
  send.forEach((g, i) => {
    if (g?.flow) {
      const [ns, ...rest] = String(g.flow).split('/');
      if (!rest.length) issues.push(`send[${i}].flow: "<project>/<flow>".`);
      targets.push(clean({ type: 'workflow', namespace: ns, name: rest.join('/'), input_mapping: g.input }) as StoredTarget);
    } else if (g?.agent) targets.push(clean({ type: 'a3t_agent', agent_id: String(g.agent), message: g.message, conversation: g.conversation }) as StoredTarget);
    else if (g?.url) targets.push(clean({ type: 'webhook', url: String(g.url), headers: g.headers }) as StoredTarget);
    else if (g?.inbox) targets.push({ type: 'agent', agent_id: String(g.inbox) });
    else issues.push(`send[${i}]: one of flow, agent or url.`);
    for (const k of ['input', 'message', 'conversation'] as const) {
      const v = g?.[k];
      if (v != null && typeof v !== 'string') issues.push(`send[${i}].${k}: a jq expression in a string, e.g. '\${ .body }'.`);
    }
  });
  return {
    trigger: { name: String(d.name ?? ''), namespace: String(d.project ?? 'default'), description: d.description ?? '', enabled: d.enabled !== false, config, targets },
    issues,
  };
}

/// A plain label for a target.
export function targetLabel(g: StoredTarget): string {
  return g.type === 'workflow' ? `flow ${g.namespace}/${g.name}` : g.type === 'a3t_agent' ? `agent ${g.agent_id}` : g.type === 'webhook' ? `url ${g.url}` : `inbox ${g.agent_id}`;
}
