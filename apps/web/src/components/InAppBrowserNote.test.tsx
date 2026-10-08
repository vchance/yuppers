// @vitest-environment jsdom
import { afterEach, describe, expect, test, vi } from 'vitest'

import { button, field, press, start, stop, type, until } from '../test/harness'

/*
 * The note on the sign-in form for a page open in another app's built-in
 * browser: shown there, on both steps, until hidden, and never in a browser
 * proper.
 */

const OUTLOOK_IOS =
  'Mozilla/5.0 (iPhone; CPU iPhone OS 18_6 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Mobile/15E148 Outlook-iOS/749.4.prod.iphone (4.2534.0)'
const SAFARI_IOS =
  'Mozilla/5.0 (iPhone; CPU iPhone OS 18_6 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.6 Mobile/15E148 Safari/604.1'
const FACEBOOK_ANDROID =
  'Mozilla/5.0 (Linux; Android 14; Pixel 8 Build/AP2A.240805.005; wv) AppleWebKit/537.36 (KHTML, like Gecko) Version/4.0 Chrome/127.0.6533.103 Mobile Safari/537.36 [FB_IAB/FB4A;FBAV/478.0.0.43.115;]'

function browsingWith(userAgent: string) {
  vi.spyOn(window.navigator, 'userAgent', 'get').mockReturnValue(userAgent)
}

const note = () => document.querySelector<HTMLElement>('[role="note"]')

afterEach(async () => {
  await stop()
  vi.restoreAllMocks()
})

describe('the in-app browser note', () => {
  test('in Outlook on iOS: says so, opens Safari, and stays on the code step', async () => {
    browsingWith(OUTLOOK_IOS)
    const { wording } = await start('/', null)
    const w = wording.inAppBrowser
    await until(() => note() !== null, 'the note')

    const shown = note()!
    expect(shown.textContent).toContain(w.noteIos.replace('{app}', 'Outlook'))
    // Named by its text, for a screen reader moving by landmarks and notes.
    expect(document.getElementById(shown.getAttribute('aria-labelledby')!)?.textContent).toBe(
      w.noteIos.replace('{app}', 'Outlook'),
    )
    const open = [...shown.querySelectorAll('a')].find((link) => link.textContent === w.openSafari)
    expect(open?.getAttribute('href')).toBe(
      `x-safari-${window.location.protocol.slice(0, -1)}://${window.location.host}/`,
    )
    expect(shown.textContent).toContain(w.menu)
    button(w.copy)
    button(w.hide)

    // The note sits at the top of the form, and the code step has it too.
    await type(field(wording.signIn.identifierLabel), 'ana@example.test')
    await press(button(wording.signIn.sendCode))
    await until(() => field(wording.signIn.codeLabel) !== null, 'the code step')
    expect(note()?.textContent).toContain(w.openSafari)
  })

  test('copies the page’s address, never its query', async () => {
    browsingWith(FACEBOOK_ANDROID)
    const writeText = vi.fn().mockResolvedValue(undefined)
    Object.defineProperty(window.navigator, 'clipboard', {
      value: { writeText },
      configurable: true,
    })
    const { wording } = await start('/?code=123456', null)
    const w = wording.inAppBrowser
    await until(() => note() !== null, 'the note')
    expect(note()!.textContent).toContain(w.noteOther.replace('{app}', 'Facebook'))
    const open = [...note()!.querySelectorAll('a')].find((link) => link.textContent === w.openBrowser)
    expect(open?.getAttribute('href')).toBe(
      `intent://${window.location.host}/#Intent;scheme=${window.location.protocol.slice(0, -1)};end`,
    )

    await press(button(w.copy))
    expect(writeText).toHaveBeenCalledWith(`${window.location.origin}/`)
    await until(() => note()!.textContent!.includes(w.copied), 'copied')
  })

  test('hides when asked, for the rest of the visit, and the focus goes to the form', async () => {
    browsingWith(OUTLOOK_IOS)
    const { wording } = await start('/', null)
    await until(() => note() !== null, 'the note')
    await press(button(wording.inAppBrowser.hide))
    expect(note()).toBeNull()
    expect(document.activeElement).toBe(field(wording.signIn.identifierLabel))

    await type(field(wording.signIn.identifierLabel), 'ana@example.test')
    await press(button(wording.signIn.sendCode))
    await until(() => field(wording.signIn.codeLabel) !== null, 'the code step')
    expect(note()).toBeNull()
  })

  test('is never shown in Safari', async () => {
    browsingWith(SAFARI_IOS)
    const { wording } = await start('/', null)
    await until(() => field(wording.signIn.identifierLabel) !== null, 'the sign-in form')
    expect(note()).toBeNull()
    expect(document.body.textContent).not.toContain(wording.inAppBrowser.hide)
  })
})
