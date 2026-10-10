// kafkajs against the LawTwin Kafka twin. Each scenario asserts what the
// client observes and prints a JSON summary; the Rust test that runs it then
// checks the contract's laws over the twin's history.
//
//   [LAWTWIN_NS=<prefix>] node scenarios.mjs <scenario> <bootstrap> <admin url>
import { Kafka, logLevel } from "kafkajs";
import assert from "node:assert/strict";

const [scenario, bootstrap, admin] = process.argv.slice(2);
// LAWTWIN_NS prefixes every topic, group and transactional id, so that
// scenarios share one real broker without meeting.
const ns = process.env.LAWTWIN_NS ?? "";
const n = (name) => ns + name;
const kafka = new Kafka({
  clientId: "kafkajs-scenarios",
  brokers: [bootstrap],
  logLevel: logLevel.NOTHING,
  retry: { retries: 20, initialRetryTime: 100, maxRetryTime: 1000 },
});

async function control(method, path, body) {
  const r = await fetch(admin + path, {
    method,
    headers: { "content-type": "application/json" },
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  return { status: r.status, body: await r.json().catch(() => null) };
}

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

async function until(what, test, ms = 30000) {
  const end = Date.now() + ms;
  while (!(await test())) {
    if (Date.now() > end) throw new Error("timed out waiting for " + what);
    await sleep(50);
  }
}

async function createTopic(topic, numPartitions) {
  const a = kafka.admin();
  await a.connect();
  await a.createTopics({ topics: [{ topic, numPartitions }], waitForLeaders: true });
  await a.disconnect();
}

// A consumer that keeps what it reads and the assignments it is given.
async function reader(groupId, topic, options = {}) {
  const c = kafka.consumer({ groupId, sessionTimeout: 10000, heartbeatInterval: 500, maxWaitTimeInMs: 100, rebalanceTimeout: 10000, ...options });
  const seen = [];
  const joins = [];
  c.on(c.events.GROUP_JOIN, (e) => joins.push(e.payload.memberAssignment[topic] ?? []));
  await c.connect();
  await c.subscribe({ topic, fromBeginning: true });
  await c.run({
    autoCommit: options.autoCommit ?? true,
    eachMessage: async ({ partition, message }) => {
      seen.push({ partition, offset: Number(message.offset), value: message.value?.toString() });
    },
  });
  return { c, seen, joins, assigned: () => joins.at(-1) ?? [] };
}

function inOrder(seen) {
  const by = {};
  for (const r of seen) (by[r.partition] ??= []).push(r.offset);
  for (const offsets of Object.values(by)) {
    for (let i = 1; i < offsets.length; i++) assert.ok(offsets[i] > offsets[i - 1], "offsets increase within a partition: " + offsets);
  }
  return by;
}

const scenarios = {
  async "produce-consume"() {
    await createTopic(n("orders"), 3);
    const p = kafka.producer();
    await p.connect();
    for (let i = 0; i < 30; i++) {
      await p.send({ topic: n("orders"), acks: -1, messages: [{ key: "key-" + (i % 7), value: "v" + i }] });
    }
    await p.disconnect();
    const r = await reader(n("readers"), n("orders"));
    await until("30 records", () => r.seen.length >= 30);
    await r.c.disconnect();
    const by = inOrder(r.seen);
    for (const offsets of Object.values(by)) {
      offsets.forEach((o, i) => assert.equal(o, i, "dense offsets from 0"));
    }
    return { records: r.seen.length, partitions: Object.keys(by).length };
  },

  async "group-rebalance"() {
    await createTopic(n("work"), 4);
    const a = await reader(n("workers"), n("work"));
    await until("a owns 4", () => a.assigned().length === 4);
    const b = await reader(n("workers"), n("work"));
    await until("a split", () => a.assigned().length === 2 && b.assigned().length === 2);
    assert.equal(new Set([...a.assigned(), ...b.assigned()]).size, 4, "each partition has one owner");
    await b.c.disconnect();
    await until("a owns 4 again", () => a.assigned().length === 4);
    await a.c.disconnect();
    return { joinsOfA: a.joins.length };
  },

  async idempotent() {
    await createTopic(n("pay"), 1);
    await control("POST", "/v1/faults/duplicate-delivery", { count: 2 });
    const p = kafka.producer({ idempotent: true, maxInFlightRequests: 1 });
    await p.connect();
    for (let i = 0; i < 5; i++) await p.send({ topic: n("pay"), acks: -1, messages: [{ key: "k", value: "idem-" + i, partition: 0 }] });
    await p.disconnect();
    await control("POST", "/v1/faults/duplicate-delivery", { count: 1 });
    const plain = kafka.producer({ idempotent: false });
    await plain.connect();
    await plain.send({ topic: n("pay"), acks: -1, messages: [{ key: "k", value: "plain", partition: 0 }] });
    await plain.disconnect();
    const r = await reader(n("audit"), n("pay"));
    await until("7 records", () => r.seen.length >= 7);
    await sleep(300);
    await r.c.disconnect();
    assert.deepEqual(r.seen.map((x) => x.value), ["idem-0", "idem-1", "idem-2", "idem-3", "idem-4", "plain", "plain"]);
    return { values: r.seen.map((x) => x.value) };
  },

  async transactions() {
    await createTopic(n("ledger"), 1);
    const p = kafka.producer({ transactionalId: n("ledger-writer"), idempotent: true, maxInFlightRequests: 1 });
    await p.connect();
    const send = (tx, value) => tx.send({ topic: n("ledger"), messages: [{ key: "acct", value, partition: 0 }] });
    let tx = await p.transaction();
    for (let i = 0; i < 3; i++) await send(tx, "committed-" + i);
    await tx.commit();
    tx = await p.transaction();
    for (let i = 0; i < 2; i++) await send(tx, "aborted-" + i);
    await tx.abort();
    tx = await p.transaction();
    await send(tx, "committed-3");
    await tx.commit();
    const rc = await reader(n("rc"), n("ledger"), { readUncommitted: false });
    const ru = await reader(n("ru"), n("ledger"), { readUncommitted: true });
    await until("committed records", () => rc.seen.length >= 4 && ru.seen.length >= 6);
    await sleep(300);
    assert.deepEqual(rc.seen.map((x) => x.value), ["committed-0", "committed-1", "committed-2", "committed-3"]);
    assert.equal(ru.seen.length, 6);
    // A second instance fences the first.
    const fence = kafka.producer({ transactionalId: n("ledger-writer"), idempotent: true, maxInFlightRequests: 1 });
    await fence.connect();
    const t2 = await fence.transaction();
    await t2.abort().catch(() => {});
    let fenced = false;
    try {
      const zombie = await p.transaction();
      await send(zombie, "zombie");
      await zombie.commit();
    } catch (e) {
      fenced = true;
    }
    assert.ok(fenced, "the zombie is fenced");
    await Promise.all([rc.c.disconnect(), ru.c.disconnect(), p.disconnect().catch(() => {}), fence.disconnect()]);
    return { readCommitted: rc.seen.length, readUncommitted: ru.seen.length, fenced };
  },

  async faults() {
    await createTopic(n("f"), 1);
    const p = kafka.producer();
    await p.connect();
    const send = (value) => p.send({ topic: n("f"), acks: -1, messages: [{ key: "k", value, partition: 0 }] });
    await send("before");
    await control("POST", "/v1/faults/request-latency", { delay: "400ms", api: "Produce" });
    let t = Date.now();
    await send("slow");
    assert.ok(Date.now() - t >= 400, "the latency is seen");
    await control("DELETE", "/v1/faults/request-latency");
    await control("POST", "/v1/faults/leader-change", { topic: n("f") });
    await send("after-leader-change");
    await control("POST", "/v1/faults/broker-unavailable", { for: "5s" });
    const pending = send("while-down");
    await sleep(1000);
    await control("POST", "/v1/clock/advance", { by: "5s" });
    await pending;
    await p.disconnect();
    await control("POST", "/v1/faults/consumer-lag", { records: 1 });
    const r = await reader(n("lagging"), n("f"));
    await until("4 records", () => r.seen.length >= 4);
    await control("DELETE", "/v1/faults");
    const joins = r.joins.length;
    await control("POST", "/v1/faults/rebalance-storm", { rounds: 2 });
    await until("the storm", async () => (await control("GET", "/v1/faults")).body.active.length === 0, 40000);
    await until("rejoins", () => r.joins.length >= joins + 2, 20000);
    await r.c.disconnect();
    assert.deepEqual(r.seen.map((x) => x.value), ["before", "slow", "after-leader-change", "while-down"]);
    return { joins: r.joins.length };
  },

  async force() {
    const refused = await control("POST", "/v1/state", { topics: [{ name: n("orders"), partitions: 0 }] });
    assert.equal(refused.status, 422);
    assert.equal(refused.body.law, "a new topic has the partitions asked for");
    const forced = await control("POST", "/v1/state", {
      topics: [{ name: n("orders"), partitions: 1 }],
      records: [{ topic: n("orders"), partition: 0, records: [{ value: "r0" }, { value: "r1" }, { value: "r2" }, { value: "r3" }] }],
      committed: [{ group: n("resume"), topic: n("orders"), partition: 0, offset: 2 }],
    });
    assert.equal(forced.status, 200);
    const r = await reader(n("resume"), n("orders"));
    await until("2 records", () => r.seen.length >= 2);
    await sleep(300);
    await r.c.disconnect();
    assert.deepEqual(r.seen.map((x) => x.value), ["r2", "r3"]);
    return { resumedAt: r.seen[0].offset };
  },
};

const run = scenarios[scenario];
if (!run) {
  console.error("unknown scenario " + scenario);
  process.exit(2);
}
const timer = setTimeout(() => {
  console.error("scenario " + scenario + " timed out");
  process.exit(1);
}, 90000);
try {
  const summary = await run();
  console.log(JSON.stringify({ client: "kafkajs", scenario, ok: true, ...summary }));
  clearTimeout(timer);
  process.exit(0);
} catch (e) {
  console.error(e?.stack ?? String(e));
  process.exit(1);
}
