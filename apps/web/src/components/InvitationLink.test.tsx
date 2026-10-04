// @vitest-environment jsdom
import { createI18n, wordingFor } from '@yuppers/shared'
import axe from 'axe-core'
import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, expect, test, vi } from 'vitest'

import { I18nContext } from '../app/context'
import { readQr } from '../test/qr'
import { InvitationLink } from './InvitationLink'

/*
 * Passing an invitation link on (DESIGN.md §8). "Share link" always does
 * something: the device's share sheet where it has one, and otherwise a
 * panel of ways to start a message, a copy and a QR code. Nothing is sent by
 * the service, and the token never goes into an address of ours.
 */

;(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true

const wording = wordingFor('en')
const w = wording.invitationLink
const i18n = createI18n('en', wording, () => {})
const TOKEN = 'Tok3n_with-dashes_0123456789abcdefghij'
const LINK = `${window.location.origin}/en/i#${TOKEN}`
const MESSAGE = `I’ve sent you a yup to review: ${LINK}`

let root: Root | null = null
const copied: string[] = []

beforeEach(() => {
  copied.length = 0
  Object.defineProperty(navigator, 'clipboard', {
    configurable: true,
    value: {
      writeText: async (text: string) => {
        copied.push(text)
      },
    },
  })
})

afterEach(async () => {
  const mounted = root
  root = null
  if (mounted) await act(async () => mounted.unmount())
  // What a test gave the browser is taken away again.
  for (const name of ['share', 'canShare', 'clipboard']) {
    Reflect.deleteProperty(navigator, name)
  }
})

/** A browser with a share sheet of its own, taking whatever `canShare` says. */
function withShareSheet(canShare?: (data: ShareData) => boolean) {
  const shared: ShareData[] = []
  Object.defineProperty(navigator, 'share', {
    configurable: true,
    value: async (data: ShareData) => {
      shared.push(data)
    },
  })
  if (canShare) {
    Object.defineProperty(navigator, 'canShare', { configurable: true, value: canShare })
  }
  return shared
}

async function show(boundTo: string | null = null) {
  document.documentElement.lang = 'en'
  document.title = 'Invitation link'
  document.body.innerHTML = '<main id="root"></main>'
  await act(async () => {
    root = createRoot(document.getElementById('root')!)
    root.render(
      <I18nContext.Provider value={i18n}>
        <h1>Test</h1>
        <InvitationLink token={TOKEN} boundTo={boundTo} />
      </I18nContext.Provider>,
    )
  })
}

function button(text: string): HTMLButtonElement | undefined {
  return [...document.querySelectorAll('button')].find((found) => found.textContent === text)
}

function link(text: string): HTMLAnchorElement | undefined {
  return [...document.querySelectorAll('a')].find((found) => found.textContent?.startsWith(text))
}

async function press(element: HTMLElement) {
  await act(async () => {
    element.focus()
    element.click()
  })
}

/** Waits for the QR code, which is drawn by a module loaded only when wanted. */
async function qrCode(): Promise<SVGSVGElement> {
  for (let tries = 0; tries < 100; tries += 1) {
    const found = document.querySelector<SVGSVGElement>('svg[role="img"]')
    if (found) return found
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 10))
    })
  }
  throw new Error('no QR code')
}

async function violations(): Promise<string[]> {
  const results = await axe.run(document, {
    runOnly: {
      type: 'tag',
      values: ['wcag2a', 'wcag2aa', 'wcag21a', 'wcag21aa', 'wcag22aa', 'best-practice'],
    },
    rules: { 'color-contrast': { enabled: false } },
  })
  return results.violations.map((violation) => `${violation.id}: ${violation.help}`)
}

/** The query of a `mailto:`, `sms:` or web address, decoded. */
function query(address: string): URLSearchParams {
  return new URLSearchParams(address.slice(address.indexOf('?') + 1))
}

test('it says, where the link is, that sending it is the person’s own job', async () => {
  await show()
  expect(document.body.textContent).toContain(
    'Yuppers doesn’t send this for you. Share the link with them, and they can read the proposal after signing in.',
  )
  // Shown once, and how to get another.
  expect(document.body.textContent).toContain(w.shownOnce)
  expect(w.shownOnce).toContain(w.reissue)
})

test('with a share sheet, “Share link” opens it, and copying and the QR code are beside it', async () => {
  const shared = withShareSheet(() => true)
  await show()

  const share = button(w.share)!
  expect(share.className).toBe('primary')
  // It opens something of the system's, not a panel of ours.
  expect(share.hasAttribute('aria-expanded')).toBe(false)
  await press(share)
  expect(shared).toEqual([{ title: wording.linkPreview.title, text: w.shareText, url: LINK }])
  expect(document.querySelector('.panel')).toBeNull()

  await press(button(w.copy)!)
  expect(copied).toEqual([LINK])
  expect(document.body.textContent).toContain(w.copied)

  await press(button(w.shareQr)!)
  expect(readQr(await qrCode())).toBe(LINK)
  expect(button(w.hideQr)!.getAttribute('aria-expanded')).toBe('true')
  expect(await violations()).toEqual([])
})

test('a share sheet that cannot take the link counts as none', async () => {
  const shared = withShareSheet(() => false)
  await show()
  await press(button(w.share)!)
  expect(shared).toEqual([])
  expect(document.querySelector('.panel')).not.toBeNull()
})

test('without a share sheet, “Share link” opens the ways to pass it on, each carrying the link', async () => {
  await show('carla@example.test')
  // No second copy button outside the panel: it is the panel's first choice.
  expect(button(w.copy)).toBeUndefined()
  const share = button(w.share)!
  expect(share.getAttribute('aria-expanded')).toBe('false')
  await press(share)

  expect(share.getAttribute('aria-expanded')).toBe('true')
  const panel = document.querySelector<HTMLElement>('.panel')!
  expect(panel.getAttribute('role')).toBe('group')
  expect(document.activeElement).toBe(panel)
  const choices = [...panel.querySelectorAll('li')].map((item) => item.textContent)
  expect(choices).toEqual([
    w.copy,
    w.shareEmail,
    w.shareSms,
    `${w.shareWhatsApp} ${wording.help.newTab}`,
    w.shareQr,
  ])

  // An email to whom the invitation is for, with the message and the link.
  const email = link(w.shareEmail)!.getAttribute('href')!
  expect(email.startsWith('mailto:carla@example.test?')).toBe(true)
  expect(query(email).get('subject')).toBe(wording.linkPreview.title)
  expect(query(email).get('body')).toBe(MESSAGE)
  const sms = link(w.shareSms)!.getAttribute('href')!
  expect(sms.startsWith('sms:?body=')).toBe(true)
  expect(query(sms).get('body')).toBe(MESSAGE)
  const whatsApp = new URL(link(w.shareWhatsApp)!.href)
  expect(whatsApp.origin + whatsApp.pathname).toBe('https://wa.me/')
  expect(whatsApp.searchParams.get('text')).toBe(MESSAGE)
  expect(link(w.shareWhatsApp)!.target).toBe('_blank')
  expect(link(w.shareWhatsApp)!.rel).toContain('noopener')
  // The token is inside each message, encoded, never the address's own
  // fragment; and none of them is an address of ours.
  for (const anchor of panel.querySelectorAll('a')) {
    const href = anchor.getAttribute('href')!
    expect(href).not.toContain('#')
    expect(href).toContain(`%23${TOKEN}`)
    expect(href.startsWith(window.location.origin)).toBe(false)
  }

  await press(button(w.copy)!)
  expect(copied).toEqual([LINK])
  expect(document.body.textContent).toContain(w.copied)
  expect(await violations()).toEqual([])
})

test('the QR code, drawn on the device, reads back as the link', async () => {
  await show()
  await press(button(w.share)!)
  await press(button(w.shareQr)!)

  const code = await qrCode()
  expect(code.getAttribute('aria-label')).toBe(w.qrLabel)
  expect(readQr(code)).toBe(LINK)
  expect(document.body.textContent).toContain(w.qrHint)
  // Drawn, not fetched: nothing for the Content-Security-Policy to refuse.
  expect(document.querySelector('img')).toBeNull()
  expect(await violations()).toEqual([])

  await press(button(w.hideQr)!)
  expect(document.querySelector('svg')).toBeNull()
})

test('closing the panel gives the focus back to “Share link”', async () => {
  await show()
  const share = button(w.share)!
  await press(share)
  await press(button(w.closeShare)!)
  expect(document.querySelector('.panel')).toBeNull()
  expect(document.activeElement).toBe(share)
  expect(share.getAttribute('aria-expanded')).toBe('false')
})

test('a copy that fails says how to copy it by hand', async () => {
  Object.defineProperty(navigator, 'clipboard', {
    configurable: true,
    value: { writeText: vi.fn().mockRejectedValue(new Error('denied')) },
  })
  await show()
  await press(button(w.share)!)
  await press(button(w.copy)!)
  expect(document.body.textContent).toContain(w.copyFailed)
})
