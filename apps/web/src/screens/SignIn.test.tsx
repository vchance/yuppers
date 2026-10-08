// @vitest-environment jsdom
import { SMS_CODE_CONSENT_VERSION } from '@yuppers/shared'
import { act } from 'react'
import { afterEach, describe, expect, test, vi } from 'vitest'

import { CODE_SENDER } from '../test/fake-service'
import { button, field, press, settle, start, stop, type, until } from '../test/harness'

/*
 * Signing in asks for what the service can send a code to (`GET /v1/meta`,
 * `sign_in_channels`): an email address only where it cannot send text
 * messages, and a phone number typed anyway is stopped before it is sent.
 */

afterEach(stop)

function hasLabel(text: string): boolean {
  return [...document.querySelectorAll('label')].some((label) => label.textContent?.trim() === text)
}

describe('signing in where the service has no text messages', () => {
  test('asks for an email address, and only that', async () => {
    const { wording, service } = await start('/', null, 'en', (fake) => {
      fake.phone = false
    })
    const w = wording.signIn
    await until(() => service.sent.some((request) => request.call === 'GET /v1/meta'), 'meta')
    await settle()

    expect(document.body.textContent).toContain(w.introEmail)
    expect(document.body.textContent).not.toContain(w.intro)
    expect(hasLabel(w.identifierLabel)).toBe(false)
    const email = field(w.emailLabel) as HTMLInputElement
    expect(email.type).toBe('email')
    expect(email.inputMode).toBe('email')
    // Nothing about phone numbers or their countries.
    expect(document.body.textContent).not.toMatch(/\+1/)
  })

  test('a phone number typed anyway is stopped here, with what to do instead', async () => {
    const { wording, service } = await start('/', null, 'en', (fake) => {
      fake.phone = false
    })
    const w = wording.signIn
    await until(() => service.sent.some((request) => request.call === 'GET /v1/meta'), 'meta')
    await settle()

    const email = field(w.emailLabel)
    await type(email, '+1 555 123 4567')
    // No box: no code would go by text.
    expect(document.querySelector('input[type="checkbox"]')).toBeNull()
    await press(button(w.sendCode))
    await until(() => document.body.textContent!.includes(w.emailOnly), 'the email-only message')
    expect(document.body.textContent).not.toContain(wording.errors.SERVICE_UNAVAILABLE)
    expect(email.getAttribute('aria-invalid')).toBe('true')
    const note = [...document.querySelectorAll('.notice-error')].find(
      (found) => found.textContent === w.emailOnly,
    )
    expect(email.getAttribute('aria-describedby')).toContain(note!.id)
    expect(service.sent.some((request) => request.call === 'POST /v1/auth/codes')).toBe(false)

    // Typing again clears it; an email address goes through.
    await type(email, 'ben@example.test')
    expect(document.body.textContent).not.toContain(w.emailOnly)
    await press(button(w.sendCode))
    await until(() => hasLabel(w.codeLabel), 'the code field')
    expect(service.sent.at(-1)).toEqual({
      call: 'POST /v1/auth/codes',
      body: { identifier: 'ben@example.test' },
    })
    // Going back offers another email address, not a phone number.
    button(w.changeEmail)
    expect(document.body.textContent).not.toContain(w.changeIdentifier)
  })
})

describe('signing in where the service sends text messages', () => {
  test('a phone number is asked for too, US numbers only, and sent in E.164', async () => {
    const { wording, service } = await start('/', null)
    const w = wording.signIn
    await until(() => hasLabel(w.identifierLabel), 'the email or phone field')

    expect(document.body.textContent).toContain(w.intro)
    // Only +1 is texted: no country code is asked for.
    expect(document.body.textContent).toContain(w.identifierHintUs)
    expect(document.body.textContent).not.toContain('country code')
    const identifier = field(w.identifierLabel) as HTMLInputElement
    // An email field would refuse the brackets and the spaces of a phone number.
    expect(identifier.type).toBe('text')

    // Typed as people in the US write it, without +1.
    await act(async () => identifier.focus())
    await type(identifier, '555-234-5678')
    // Ticking the box leaves the field, which writes the number the American
    // way; the box stays ticked, for it is the same number.
    await press(field(wording.smsCode.signIn))
    expect(identifier.value).toBe('(555) 234-5678')
    expect((field(wording.smsCode.signIn) as HTMLInputElement).checked).toBe(true)
    await press(button(w.sendCode))
    await until(() => hasLabel(w.codeLabel), 'the code field')
    expect(service.sent.at(-1)).toEqual({
      call: 'POST /v1/auth/codes',
      body: {
        identifier: '+15552345678',
        sms_consent: { version: SMS_CODE_CONSENT_VERSION, language: 'en' },
      },
    })
    expect(document.body.textContent).toContain(
      w.codeSent.replace('{identifier}', '(555) 234-5678'),
    )
    button(w.changeIdentifier)
  })

  test('a number of another country, or not a number, is stopped here in the service’s words', async () => {
    const { wording, service } = await start('/', null)
    const w = wording.signIn
    await until(() => hasLabel(w.identifierLabel), 'the email or phone field')
    const identifier = field(w.identifierLabel) as HTMLInputElement

    for (const [typed, code] of [
      ['+44 20 7946 0958', 'PHONE_COUNTRY_NOT_SERVED'],
      ['555 134 5678', 'INVALID_IDENTIFIER'],
      ['555 2345', 'INVALID_IDENTIFIER'],
    ] as const) {
      await type(identifier, typed)
      await press(field(wording.smsCode.signIn))
      await press(button(w.sendCode))
      await until(
        () => document.body.textContent!.includes(wording.errors[code]),
        `${code} for ${typed}`,
      )
      expect(identifier.value).toBe(typed)
    }
    expect(service.sent.some((request) => request.call === 'POST /v1/auth/codes')).toBe(false)
  })
})

describe('the box beside a phone number', () => {
  const box = () => document.querySelector<HTMLInputElement>('input[type="checkbox"]')

  test('appears unticked once what is typed reads as a phone number, never for an email address', async () => {
    const { wording } = await start('/', null)
    const w = wording.signIn
    await until(() => hasLabel(w.identifierLabel), 'the email or phone field')
    const identifier = field(w.identifierLabel)
    expect(box()).toBeNull()

    await type(identifier, 'ben@example.test')
    expect(box()).toBeNull()
    expect(button(w.sendCode).disabled).toBe(false)
    expect(button(w.sendCode).getAttribute('aria-describedby')).toBeNull()

    await type(identifier, '+1')
    expect(box()).not.toBeNull()
    expect(box()!.checked).toBe(false)
    await type(identifier, '+1 201 555 0123')
    expect(box()!.checked).toBe(false)
    // The short line it replaces is gone; the link to the policy on texts stays.
    expect(document.body.textContent).not.toContain(
      'Message and data rates may apply. Reply STOP to opt out.',
    )
    expect(document.body.textContent).toContain(wording.privacy.smsLink)

    await type(identifier, 'ben@example.test')
    expect(box()).toBeNull()
  })

  test('is labelled with the words the terms quote, its two addresses links that open a new tab', async () => {
    const { wording } = await start('/', null)
    await until(() => hasLabel(wording.signIn.identifierLabel), 'the email or phone field')
    await type(field(wording.signIn.identifierLabel), '+12015550123')

    expect(field(wording.smsCode.signIn)).toBe(box())
    expect(wording.smsCode.signIn).toBe(
      'Text me a one-time sign-in code from yuppers.app at this number. One message per request. Msg & data rates may apply. Reply HELP for help or STOP to opt out. Terms: https://yuppers.app/terms. Privacy Policy: https://yuppers.app/privacy.',
    )
    const links = [...box()!.closest('label')!.querySelectorAll('a')]
    expect(links.map((link) => [link.textContent, link.getAttribute('href'), link.target])).toEqual([
      ['https://yuppers.app/terms', 'https://yuppers.app/terms', '_blank'],
      ['https://yuppers.app/privacy', 'https://yuppers.app/privacy', '_blank'],
    ])
  })

  test('holds back “Send code”, saying why, until it is ticked', async () => {
    const { wording, service } = await start('/', null)
    const w = wording.signIn
    await until(() => hasLabel(w.identifierLabel), 'the email or phone field')
    const identifier = field(w.identifierLabel)
    await type(identifier, '+12015550123')

    const send = button(w.sendCode)
    expect(send.disabled).toBe(true)
    const why = document.getElementById(send.getAttribute('aria-describedby')!)
    expect(why?.textContent).toBe(wording.smsCode.tickToSend)
    // Submitted anyway, from the keyboard: nothing is asked for.
    await act(async () => {
      send.form!.requestSubmit()
    })
    expect(service.sent.some((request) => request.call === 'POST /v1/auth/codes')).toBe(false)

    await press(box()!)
    expect(box()!.checked).toBe(true)
    expect(send.disabled).toBe(false)
    expect(send.getAttribute('aria-describedby')).toBeNull()
    expect(document.body.textContent).not.toContain(wording.smsCode.tickToSend)

    // Another number unticks it, and typing the first again does not tick it back.
    await type(identifier, '+12015550124')
    expect(box()!.checked).toBe(false)
    await type(identifier, '+12015550123')
    expect(box()!.checked).toBe(false)
    expect(button(w.sendCode).disabled).toBe(true)
  })

  test('is unticked again on coming back to the number', async () => {
    const { wording } = await start('/', null)
    const w = wording.signIn
    await until(() => hasLabel(w.identifierLabel), 'the email or phone field')
    await type(field(w.identifierLabel), '+12015550123')
    await press(box()!)
    await press(button(w.sendCode))
    await until(() => hasLabel(w.codeLabel), 'the code field')

    await press(button(w.changeIdentifier))
    await until(() => box() !== null, 'the box again')
    expect(box()!.checked).toBe(false)
    expect(button(w.sendCode).disabled).toBe(true)
  })

  test('is in Spanish in Spanish, HELP, STOP and the addresses as they are', async () => {
    const { wording, service } = await start('/', null, 'es')
    await until(() => hasLabel(wording.signIn.identifierLabel), 'the email or phone field')
    await type(field(wording.signIn.identifierLabel), '+12015550123')
    const label = box()!.closest('label')!
    expect(label.textContent).toBe(wording.smsCode.signIn)
    expect(label.textContent).toMatch(/HELP.*STOP.*https:\/\/yuppers\.app\/terms.*https:\/\/yuppers\.app\/privacy/)
    await press(box()!)
    await press(button(wording.signIn.sendCode))
    await until(() => hasLabel(wording.signIn.codeLabel), 'the code field')
    expect(service.sent.at(-1)?.body).toEqual({
      identifier: '+12015550123',
      sms_consent: { version: SMS_CODE_CONSENT_VERSION, language: 'es' },
    })
  })
})

describe('waiting for the code', () => {
  afterEach(() => {
    vi.useRealTimers()
  })

  test('says to look in the spam folder too, for an email from the address the service names', async () => {
    const { wording } = await start('/', null)
    const w = wording.signIn
    await until(() => hasLabel(w.identifierLabel), 'the email or phone field')
    await type(field(w.identifierLabel), 'ben@example.test')
    await press(button(w.sendCode))
    await until(() => hasLabel(w.codeLabel), 'the code field')

    expect(document.body.textContent).toContain(
      `Check your inbox, and your spam folder, for an email from ${CODE_SENDER}.`,
    )
  })

  test('without an address from the service, the folders alone; for a phone number, nothing', async () => {
    const { wording } = await start('/', null, 'en', (fake) => {
      fake.codeSender = null
    })
    const w = wording.signIn
    await until(() => hasLabel(w.identifierLabel), 'the email or phone field')
    await type(field(w.identifierLabel), 'ben@example.test')
    await press(button(w.sendCode))
    await until(() => hasLabel(w.codeLabel), 'the code field')
    expect(document.body.textContent).toContain(w.checkInboxAnySender)
    expect(document.body.textContent).not.toContain('an email from')

    await press(button(w.changeIdentifier))
    await type(field(w.identifierLabel), '+15552345678')
    await press(field(wording.smsCode.signIn))
    await press(button(w.sendCode))
    await until(() => hasLabel(w.codeLabel), 'the code field')
    expect(document.body.textContent).not.toContain(w.checkInboxAnySender)
  })

  test('offers another code only after 30 seconds, then again 30 seconds after that one', async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true })
    const { wording, service } = await start('/', null)
    const w = wording.signIn
    await until(() => hasLabel(w.identifierLabel), 'the email or phone field')
    await type(field(w.identifierLabel), 'ben@example.test')
    await press(button(w.sendCode))
    await until(() => hasLabel(w.codeLabel), 'the code field')
    const resend = () =>
      [...document.querySelectorAll('button')].find((found) => found.textContent === w.resend)

    expect(resend()).toBeUndefined()
    expect(document.body.textContent).toContain(w.resendSoon)
    await act(async () => {
      vi.advanceTimersByTime(29_000)
    })
    expect(resend()).toBeUndefined()
    await act(async () => {
      vi.advanceTimersByTime(1_000)
    })
    expect(resend()).toBeDefined()
    expect(document.body.textContent).not.toContain(w.resendSoon)

    await press(resend()!)
    await until(() => document.body.textContent!.includes(w.resent), 'the new code')
    expect(service.sent.filter((request) => request.call === 'POST /v1/auth/codes')).toHaveLength(2)
    expect(resend()).toBeUndefined()
    // The button pressed has gone; the keyboard is back on the code.
    expect(document.activeElement).toBe(field(w.codeLabel))
    await act(async () => {
      vi.advanceTimersByTime(30_000)
    })
    expect(resend()).toBeDefined()
  })

  test('another code over the limit says so, as before', async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true })
    const { wording, service } = await start('/', null)
    const w = wording.signIn
    await until(() => hasLabel(w.identifierLabel), 'the email or phone field')
    await type(field(w.identifierLabel), 'ben@example.test')
    await press(button(w.sendCode))
    await until(() => hasLabel(w.codeLabel), 'the code field')
    await act(async () => {
      vi.advanceTimersByTime(30_000)
    })
    service.refuseCodes = 'TOO_MANY_REQUESTS'
    await press(button(w.resend))
    await until(
      () => document.body.textContent!.includes(wording.errors.TOO_MANY_REQUESTS),
      'the limit',
    )
    expect(document.body.textContent).not.toContain(w.resent)
  })
})
