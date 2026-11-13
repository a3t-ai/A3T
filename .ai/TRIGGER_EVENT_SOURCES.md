# Trigger event sources: Kafka, Postgres, Pulsar — parked (WASM source plugins in FlowR → ADB topic)

*13 Nov 2026. Status: **parked**. The user, 13 Nov: "let ADB connect to Kafka, PG and Pulsar and trigger events — park it in A3T"; then: "FlowR has a WASM plugin module — ideally listeners and other resources live there" (see Preferred below).*

## The shape

A3T does not consume Kafka, Postgres or Pulsar itself. **ADB connects to them** and
publishes each event to an **ADB topic**; an A3T trigger with `on: adb_topic` starts
flows and agents from that topic, with the same mapping as any other trigger.

```
Kafka topic ─┐
PG changes  ─┼─▶ ADB connector ─▶ ADB topic ─▶ A3T trigger (on: adb_topic) ─▶ flows / agents
Pulsar topic─┘                                 (send: … input / message mapped in jq)
```

So one path: one dedupe (the ADB offset, CA-10), one retention, one place where the
broker credentials live (an ADB resource), and every FlowR replica sees the same
stream.

## What A3T has now (built 13 Nov)

- `adb_topic` triggers: once per message on a topic, on every FlowR, one run per
  message and target (`_dedupe`, CA-2/CA-10).
- Mapping for every target (`input` for a flow; `message`, `conversation` for an
  agent): jq over `{ body, message, source, trigger, time }`; `.body` is the topic
  message's fields plus `_topic`, `_offset`.
- Triggers as YAML (`on` / `send`), the same text on the Triggers page and in the Studio.

## The request to ADB

| ID | Request | Done when |
|---|---|---|
| ES-1 | Kafka connector: an ADB resource with brokers, topic(s), consumer group and credentials; each record published to an ADB topic as `{key, value (JSON if it parses), headers, partition, offset, timestamp}` | A message produced to Kafka starts an A3T trigger's flow once |
| ES-2 | Postgres connector: logical replication (or LISTEN/NOTIFY for simple cases) of chosen tables; each change published as `{table, op: insert/update/delete, new, old, lsn, at}` | An INSERT into a watched table starts a flow once, with the row in `.body.new` |
| ES-3 | Pulsar connector: subscription to a topic; each message as `{key, value, properties, message_id, publish_time}` | A message published to Pulsar starts a flow once |
| ES-4 | At-least-once into ADB, ordered per partition / key; the connector's position kept in ADB so a restart resumes | Restarting the ADB pod loses and repeats nothing visible to A3T (ADB offsets dedupe) |

## Preferred: listeners as FlowR WASM plugins (the user, 13 Nov)

*"FlowR has a WASM plugin module — ideally listeners and other resources live there."*

So the connector is a **WASM source plugin run by FlowR**, not code inside ADB. The ADB
topic stays as the hand-off, so dedupe, retention and positions stay in one place:

```
Kafka / PG / Pulsar ─▶ WASM source plugin (in FlowR, one owner) ─▶ ADB topic ─▶ adb_topic trigger ─▶ flows / agents
```

What `shared/flowr-wasm` has today: WASM **task handlers** only (one call, JSON in → JSON out,
host capability `log`), a core-module ABI, no CPU/time limits, and **not yet loaded by
flowr-ce** (only its own tests use it). A listener needs more:

| ID | Work | Notes |
|---|---|---|
| WP-1 | Plugin kinds: `task` (today) and `source` (long-lived listener); later `resource` (a client for an external system that flows call) | One plugin manifest: name, kind, version, the capabilities it asks for, the config schema (an A3T resource holds the config and credentials) |
| WP-2 | Move to the component model (WIT): `source` exports `start(config)`, `poll(max) -> list<event>`, `ack(position)`; host imports `emit(topic, event)`, `kv` (positions), `log` | The plan already in the crate docs; WIT gives typed, per-plugin capability grants |
| WP-3 | Network capability: WASI sockets / TLS granted per plugin, only to the hosts in its resource | A plugin that asks for nothing can reach nothing (the boundary the crate already has) |
| WP-4 | One owner per source across FlowR replicas: a lease in ADB; the plugin's position kept in ADB (ES-4) so another FlowR resumes it | Without this every replica would consume the same stream |
| WP-5 | Limits: fuel or epoch interruption, memory cap, restart with backoff | Not in the crate today |
| WP-6 | Load plugins in flowr-ce: from the registry (an ADB blob or OCI), signed; EE decides who may load and grant what (Cedar, `aesop.task.source = "wasm"`) | Wire-up missing today |
| WP-7 | First plugins: Postgres (logical replication), Kafka, Pulsar, written in Rust against the WIT | Kafka and Pulsar need pure-Rust clients that build for wasm32-wasip2 (rdkafka is C); try that first, before committing to a broker |

With WASM plugins, ES-1..ES-3 stop being ADB requests: they become plugins A3T owns.
**ES-4 is still asked of ADB:** keep positions and leases, and dedupe by offset on the topic.

## On the A3T side, when they land

- The Triggers page's "On" step: Kafka, Postgres and Pulsar shown as sources that pick
  the ADB connector resource and fill in `adb_topic` (no new trigger type).
- `BODY_OF` in `triggerSpec.ts` per connector, so mappings say what `.body` holds.
- A test per source in `e2e-triggers.mjs`.
