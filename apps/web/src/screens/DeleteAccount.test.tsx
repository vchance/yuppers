// @vitest-environment jsdom
import { SMS_CODE_CONSENT_VERSION } from '@yuppers/shared'
import { afterEach, expect, test } from 'vitest'

import { ana } from '../test/fake-service'
import { button, field, press, start, stop, until, violations } from '../test/harness'

/*
 * Deleting the account with a code by text: the box beside the number,
 * shown only when the code is to go to the phone, unticked, labelled with
 * the words the terms quote, and holding back the code until it is ticked.
 */

afterEach(stop)

const PHONE = '+15552345678'

const box = () => document.querySelector<HTMLInputElement>('input[type="checkbox"]')
const radio = (channel: string) =>
  document.querySelector<HTMLInputElement>(`input[type="radio"][value="${channel}"]`)!

test('a code to the phone waits for the box beside it; one by email needs none', async () => {
  const { wording, service } = await start('/account', { ...ana, phone: PHONE })
  const w = wording.deletion
  await until(() => document.body.textContent!.includes(w.open), 'deleting the account')
  await press(button(w.open))
  await until(() => document.body.textContent!.includes(w.nothingOpen), 'what deleting would do')

  // By email, the first offered: no box.
  expect(radio('EMAIL').checked).toBe(true)
  expect(box()).toBeNull()
  expect(button(w.sendCode).disabled).toBe(false)

  // To the phone: the box, unticked, with the words for deleting.
  await press(radio('PHONE'))
  expect(box()).not.toBeNull()
  expect(box()!.checked).toBe(false)
  expect(field(wording.smsCode.deleteAccount)).toBe(box())
  expect(wording.smsCode.deleteAccount).toBe(
    'Text me a one-time account-deletion code from yuppers.app at this number. One message per request. Msg & data rates may apply. Reply HELP for help or STOP to opt out. Terms: https://yuppers.app/terms. Privacy Policy: https://yuppers.app/privacy.',
  )
  const send = button(w.sendCode)
  expect(send.disabled).toBe(true)
  expect(document.getElementById(send.getAttribute('aria-describedby')!)?.textContent).toBe(
    wording.smsCode.tickToSend,
  )
  expect(await violations()).toEqual([])

  // Ticked, then back to email and to the phone again: unticked, never remembered.
  await press(box()!)
  expect(button(w.sendCode).disabled).toBe(false)
  await press(radio('EMAIL'))
  expect(box()).toBeNull()
  await press(radio('PHONE'))
  expect(box()!.checked).toBe(false)

  await press(box()!)
  await press(button(w.sendCode))
  await until(() => document.body.textContent!.includes(w.codeSent.replace('{identifier}', '(555) 234-5678').split('.')[0]), 'the code step')
  const asked = service.sent.filter((sent) => sent.call === 'POST /v1/me/deletion/codes')
  expect(asked.map((sent) => sent.body)).toEqual([
    { channel: 'PHONE', sms_consent: { version: SMS_CODE_CONSENT_VERSION, language: 'en' } },
  ])
})
