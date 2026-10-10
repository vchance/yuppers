import { mkdirSync, readFileSync } from 'node:fs'
import { dirname, resolve } from 'node:path'

/*
 * Where everything the mobile end-to-end tests need lives. Each can be
 * overridden from the environment; the defaults suit a checkout with the
 * backend built by `cargo build` and the app exported for the web by
 * `npm run e2e:export -w @yuppers/mobile`.
 *
 * Three processes on this machine: the API, the harness proxy in front of it
 * (scripts/harness-proxy.mjs, which lets a page from another origin call it),
 * and a static server for the app's web export. Their ports differ from the
 * web suite's (8090, 8091), so both suites can run at once.
 * `support/export-web.mjs` bakes the proxy's address into the export and
 * reads the same variables with the same defaults.
 */

// The mobile app's package is CommonJS, so the tests are compiled as CommonJS.
export const repoRoot = resolve(__dirname, '../../../..')
export const mobileRoot = resolve(repoRoot, 'apps/mobile')

/** The version of the Terms and the Privacy policy the service knows, which a sign-in names (`backend/src/terms.rs`). */
export function termsVersion(): string {
  const source = readFileSync(resolve(repoRoot, 'backend/src/terms.rs'), 'utf8')
  return /TERMS_VERSION: &str = "([^"]+)"/.exec(source)![1]
}

const host = '127.0.0.1'

/** The API, as the tests reach it directly when acting for someone outside the app. */
export const apiPort = Number(process.env.E2E_MOBILE_API_PORT ?? 8103)
export const apiURL = `http://${host}:${apiPort}`

/** The harness proxy, which is what the app in the browser calls. */
export const proxyPort = Number(process.env.E2E_MOBILE_PROXY_PORT ?? 8203)
export const proxyURL = `http://${host}:${proxyPort}`

/** The app's screens, served from the web export. */
export const webPort = Number(process.env.E2E_MOBILE_WEB_PORT ?? 5203)
export const webURL = `http://${host}:${webPort}`

export const apiBinary = resolve(
  process.env.E2E_API_BIN ?? resolve(repoRoot, 'backend/target/debug/api'),
)
export const exportDir = resolve(
  process.env.E2E_MOBILE_EXPORT_DIR ?? resolve(mobileRoot, 'e2e/.output/web'),
)

/**
 * The API's log. With CODE_DELIVERY=log the service writes each one-time
 * code there, and the tests read it back instead of receiving email.
 */
export const apiLog = resolve(
  process.env.E2E_MOBILE_API_LOG ?? resolve(mobileRoot, 'e2e/.output/api.log'),
)
mkdirSync(dirname(apiLog), { recursive: true })

/** The settings the API the tests start runs with, beside the database from `.env`. */
export function apiEnvironment(): Record<string, string> {
  return {
    BIND_ADDR: `${host}:${apiPort}`,
    // No cookie session is used: the app signs in for a bearer token, and the
    // proxy strips the browser's origin. This only has to be set.
    WEB_ORIGIN: webURL,
    CODE_DELIVERY: 'log',
    NOTIFICATION_DELIVERY: 'log',
    RUST_LOG: 'info',
    // Every browser and API call here comes from 127.0.0.1, so the per-address
    // sign-in limit is raised out of the way (README, "Signing in").
    SIGN_IN_CODE_REQUESTS_PER_ADDRESS_PER_HOUR: '1000000',
    // Plain lines, so the codes can be read back.
    NO_COLOR: '1',
  }
}
