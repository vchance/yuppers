// @vitest-environment jsdom
import { formatMessage } from '@yuppers/shared'
import { afterEach, expect, test } from 'vitest'

import { COMBINE_OFFER, GOOD_CODE, INVITATION, ana } from '../test/fake-service'
import {
  button,
  field,

  press,
  start,
  stop,
  type,
  until,
  violations,
} from '../test/harness'

/*
 * The account's email address and phone number (README, "Combining
 * accounts"): adding, changing and removing each, Remove refused for the
 * only one with why, the offer to combine another account that a code
 * proved, and an invitation sent to another address.
 */

afterEach(stop)

const text = () => document.body.textContent ?? ''
const seen = (words: string) => until(() => text().includes(words), words)
const sent = (service: { sent: { call: string; body: unknown }[] }, call: string) =>
  service.sent.filter((each) => each.call === call).map((each) => each.body)

test('both are shown with their buttons, and Remove waits while there is only one', async () => {
  const { wording } = await start('/account', ana)
  const w = wording.identifiers
  await seen(w.heading)
  expect(text()).toContain('ana@example.test')
  expect(text()).toContain(w.none)
  const remove = button(w.remove)
  expect(remove.disabled).toBe(true)
  expect(document.getElementById(remove.getAttribute('aria-describedby')!)?.textContent).toBe(
    w.onlyEmail,
  )
  expect(button(w.add)).toBeTruthy()
  expect(button(w.change)).toBeTruthy()
  expect(await violations()).toEqual([])
})

test('a phone number is added with a code by text, once the box is ticked, then the email removed', async () => {
  const { service, wording } = await start('/account', ana)
  const w = wording.identifiers
  await seen(w.heading)
  await press(button(w.add))
  await type(field(w.newPhoneLabel), '(856) 548-8780')
  const send = button(w.sendCode)
  expect(send.disabled).toBe(true)
  await press(field(wording.smsCode.verifyNumber))
  await press(button(w.sendCode))
  await until(() => text().includes(formatMessage(w.codeSent, { identifier: '(856) 548-8780' }, 'en')), 'the code step')
  expect(sent(service, 'POST /v1/auth/codes').at(-1)).toMatchObject({ identifier: '+18565488780' })
  await type(field(wording.signIn.codeLabel), GOOD_CODE)
  await press(button(w.confirm))
  await until(() => text().includes('(856) 548-8780') && !text().includes(w.none), 'the number added')
  expect(text()).toContain(formatMessage(w.added, { identifier: '(856) 548-8780' }, 'en'))

  // Now the email can go, proved by a code to the number that stays.
  const removes = [...document.querySelectorAll('button')].filter(
    (candidate) => candidate.textContent === w.remove,
  )
  expect(removes.every((candidate) => !candidate.disabled)).toBe(true)
  await press(removes[0])
  await seen(w.removeEmailTitle)
  expect(text()).toContain(formatMessage(w.removeIntro, { staying: '(856) 548-8780' }, 'en'))
  const sendTo = formatMessage(w.sendRemovalCode, { staying: '(856) 548-8780' }, 'en')
  expect(button(sendTo).disabled).toBe(true)
  await press(field(wording.smsCode.verifyNumber))
  await press(button(sendTo))
  await until(() => text().includes(wording.signIn.codeLabel), 'the code step')
  await type(field(wording.signIn.codeLabel), '000000')
  await press(button(w.removeConfirm))
  await until(() => text().includes(wording.errors.INVALID_CODE), 'the wrong code refused')
  await type(field(wording.signIn.codeLabel), GOOD_CODE)
  await press(button(w.removeConfirm))
  await until(() => !text().includes('ana@example.test'), 'the email removed')
  expect(service.sent.some((each) => each.call === 'DELETE /v1/me/identifiers/email')).toBe(true)
})

test('an address on another account offers to combine, says what moves, and combines on a deliberate tap', async () => {
  const { service, wording } = await start('/account', ana, 'en', (service) => {
    service.otherAccountAt = 'old@example.test'
  })
  const w = wording.combine
  await seen(wording.identifiers.heading)
  await press(button(wording.identifiers.change))
  await type(field(wording.identifiers.newEmailLabel), 'old@example.test')
  await press(button(wording.identifiers.sendCode))
  await until(() => text().includes(wording.signIn.codeLabel), 'the code step')
  await type(field(wording.signIn.codeLabel), GOOD_CODE)
  await press(button(wording.identifiers.confirm))
  await seen(w.headingEmail)
  for (const line of [
    'Name: Ana R.',
    'Email: a•••@old.example.test',
    '3 yups: 2 in force, 1 waiting for a signature, 0 drafts, 0 closed',
    w.movesYups,
    'Its email address, a•••@old.example.test, is added to this account.',
    w.paymentMove,
    w.ends,
    w.cannotUndo,
  ]) {
    expect(text()).toContain(line)
  }
  expect(await violations()).toEqual([])
  // Nothing is combined until the button.
  expect(service.sent.some((each) => each.call === 'POST /v1/me/combine')).toBe(false)
  await press(button(w.confirm))
  await until(() => text().includes(w.done), 'combined')
  expect(sent(service, 'POST /v1/me/combine')).toEqual([{ token: COMBINE_OFFER.token }])
  expect(text()).toContain('ana@old.example.test')
})

test('an invitation sent to another address asks to add it, masked, and opens once added', async () => {
  const { service, wording } = await start(`/en/i#${INVITATION}`, ana, 'en', (service) => {
    service.sentTo = { kind: 'PHONE', masked: '(•••) •••-8780', replaces: false }
  })
  const w = wording.invitation
  const said = formatMessage(w.sentTo, { identifier: '(•••) •••-8780' }, 'en')
  await seen(said)
  // No button that would claim it as it is.
  expect(text()).not.toContain(formatMessage(w.respondAs, { name: ana.display_name }, 'en'))
  const send = button(formatMessage(w.sendAddressCode, { identifier: '(•••) •••-8780' }, 'en'))
  expect(send.disabled).toBe(true)
  await press(field(wording.smsCode.verifyNumber))
  await press(send)
  await until(() => text().includes(wording.signIn.codeLabel), 'the code step')
  expect(sent(service, 'POST /v1/invitations/address/codes')[0]).toMatchObject({
    token: INVITATION,
  })
  expect(await violations()).toEqual([])
  await type(field(wording.signIn.codeLabel), GOOD_CODE)
  await press(button(w.addAndOpen))
  await until(() => window.location.pathname.startsWith('/exchanges/'), 'the yup opened')
  expect(sent(service, 'POST /v1/invitations/address')[0]).toEqual({
    token: INVITATION,
    code: GOOD_CODE,
    replace: false,
  })
})

test('an invitation sent to another account’s address offers to combine, then opens', async () => {
  const { service, wording } = await start(`/en/i#${INVITATION}`, ana, 'en', (service) => {
    service.sentTo = { kind: 'EMAIL', masked: 'a•••@old.example.test', replaces: true }
    service.otherAccountAt = 'ana@old.example.test'
  })
  const w = wording.invitation
  await seen(formatMessage(w.sentTo, { identifier: 'a•••@old.example.test' }, 'en'))
  expect(text()).toContain(w.sentToReplacesEmail)
  expect(text()).toContain(formatMessage(w.signInInstead, { identifier: 'a•••@old.example.test' }, 'en'))
  await press(button(formatMessage(w.sendAddressCode, { identifier: 'a•••@old.example.test' }, 'en')))
  await until(() => text().includes(wording.signIn.codeLabel), 'the code step')
  await type(field(wording.signIn.codeLabel), GOOD_CODE)
  await press(button(w.replaceAndOpen))
  await seen(wording.combine.headingEmail)
  await press(button(wording.combine.confirm))
  await until(() => window.location.pathname.startsWith('/exchanges/'), 'the yup opened')
  expect(service.sent.map((each) => each.call)).toEqual(
    expect.arrayContaining(['POST /v1/me/combine', 'POST /v1/invitations/claim']),
  )
})
