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
 * Passing an invitation link on (DESIGN.md §8). Nothing is sent by the
 * service: the ways to send it are real buttons and links, led by the one
 * that reaches the person the link was made for, and opening any of them is
 * reported (`onShared`). The token never goes into an address of ours.
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
let shared = 0

beforeEach(() => {
  copied.length = 0
  shared = 0
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
        <InvitationLink
          token={TOKEN}
          boundTo={boundTo}
          onShared={() => {
            shared += 1
          }}
        />
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

/** The buttons and links of the ways to send, in order, as read. */
function ways(): string[] {
  return [...document.querySelectorAll('.share-actions > *')].map(
    (found) => found.textContent?.trim() ?? '',
  )
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

test('made for a phone number, it leads with a text message to that number', async () => {
  await show('(202) 555-0142')
  expect(ways()).toEqual([w.sendText, `${w.sendWhatsApp} ${wording.help.newTab}`, w.copy, w.shareQr])

  const text = link(w.sendText)!
  expect(text.className).toBe('button primary')
  const sms = text.getAttribute('href')!
  expect(sms.startsWith('sms:+12025550142?body=')).toBe(true)
  expect(query(sms).get('body')).toBe(MESSAGE)
  // Nothing else is the primary action.
  expect(document.querySelectorAll('.primary')).toHaveLength(1)

  const whatsApp = new URL(link(w.sendWhatsApp)!.href)
  expect(whatsApp.origin + whatsApp.pathname).toBe('https://wa.me/')
  expect(whatsApp.searchParams.get('text')).toBe(MESSAGE)
  expect(link(w.sendWhatsApp)!.target).toBe('_blank')
  expect(link(w.sendWhatsApp)!.rel).toContain('noopener')

  // Opening any of them is reported.
  await press(text)
  expect(shared).toBe(1)
  await press(button(w.copy)!)
  expect(shared).toBe(2)
  expect(copied).toEqual([LINK])
  expect(document.body.textContent).toContain(w.copied)
  expect(await violations()).toEqual([])
})

test('made for an email address, it leads with an email to that address', async () => {
  await show('carla@example.test')
  expect(ways()).toEqual([w.sendEmail, w.copy, w.shareQr])

  const mail = link(w.sendEmail)!
  expect(mail.className).toBe('button primary')
  const email = mail.getAttribute('href')!
  expect(email.startsWith('mailto:carla@example.test?')).toBe(true)
  expect(query(email).get('subject')).toBe(wording.linkPreview.title)
  expect(query(email).get('body')).toBe(MESSAGE)
  await press(mail)
  expect(shared).toBe(1)
  expect(await violations()).toEqual([])
})

test('for anyone, without a share sheet, it leads with copying, and the messages go to nobody in particular', async () => {
  await show()
  expect(ways()).toEqual([
    w.copy,
    w.shareSms,
    `${w.shareWhatsApp} ${wording.help.newTab}`,
    w.shareEmail,
    w.shareQr,
  ])
  expect(button(w.copy)!.className).toBe('primary')
  expect(button(w.share)).toBeUndefined()

  const sms = link(w.shareSms)!.getAttribute('href')!
  expect(sms.startsWith('sms:?body=')).toBe(true)
  expect(query(sms).get('body')).toBe(MESSAGE)
  const email = link(w.shareEmail)!.getAttribute('href')!
  expect(email.startsWith('mailto:?subject=')).toBe(true)
  // The token is inside each message, encoded, never the address's own
  // fragment; and none of them is an address of ours.
  for (const anchor of document.querySelectorAll('a')) {
    const href = anchor.getAttribute('href')!
    expect(href).not.toContain('#')
    expect(href).toContain(`%23${TOKEN}`)
    expect(href.startsWith(window.location.origin)).toBe(false)
  }

  await press(button(w.copy)!)
  expect(copied).toEqual([LINK])
  expect(shared).toBe(1)
  expect(await violations()).toEqual([])
})

test('for anyone, with a share sheet, it leads with the sheet', async () => {
  const sheet = withShareSheet(() => true)
  await show()
  expect(ways().slice(0, 2)).toEqual([w.share, w.copy])
  const share = button(w.share)!
  expect(share.className).toBe('primary')
  expect(button(w.copy)!.className).toBe('')
  await press(share)
  expect(sheet).toEqual([{ title: wording.linkPreview.title, text: w.shareText, url: LINK }])
  expect(shared).toBe(1)
  expect(await violations()).toEqual([])
})

test('a share sheet that cannot take the link counts as none', async () => {
  withShareSheet(() => false)
  await show()
  expect(button(w.share)).toBeUndefined()
  expect(button(w.copy)!.className).toBe('primary')
})

test('the QR code, drawn on the device, reads back as the link, and showing it counts as sharing', async () => {
  await show()
  const toggle = button(w.shareQr)!
  expect(toggle.getAttribute('aria-expanded')).toBe('false')
  await press(toggle)
  expect(shared).toBe(1)

  const code = await qrCode()
  expect(code.getAttribute('aria-label')).toBe(w.qrLabel)
  expect(readQr(code)).toBe(LINK)
  expect(document.body.textContent).toContain(w.qrHint)
  // Drawn, not fetched: nothing for the Content-Security-Policy to refuse.
  expect(document.querySelector('img')).toBeNull()
  expect(button(w.hideQr)!.getAttribute('aria-expanded')).toBe('true')
  expect(await violations()).toEqual([])

  // Hiding it is not sharing it.
  await press(button(w.hideQr)!)
  expect(document.querySelector('svg')).toBeNull()
  expect(shared).toBe(1)
})

test('a copy that fails says how to copy it by hand', async () => {
  Object.defineProperty(navigator, 'clipboard', {
    configurable: true,
    value: { writeText: vi.fn().mockRejectedValue(new Error('denied')) },
  })
  await show()
  await press(button(w.copy)!)
  expect(document.body.textContent).toContain(w.copyFailed)
})
