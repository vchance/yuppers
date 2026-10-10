import { mkdirSync, readFileSync } from 'node:fs'
import { dirname, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

/*
 * Where everything the end-to-end tests need lives. Each can be overridden
 * from the environment; the defaults suit a checkout with the backend built
 * by `cargo build` and the web app by `npm run build:web`.
 */

const here = dirname(fileURLToPath(import.meta.url))

export const repoRoot = resolve(here, '../../../..')
export const webRoot = resolve(repoRoot, 'apps/web')

/** The version of the Terms and the Privacy policy the service knows, which a sign-in names (`backend/src/terms.rs`). */
export function termsVersion(): string {
  const source = readFileSync(resolve(repoRoot, 'backend/src/terms.rs'), 'utf8')
  return /TERMS_VERSION: &str = "([^"]+)"/.exec(source)![1]
}

/** The API the tests drive. It serves the built web app too, so the browser talks to one origin. */
export const port = Number(process.env.E2E_PORT ?? 8090)
/** Always 127.0.0.1: the session cookie belongs to the exact host, and WEB_ORIGIN must match it. */
export const baseURL = `http://127.0.0.1:${port}`

/** A second API, started by the test that needs a service refusing this build as too old. */
export const outdatedPort = Number(process.env.E2E_OUTDATED_PORT ?? 8091)

export const apiBinary = resolve(
  process.env.E2E_API_BIN ?? resolve(repoRoot, 'backend/target/debug/api'),
)
export const webDir = resolve(process.env.E2E_WEB_DIR ?? resolve(webRoot, 'dist'))

/** The worker, started by the test that waits for an agreement update to be texted. */
export const workerBinary = resolve(
  process.env.E2E_WORKER_BIN ?? resolve(repoRoot, 'backend/target/debug/worker'),
)

/**
 * The API's log. With CODE_DELIVERY=log the service writes each one-time
 * code there, and the tests read it back instead of receiving email.
 */
export const apiLog = resolve(process.env.E2E_API_LOG ?? resolve(webRoot, 'e2e/.output/api.log'))
mkdirSync(dirname(apiLog), { recursive: true })

/**
 * The sign-in limit per network address, high enough that a whole run, and
 * the runs before it within the hour, never reach it from the one address
 * they all share. CI starts its API with the same value.
 */
export const signInLimits = {
  SIGN_IN_CODE_REQUESTS_PER_ADDRESS_PER_HOUR: '1000000',
}

/**
 * Text messages, written to the log rather than sent: agreement updates
 * (`SMS_DELIVERY=log`), so that they are offered and their texts can be
 * read back, and one-time codes for phone numbers (`SMS_CODE_DELIVERY=log`,
 * where Twilio Verify would make and send them), counted as texts and read
 * back by number (`codes.ts`); with the hourly caps out of the way, since
 * every run's texts count. CI starts its API with the same values.
 */
export const textMessages = {
  SMS_DELIVERY: 'log',
  SMS_CODE_DELIVERY: 'log',
  SMS_MAX_PER_HOUR: '1000000',
  SMS_MAX_PER_PREFIX_PER_HOUR: '1000000',
}

/** The settings every API process the tests start runs with, beside the database from `.env`. */
export function apiEnvironment(listenPort: number): Record<string, string> {
  return {
    BIND_ADDR: `127.0.0.1:${listenPort}`,
    WEB_ORIGIN: `http://127.0.0.1:${listenPort}`,
    WEB_DIR: webDir,
    CODE_DELIVERY: 'log',
    NOTIFICATION_DELIVERY: 'log',
    RUST_LOG: 'info',
    // Every browser here connects from 127.0.0.1, and sign-in is limited per
    // requester's address, so those limits are raised out of the way. The
    // per-identifier limits stay as they are: every person has an address
    // of their own.
    ...signInLimits,
    ...textMessages,
    // Plain lines, so the codes can be read back.
    NO_COLOR: '1',
  }
}
