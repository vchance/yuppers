#!/usr/bin/env node
// A load check against a running API and worker (README, "Load check").
//
// Simulates pairs of people taking an exchange from sign-in to closed through
// the HTTP API, the way the clients do, and reports what it saw: latency per
// endpoint, errors, throughput, the API's peak memory, the database
// connections in use, and how fast the worker drained the notifications the
// run queued.
//
// Plain Node, no dependencies. It needs the API started with
// CODE_DELIVERY=log, because it signs each person in with the one-time code
// the API writes to its log, and `psql` for the database figures.
//
//   node scripts/load-check.mjs --pairs 50 --concurrency 10 \
//     --base-url http://127.0.0.1:8100 --api-log /path/to/api.log \
//     --database-url postgres://exchange:exchange@127.0.0.1:5432/yuppers_load
//
// The database URL is only read from: pg_stat_activity and the outbox. Give
// the schema owner's, which can see every connection's state.
//
// Every person is a new account with an email address of its own, and every
// exchange has its own initiator, so no per-account limit is approached:
// one code per email address (codes_per_hour), one exchange per initiator
// (exchanges_per_day), one invitation per exchange (invitations_per_day) and
// at most six changes by one party to one exchange (changes_per_minute).
//
// Code requests are also limited per network address, and every request here
// comes from one machine. So the API under test is started with that limit
// raised out of the way: SIGN_IN_CODE_REQUESTS_PER_ADDRESS_PER_HOUR=1000000.

import { spawn } from "node:child_process";
import { open } from "node:fs/promises";
import { randomUUID } from "node:crypto";
import { parseArgs } from "node:util";

const { values: options } = parseArgs({
  options: {
    pairs: { type: "string", default: "50" },
    concurrency: { type: "string", default: "10" },
    "base-url": { type: "string", default: "http://127.0.0.1:8080" },
    "api-log": { type: "string" },
    "api-pid": { type: "string" },
    "database-url": { type: "string", default: process.env.MIGRATION_DATABASE_URL },
    psql: { type: "string", default: process.env.PSQL ?? "psql" },
    "drain-timeout": { type: "string", default: "300" },
    json: { type: "boolean", default: false },
  },
});

const pairs = Number(options.pairs);
const concurrency = Number(options.concurrency);
const baseUrl = options["base-url"].replace(/\/$/, "");
if (!options["api-log"]) {
  console.error("--api-log is required: the API's log file, where codes are written");
  process.exit(2);
}
if (!(pairs > 0 && concurrency > 0)) {
  console.error("--pairs and --concurrency must be positive numbers");
  process.exit(2);
}

// Must match the API's current consent wording version (backend/src/bin/api.rs).
const CONSENT = { language: "en", version: "draft-1" };
const run = Date.now().toString(36);

// ---- One-time codes from the API's log --------------------------------------

/** Follows the API log from its current end and hands out codes by address. */
class CodeReader {
  constructor(path) {
    this.path = path;
    this.codes = new Map();
    this.waiting = new Map();
    this.rest = "";
  }

  async start() {
    this.file = await open(this.path, "r");
    this.offset = (await this.file.stat()).size;
    this.timer = setInterval(() => this.poll().catch(() => {}), 20);
  }

  async poll() {
    if (this.reading) return;
    this.reading = true;
    try {
      const buffer = Buffer.alloc(1 << 20);
      for (;;) {
        const { bytesRead } = await this.file.read(buffer, 0, buffer.length, this.offset);
        if (bytesRead === 0) break;
        this.offset += bytesRead;
        const lines = (this.rest + buffer.toString("utf8", 0, bytesRead)).split("\n");
        this.rest = lines.pop();
        for (const line of lines) this.take(line);
      }
    } finally {
      this.reading = false;
    }
  }

  take(line) {
    // eslint-disable-next-line no-control-regex
    const plain = line.replace(/\x1b\[[0-9;]*m/g, "");
    const match = /one-time code .*\bto="?([^"\s]+)"? code="?(\d+)"?/.exec(plain);
    if (!match) return;
    const [, to, code] = match;
    const resolve = this.waiting.get(to);
    if (resolve) {
      this.waiting.delete(to);
      resolve(code);
    } else {
      this.codes.set(to, code);
    }
  }

  code(address, timeoutMs = 10_000) {
    const code = this.codes.get(address);
    if (code) {
      this.codes.delete(address);
      return Promise.resolve(code);
    }
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        this.waiting.delete(address);
        reject(new Error(`no code for ${address} in the log`));
      }, timeoutMs);
      this.waiting.set(address, (value) => {
        clearTimeout(timer);
        resolve(value);
      });
    });
  }

  async stop() {
    clearInterval(this.timer);
    await this.file.close();
  }
}

// ---- Measuring requests ------------------------------------------------------

const samples = new Map(); // endpoint -> { times: [], errors: Map(status code -> n) }

function record(endpoint, ms, error) {
  let entry = samples.get(endpoint);
  if (!entry) {
    entry = { times: [], errors: new Map() };
    samples.set(endpoint, entry);
  }
  entry.times.push(ms);
  if (error) entry.errors.set(error, (entry.errors.get(error) ?? 0) + 1);
}

class RequestFailed extends Error {}

async function call(endpoint, method, path, { token, body, idempotent } = {}) {
  const headers = { "x-client-version": "web/0.0.0" };
  if (token) headers.authorization = `Bearer ${token}`;
  if (body !== undefined) headers["content-type"] = "application/json";
  if (idempotent) headers["idempotency-key"] = randomUUID();
  const started = performance.now();
  let response;
  let text;
  try {
    response = await fetch(baseUrl + path, {
      method,
      headers,
      body: body === undefined ? undefined : JSON.stringify(body),
    });
    text = await response.text();
  } catch (error) {
    record(endpoint, performance.now() - started, "network");
    throw new RequestFailed(`${endpoint}: ${error.message}`);
  }
  const ms = performance.now() - started;
  const json = text ? JSON.parse(text) : null;
  if (!response.ok) {
    const label = `${response.status} ${json?.code ?? ""}`.trim();
    record(endpoint, ms, label);
    const hint =
      response.status === 429 && endpoint === "POST /v1/auth/codes"
        ? " (is the API running with SIGN_IN_CODE_REQUESTS_PER_ADDRESS_PER_HOUR raised?)"
        : "";
    throw new RequestFailed(`${endpoint}: ${label}${hint}`);
  }
  record(endpoint, ms);
  return json;
}

// ---- One pair -------------------------------------------------------------------

async function signIn(codes, label) {
  const identifier = `load-${run}-${label}@example.test`;
  await call("POST /v1/auth/codes", "POST", "/v1/auth/codes", { body: { identifier } });
  const code = await codes.code(identifier);
  const session = await call("POST /v1/auth/sessions", "POST", "/v1/auth/sessions", {
    body: { identifier, code, delivery: "TOKEN", language: "en" },
  });
  const token = session.token;
  await call("PATCH /v1/me", "PATCH", "/v1/me", {
    token,
    body: { display_name: `Load ${label}`, adult_confirmed: true },
  });
  return token;
}

const view = (token, id) => call("GET /v1/exchanges/{id}", "GET", `/v1/exchanges/${id}`, { token });

/** Runs a command at the version the person last saw, as a client does. */
async function command(token, id, version, command) {
  try {
    return await call("POST /v1/exchanges/{id}/commands", "POST", `/v1/exchanges/${id}/commands`, {
      token,
      idempotent: true,
      body: { expected_version: version, command },
    });
  } catch (error) {
    error.message = `${command.type} ${command.action ?? ""}: ${error.message}`;
    throw error;
  }
}

function terms(ids, n) {
  const [build, payment, cleanup] = ids;
  return {
    party_a_name: `Load ${n}a`,
    party_b_name: `Load ${n}b`,
    terms: "Build a garden shed, paid on completion, and take the old one away.",
    contributions: [
      {
        id: build,
        from: "A",
        type: "SERVICE",
        description: "Build the shed",
        quantity: null,
        due: { kind: "ON_AGREEMENT" },
        completion_criteria: "Door closes and the roof does not leak",
        required: true,
        amount_minor: null,
      },
      {
        id: payment,
        from: "B",
        type: "MONEY",
        description: "Payment on completion",
        quantity: null,
        due: { kind: "AFTER_CONTRIBUTION", contribution: build },
        completion_criteria: null,
        required: true,
        amount_minor: 40000,
      },
      {
        id: cleanup,
        from: "A",
        type: "TASK",
        description: "Take the old shed away",
        quantity: null,
        due: { kind: "ON_AGREEMENT" },
        completion_criteria: null,
        required: true,
        amount_minor: null,
      },
    ],
  };
}

async function pair(codes, n) {
  const [a, b] = await Promise.all([
    signIn(codes, `${n}a`),
    signIn(codes, `${n}b`),
  ]);

  // The initiator composes: a draft, a saved working copy, then the revision.
  const draft = await call("POST /v1/exchanges", "POST", "/v1/exchanges", {
    token: a,
    body: { timezone: "America/Chicago" },
  });
  const id = draft.id;
  const ids = [randomUUID(), randomUUID(), randomUUID()];
  const proposal = terms(ids, n);
  await call("PUT /v1/exchanges/{id}/draft", "PUT", `/v1/exchanges/${id}/draft`, {
    token: a,
    body: { body: { terms: proposal } },
  });
  const sent = await call("POST /v1/exchanges/{id}/revisions", "POST", `/v1/exchanges/${id}/revisions`, {
    token: a,
    idempotent: true,
    body: {
      expected_version: draft.version,
      terms: proposal,
      consent: CONSENT,
      note: "Here is what we discussed.",
      invitation: { for_anyone: true },
    },
  });
  const revision = sent.exchange.open_revision.id;
  const invitation = { token: sent.invitation_token };

  // The counterparty, signed in, opens the link, claims it and signs.
  await call("POST /v1/invitations/preview", "POST", "/v1/invitations/preview", {
    token: b,
    body: invitation,
  });
  const claimed = await call("POST /v1/invitations/claim", "POST", "/v1/invitations/claim", {
    token: b,
    body: invitation,
  });
  await command(b, id, claimed.version, { type: "ACCEPT", revision, consent: CONSENT });

  // The initiator sees who claimed it and confirms them, which puts it in force.
  let seen = await view(a, id);
  seen = await command(a, id, seen.version, { type: "CONFIRM_COUNTERPARTY" });
  if (seen.state !== "ACTIVE") throw new Error(`pair ${n}: expected ACTIVE, got ${seen.state}`);

  // Two contributions marked delivered and confirmed, one each way.
  const [build, payment] = ids;
  seen = await command(a, id, seen.version, { type: "CONTRIBUTION", contribution: build, action: "CLAIM" });
  seen = await view(b, id);
  seen = await command(b, id, seen.version, { type: "CONTRIBUTION", contribution: build, action: "CONFIRM" });
  seen = await command(b, id, seen.version, { type: "CONTRIBUTION", contribution: payment, action: "CLAIM" });
  seen = await view(a, id);
  seen = await command(a, id, seen.version, { type: "CONTRIBUTION", contribution: payment, action: "CONFIRM" });

  // The initiator asks to end it with the third still outstanding; the
  // counterparty agrees, which closes it.
  seen = await command(a, id, seen.version, { type: "PROPOSE_END" });
  seen = await view(b, id);
  seen = await command(b, id, seen.version, { type: "ACCEPT_END" });
  if (seen.state !== "CLOSED") throw new Error(`pair ${n}: expected CLOSED, got ${seen.state}`);

  // Each reads the exchange, its history and its record, and their list.
  for (const token of [a, b]) {
    await view(token, id);
    await call("GET /v1/exchanges/{id}/history", "GET", `/v1/exchanges/${id}/history`, { token });
    await call("GET /v1/exchanges/{id}/record", "GET", `/v1/exchanges/${id}/record`, { token });
    await call("GET /v1/exchanges", "GET", "/v1/exchanges", { token });
  }
}

// ---- Watching the API process and the database ------------------------------

function capture(command, args) {
  return new Promise((resolve) => {
    const child = spawn(command, args, { stdio: ["ignore", "pipe", "ignore"] });
    let out = "";
    child.stdout.on("data", (chunk) => (out += chunk));
    child.on("error", () => resolve(null));
    child.on("close", (status) => resolve(status === 0 ? out.trim() : null));
  });
}

async function apiPid() {
  if (options["api-pid"]) return options["api-pid"];
  const port = new URL(baseUrl).port || "80";
  const pids = await capture("lsof", ["-t", `-iTCP:${port}`, "-sTCP:LISTEN"]);
  return pids?.split("\n")[0] || null;
}

function sql(query) {
  if (!options["database-url"]) return Promise.resolve(null);
  return capture(options.psql, [options["database-url"], "-XAtF", "|", "-c", query]);
}

// Connections to this database other than the sampler's own, by state, and
// how many are waiting on a lock.
const ACTIVITY = `
  SELECT count(*),
         count(*) FILTER (WHERE state = 'active'),
         count(*) FILTER (WHERE state = 'idle in transaction'),
         count(*) FILTER (WHERE wait_event_type = 'Lock')
  FROM pg_stat_activity
  WHERE datname = current_database() AND pid <> pg_backend_pid()`;
const OUTBOX = "SELECT count(*) FILTER (WHERE completed_at IS NULL), count(*) FROM outbox";

class Watcher {
  constructor(pid) {
    this.pid = pid;
    this.peakRssKb = 0;
    this.connections = { total: 0, active: 0, idleInTransaction: 0, lockWaits: 0 };
    this.lockWaitSamples = 0;
    this.samples = 0;
    this.outboxPeak = 0;
  }

  start() {
    this.memoryTimer = setInterval(async () => {
      if (!this.pid) return;
      const rss = Number(await capture("ps", ["-o", "rss=", "-p", this.pid]));
      if (rss > this.peakRssKb) this.peakRssKb = rss;
    }, 200);
    const sampleDatabase = async () => {
      const [activity, outbox] = await Promise.all([sql(ACTIVITY), sql(OUTBOX)]);
      if (activity) {
        const [total, active, idleTx, locks] = activity.split("|").map(Number);
        const peak = this.connections;
        peak.total = Math.max(peak.total, total);
        peak.active = Math.max(peak.active, active);
        peak.idleInTransaction = Math.max(peak.idleInTransaction, idleTx);
        peak.lockWaits = Math.max(peak.lockWaits, locks);
        this.samples += 1;
        if (locks > 0) this.lockWaitSamples += 1;
      }
      if (outbox) this.outboxPeak = Math.max(this.outboxPeak, Number(outbox.split("|")[0]));
      if (!this.stopped) this.databaseTimer = setTimeout(sampleDatabase, 250);
    };
    sampleDatabase();
  }

  stop() {
    this.stopped = true;
    clearInterval(this.memoryTimer);
    clearTimeout(this.databaseTimer);
  }
}

async function outboxCounts() {
  const row = await sql(OUTBOX);
  if (!row) return null;
  const [pending, total] = row.split("|").map(Number);
  return { pending, total };
}

// ---- The run --------------------------------------------------------------------

function percentile(sorted, p) {
  if (sorted.length === 0) return 0;
  const index = Math.min(sorted.length - 1, Math.ceil((p / 100) * sorted.length) - 1);
  return sorted[Math.max(0, index)];
}

const ms = (value) => value.toFixed(1);

async function main() {
  const codes = new CodeReader(options["api-log"]);
  await codes.start();
  const pid = await apiPid();
  const watcher = new Watcher(pid);
  const outboxBefore = await outboxCounts();
  watcher.start();

  const failures = [];
  let next = 0;
  let completed = 0;
  const started = performance.now();
  await Promise.all(
    Array.from({ length: Math.min(concurrency, pairs) }, async () => {
      while (next < pairs) {
        const n = next++;
        try {
          await pair(codes, n);
          completed += 1;
        } catch (error) {
          failures.push(`pair ${n}: ${error.message}`);
        }
      }
    }),
  );
  const elapsed = (performance.now() - started) / 1000;
  await codes.stop();

  // How long the worker takes to send what the run queued.
  const outboxAfterRun = await outboxCounts();
  const drainStarted = performance.now();
  let drained = null;
  if (outboxAfterRun) {
    const deadline = drainStarted + Number(options["drain-timeout"]) * 1000;
    for (;;) {
      const counts = await outboxCounts();
      if (counts.pending === 0) {
        drained = (performance.now() - drainStarted) / 1000;
        break;
      }
      if (performance.now() > deadline) break;
      await new Promise((resolve) => setTimeout(resolve, 500));
    }
  }
  watcher.stop();
  const outboxFinal = await outboxCounts();

  const endpoints = [...samples.entries()]
    .map(([endpoint, { times, errors }]) => {
      const sorted = [...times].sort((x, y) => x - y);
      return {
        endpoint,
        count: times.length,
        errors: Object.fromEntries(errors),
        p50: percentile(sorted, 50),
        p95: percentile(sorted, 95),
        p99: percentile(sorted, 99),
        max: sorted[sorted.length - 1],
      };
    })
    .sort((x, y) => y.p95 - x.p95);
  const requests = endpoints.reduce((sum, e) => sum + e.count, 0);
  const errors = endpoints.reduce(
    (sum, e) => sum + Object.values(e.errors).reduce((a, b) => a + b, 0),
    0,
  );
  const queued = outboxAfterRun && outboxBefore ? outboxAfterRun.total - outboxBefore.total : null;

  const report = {
    pairs,
    concurrency,
    completed,
    failed: failures.length,
    seconds: elapsed,
    requests,
    errors,
    requests_per_second: requests / elapsed,
    pairs_per_second: completed / elapsed,
    endpoints,
    api_peak_rss_mb: pid ? watcher.peakRssKb / 1024 : null,
    database_connections_peak: watcher.connections,
    lock_wait_samples: `${watcher.lockWaitSamples}/${watcher.samples}`,
    outbox: outboxAfterRun && {
      queued_by_run: queued,
      pending_peak: watcher.outboxPeak,
      pending_when_run_ended: outboxAfterRun.pending,
      seconds_to_drain_after_run: drained,
      pending_at_end: outboxFinal?.pending,
    },
  };

  if (options.json) {
    console.log(JSON.stringify(report, null, 2));
  } else {
    console.log(`${completed}/${pairs} pairs at concurrency ${concurrency} in ${elapsed.toFixed(1)} s`);
    console.log(
      `${requests} requests, ${errors} errors, ${report.requests_per_second.toFixed(0)} requests/s, ` +
        `${report.pairs_per_second.toFixed(2)} pairs/s`,
    );
    console.log("");
    const width = Math.max(...endpoints.map((e) => e.endpoint.length));
    console.log(`${"endpoint".padEnd(width)}  count    p50 ms   p95 ms   p99 ms   max ms  errors`);
    for (const e of endpoints) {
      const errorText = Object.entries(e.errors)
        .map(([label, count]) => `${count}x ${label}`)
        .join(", ");
      console.log(
        `${e.endpoint.padEnd(width)}  ${String(e.count).padStart(5)}  ${ms(e.p50).padStart(8)} ` +
          `${ms(e.p95).padStart(8)} ${ms(e.p99).padStart(8)} ${ms(e.max).padStart(8)}  ${errorText}`,
      );
    }
    console.log("");
    console.log(
      `API peak resident memory: ${report.api_peak_rss_mb === null ? "unknown (no pid)" : `${report.api_peak_rss_mb.toFixed(1)} MB`}`,
    );
    const c = watcher.connections;
    console.log(
      `Database connections, peak: ${c.total} open, ${c.active} active, ${c.idleInTransaction} idle in transaction, ` +
        `${c.lockWaits} waiting on a lock (lock waits seen in ${report.lock_wait_samples} samples)`,
    );
    if (report.outbox) {
      const o = report.outbox;
      console.log(
        `Outbox: ${o.queued_by_run} queued by the run, ${o.pending_peak} pending at peak, ` +
          `${o.pending_when_run_ended} pending when the run ended, ` +
          (o.seconds_to_drain_after_run === null
            ? `${o.pending_at_end} still pending after ${options["drain-timeout"]} s`
            : `drained ${o.seconds_to_drain_after_run.toFixed(1)} s later`),
      );
    }
  }
  for (const failure of failures.slice(0, 20)) console.error(failure);
  if (failures.length > 20) console.error(`... and ${failures.length - 20} more`);
  process.exitCode = failures.length > 0 ? 1 : 0;
}

await main();
