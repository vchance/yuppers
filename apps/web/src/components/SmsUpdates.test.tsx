// @vitest-environment jsdom
import { SMS_CODE_CONSENT_VERSION, SMS_CONSENT_VERSION } from '@yuppers/shared'
import { afterEach, expect, test } from 'vitest'

import { ACTIVE, DRAFT, ENDED, GOOD_CODE, ana } from '../test/fake-service'
import {
  announced,
  button,
  field,
  press,
  settle,
  start,
  stop,
  type,
  until,
  violations,
} from '../test/harness'

/*
 * "Text updates" on an agreement (DESIGN.md §12): adding a US number with a
 * code by text, the box beside the consent wording as the terms quote it,
 * what Save says, a number that replied STOP, and where the control is not
 * shown at all.
 */

afterEach(stop)

/** The control's section, once it is on the page. */
function section(): HTMLElement | null {
  const heading = [...document.querySelectorAll('h2')].find(
    (candidate) => candidate.textContent === 'Text updates',
  )
  return heading?.closest('section') ?? null
}

async function shown() {
  await until(() => section() !== null, 'the text updates control')
  return section()!
}

const lastSent = (service: { sent: { call: string; body: unknown }[] }, call: string) =>
  [...service.sent].reverse().find((sent) => sent.call === call)?.body

test('a party with no number adds one with a code, then ticks the box and saves', async () => {
  const { service, wording } = await start(`/exchanges/${ACTIVE}`, ana)
  const w = wording.smsUpdates
  const control = await shown()
  expect(control.textContent).toContain(w.intro)
  expect(control.textContent).toContain(w.addPhoneIntro)
  // Before a number gets a code, a box, unticked, beside the words the
  // terms quote for checking a number; "Text me a code" waits for it.
  const codeBox = field(wording.smsCode.verifyNumber) as HTMLInputElement
  expect(codeBox.type).toBe('checkbox')
  expect(codeBox.checked).toBe(false)
  expect(wording.smsCode.verifyNumber).toBe(
    'Text me a one-time verification code from yuppers.app at this number. One message per request. Msg & data rates may apply. Reply HELP for help or STOP to opt out. Terms: https://yuppers.app/terms. Privacy Policy: https://yuppers.app/privacy.',
  )
  const send = button(w.sendCode)
  expect(send.disabled).toBe(true)
  expect(document.getElementById(send.getAttribute('aria-describedby')!)?.textContent).toBe(
    wording.smsCode.tickToSend,
  )
  expect(await violations()).toEqual([])

  // Not a US number: said, and nothing sent.
  await type(field(w.phoneLabel), '+44 7700 900123')
  await press(field(wording.smsCode.verifyNumber))
  await press(button(w.sendCode))
  expect(control.textContent).toContain(w.phoneInvalid)
  expect(lastSent(service, 'POST /v1/auth/codes')).toBeUndefined()

  // Another number: the box is unticked again.
  await type(field(w.phoneLabel), '(555) 234-5678')
  expect((field(wording.smsCode.verifyNumber) as HTMLInputElement).checked).toBe(false)
  expect(button(w.sendCode).disabled).toBe(true)
  await press(field(wording.smsCode.verifyNumber))
  await press(button(w.sendCode))
  expect(lastSent(service, 'POST /v1/auth/codes')).toEqual({
    identifier: '+15552345678',
    sms_consent: { version: SMS_CODE_CONSENT_VERSION, language: 'en' },
  })
  await until(() => control.textContent!.includes('We texted a code to +1 •••-•••-5678.'), 'the code step')
  expect(document.activeElement).toBe(field(w.codeLabel))
  expect(await violations()).toEqual([])

  await type(field(w.codeLabel), GOOD_CODE)
  await press(button(w.addPhone))
  expect(lastSent(service, 'POST /v1/me/identifiers')).toEqual({
    identifier: '+15552345678',
    code: GOOD_CODE,
  })
  await until(() => control.querySelector('input[type="checkbox"]') !== null, 'the box')
  expect(control.textContent).toContain('+1 •••-•••-5678 is now on your account.')

  // The box's label is the consent wording, word for word, with its two
  // addresses as links that read as the addresses.
  const box = control.querySelector<HTMLInputElement>('input[type="checkbox"]')!
  expect(box.checked).toBe(false)
  const label = box.closest('label')!
  expect(label.textContent).toBe(w.consent)
  expect(
    [...label.querySelectorAll('a')].map((link) => [
      link.getAttribute('href'),
      link.textContent,
    ]),
  ).toEqual([
    ['https://yuppers.app/terms', 'https://yuppers.app/terms'],
    ['https://yuppers.app/privacy', 'https://yuppers.app/privacy'],
  ])
  expect(await violations()).toEqual([])

  // Ticked and saved: on, with the consent's version and language.
  await press(box)
  await press(button(w.save))
  expect(lastSent(service, `PUT /v1/exchanges/${ACTIVE}/sms-updates`)).toEqual({
    on: true,
    consent: { version: SMS_CONSENT_VERSION, language: 'en' },
  })
  const confirmation =
    'Text updates are on for this agreement. You’ll get one text per status change at +1 •••-•••-5678. Reply STOP to opt out.'
  await until(() => control.textContent!.includes(confirmation), 'the confirmation')
  await settle()
  expect(announced().polite).toBe(confirmation)

  // Unticked and saved: off.
  await press(control.querySelector<HTMLInputElement>('input[type="checkbox"]')!)
  await press(button(w.save))
  expect(lastSent(service, `PUT /v1/exchanges/${ACTIVE}/sms-updates`)).toEqual({ on: false })
  await until(() => control.textContent!.includes(w.off), 'off')
  expect(service.textUpdates.has(ACTIVE)).toBe(false)
})

test('a party whose number is on the account sees the box, ticked while updates are on', async () => {
  const { wording } = await start(`/exchanges/${ACTIVE}`, { ...ana, phone: '+15552345678' }, 'en', (service) => {
    service.textUpdates.add(ACTIVE)
  })
  const control = await shown()
  await until(() => control.querySelector('input[type="checkbox"]') !== null, 'the box')
  expect(control.querySelector<HTMLInputElement>('input[type="checkbox"]')!.checked).toBe(true)
  expect(control.textContent).toContain(
    'Text updates are on for this agreement. You’ll get one text per status change at +1 •••-•••-5678.',
  )
  // How it works, on the page the carriers review.
  const learn = [...control.querySelectorAll('a')].find((link) =>
    link.textContent!.startsWith(wording.smsUpdates.howItWorks),
  )!
  expect(learn.getAttribute('href')).toBe('/sms-opt-in')
})

test('a number that replied STOP is told how to get texts again, and offered no box', async () => {
  const { wording } = await start(`/exchanges/${ACTIVE}`, { ...ana, phone: '+15552345678' }, 'en', (service) => {
    service.optedOut = true
  })
  const control = await shown()
  await until(() => control.textContent!.includes('replied STOP'), 'the opted-out notice')
  expect(control.textContent).toContain(
    wording.smsUpdates.optedOut.replace('{phone}', '+1 •••-•••-5678'),
  )
  expect(control.querySelector('input[type="checkbox"]')).toBeNull()
  expect(await violations()).toEqual([])
})

test('in Spanish the box shows the Spanish wording the terms quote', async () => {
  const { wording } = await start(`/exchanges/${ACTIVE}`, { ...ana, phone: '+15552345678', language: 'es' }, 'es')
  await until(
    () => [...document.querySelectorAll('h2')].some((h) => h.textContent === wording.smsUpdates.heading),
    'the control',
  )
  const box = document.querySelector<HTMLInputElement>('.sms-updates input[type="checkbox"]')
  await until(() => document.querySelector('.sms-updates input[type="checkbox"]') !== null, 'the box')
  const label = (box ?? document.querySelector<HTMLInputElement>('.sms-updates input[type="checkbox"]')!).closest('label')!
  expect(label.textContent).toBe(
    'Recibir actualizaciones por mensaje de texto de yuppers.app sobre este acuerdo, un mensaje por cada cambio de estado. La frecuencia de los mensajes varía; no hay un máximo fijo. Pueden aplicarse tarifas por mensajes y datos. Responde HELP para obtener ayuda o STOP para cancelar. Términos: https://yuppers.app/terms. Política de privacidad: https://yuppers.app/privacy.',
  )
  const learn = document.querySelector('.sms-updates .learn-more a')!
  expect(learn.getAttribute('href')).toBe('/es/sms-opt-in')
})

test('nothing is shown where texts are not sent, on a draft, or on a closed agreement', async () => {
  await start(`/exchanges/${ACTIVE}`, ana, 'en', (service) => {
    service.texting = false
  })
  await until(() => document.querySelector('h1') !== null, 'the page')
  await settle()
  expect(section()).toBeNull()

  for (const id of [DRAFT, ENDED]) {
    const { service } = await start(`/exchanges/${id}`, ana)
    await until(() => document.querySelector('h1') !== null, 'the page')
    await settle()
    expect(section(), id).toBeNull()
    expect(service.sent.some((sent) => sent.call.startsWith('PUT') && sent.call.endsWith('/sms-updates'))).toBe(false)
  }
})
