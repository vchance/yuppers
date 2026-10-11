import type { Account } from '@yuppers/api-client'
import type { Language, Wording } from '@yuppers/shared'
import axe from 'axe-core'
import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { vi } from 'vitest'

import { fakeService, type FakeService } from './fake-service'

/*
 * Runs the whole web app in jsdom against the stand-in service, the way a
 * browser would load it at an address, and checks what it shows with axe.
 *
 * jsdom lays nothing out and paints nothing, so axe cannot judge colour
 * contrast, target size, reflow or what is visible on screen here; those are
 * checked by `contrast.test.ts` and by hand. What it can judge, and does,
 * is the document: landmarks, headings, names, roles, states, labels,
 * descriptions and ARIA.
 */

// React is told it is under test, so updates inside `act` are applied at once.
;(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true

let service: FakeService = fakeService(null)

// The app's client is made when its module loads, with `fetch` and `Request`
// as they are then; its addresses are relative to the page, as on the real
// origin. Both are put in place here, before any of the app is imported.
const NodeRequest = globalThis.Request
globalThis.Request = class extends NodeRequest {
  constructor(input: RequestInfo | URL, init?: RequestInit) {
    super(typeof input === 'string' ? new URL(input, window.location.origin) : input, init)
  }
} as typeof Request
globalThis.fetch = ((input: RequestInfo | URL, init?: RequestInit) =>
  service.fetch(input, init)) as typeof fetch

// What jsdom does not do, done as nothing.
window.scrollTo = () => {}
Element.prototype.scrollIntoView = () => {}

let root: Root | null = null

export interface Started {
  service: FakeService
  wording: Wording
}

/**
 * Opens the app at `address`, as `account` or signed out, with a fresh copy
 * of every module so nothing one test did is still there in the next. The
 * wording is English unless `speaking` names another language, or is a whole
 * wording of its own (the pseudo-language test). `prepare` sets the stand-in
 * service up before the app first asks it anything.
 */
export async function start(
  address: string,
  account: Account | null,
  speaking: Language | Wording = 'en',
  prepare?: (service: FakeService) => void,
): Promise<Started> {
  await stop()
  vi.resetModules()
  service = fakeService(account)
  prepare?.(service)
  window.sessionStorage.clear()
  window.localStorage.clear()
  window.history.replaceState(null, '', address)
  document.title = ''
  document.body.innerHTML = '<div id="root"></div>'

  const { App } = await import('../app/App')
  const { loadWording } = await import('../app/wording')
  const language: Language = typeof speaking === 'string' ? speaking : 'en'
  const wording = typeof speaking === 'string' ? await loadWording(speaking) : speaking
  await act(async () => {
    root = createRoot(document.getElementById('root')!)
    root.render(<App initialLanguage={language} initialWording={wording} />)
  })
  return { service, wording }
}

export async function stop(): Promise<void> {
  if (!root) return
  const mounted = root
  root = null
  await act(async () => mounted.unmount())
}

/**
 * How many times longer than usual to wait: a shared or busy machine sets
 * `YUPPERS_TEST_PATIENCE` so that a slow answer is not read as a missing one.
 */
const patience = Math.max(1, Number(process.env.YUPPERS_TEST_PATIENCE ?? '1') || 1)

/** Waits, letting the app answer, until `check` holds. */
export async function until(check: () => boolean, what: string): Promise<void> {
  for (let tries = 0; tries < 100 * patience; tries += 1) {
    if (check()) return
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 10))
    })
  }
  throw new Error(`timed out waiting for ${what}\n${document.body.innerHTML.slice(0, 2000)}`)
}

/** Waits for the page's main heading to say `text`. */
export function heading(text: string): Promise<void> {
  return until(() => document.querySelector('h1')?.textContent === text, `the heading “${text}”`)
}

/** The button whose text is exactly `text`. */
export function button(text: string): HTMLButtonElement {
  const found = [...document.querySelectorAll('button')].find(
    (candidate) => candidate.textContent?.trim() === text,
  )
  if (!found) throw new Error(`no button “${text}”`)
  return found
}

/** The control whose label is exactly `text`. */
export function field(text: string): HTMLInputElement | HTMLTextAreaElement | HTMLSelectElement {
  const label = [...document.querySelectorAll('label')].find(
    (candidate) => candidate.textContent?.trim() === text,
  )
  const control = label?.control
  if (!control) throw new Error(`no field labelled “${text}”`)
  return control as HTMLInputElement
}

/**
 * Names who a first proposal's invitation is for, as the composer expects,
 * once the field is labelled for what the service takes (`label`).
 */
export async function nameInvitee(label: string, address: string): Promise<void> {
  await until(
    () => [...document.querySelectorAll('label')].some((found) => found.textContent?.trim() === label),
    `the field “${label}”`,
  )
  await type(field(label), address)
}

/** Presses a control from the keyboard: it has the focus first, as it would. */
export async function press(element: HTMLElement): Promise<void> {
  await act(async () => {
    element.focus()
    element.click()
  })
}

/** Types into a field the way a person would, so React hears it. */
export async function type(
  control: HTMLInputElement | HTMLTextAreaElement | HTMLSelectElement,
  value: string,
): Promise<void> {
  const prototype = Object.getPrototypeOf(control) as object
  const setter = Object.getOwnPropertyDescriptor(prototype, 'value')?.set
  await act(async () => {
    setter?.call(control, value)
    control.dispatchEvent(
      new Event(control instanceof HTMLSelectElement ? 'change' : 'input', { bubbles: true }),
    )
  })
}

/** Lets pending timers run, such as an announcement being put into its live region. */
export async function settle(ms = 150): Promise<void> {
  await act(async () => {
    await new Promise((resolve) => setTimeout(resolve, ms))
  })
}

/** What the live regions are saying now. */
export function announced(): { polite: string; assertive: string } {
  return {
    polite: document.getElementById('announce-polite')?.textContent ?? '',
    assertive: document.getElementById('announce-assertive')?.textContent ?? '',
  }
}

/**
 * Every axe violation on the page, WCAG 2.0 to 2.2 A and AA and axe's best
 * practices, each as one line naming the rule and the elements.
 */
export async function violations(): Promise<string[]> {
  const results = await axe.run(document, {
    runOnly: {
      type: 'tag',
      values: ['wcag2a', 'wcag2aa', 'wcag21a', 'wcag21aa', 'wcag22aa', 'best-practice'],
    },
    // Needs layout and painting, which jsdom does not do (see above).
    rules: { 'color-contrast': { enabled: false } },
  })
  return results.violations.map(
    (violation) =>
      `${violation.id}: ${violation.help}\n  ${violation.nodes.map((node) => node.html).join('\n  ')}`,
  )
}
