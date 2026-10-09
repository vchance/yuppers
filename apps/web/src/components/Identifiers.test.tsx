// @vitest-environment jsdom
import { formatMessage } from '@yuppers/shared'
import { afterEach, expect, test } from 'vitest'

import { COMBINE_OFFER, GOOD_CODE, GOOD_PROOF, INVITATION, ana } from '../test/fake-service'
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

test('changing the email takes a code to one of the account’s own first, there or to the other', async () => {
  const { service, wording } = await start('/account', { ...ana, phone: '+18565488780' })
  const w = wording.identifiers
  await seen(w.heading)
  await press(button(w.change))
  await seen(formatMessage(w.proveIntro, { identifier: 'ana@example.test' }, 'en'))
  // The person may no longer have the address: the code can go to the number.
  await press(button(formatMessage(w.proveOther, { identifier: '(856) 548-8780' }, 'en')))
  await seen(formatMessage(w.proveIntro, { identifier: '(856) 548-8780' }, 'en'))
  const sendTo = button(formatMessage(w.proveSend, { identifier: '(856) 548-8780' }, 'en'))
  expect(sendTo.disabled).toBe(true)
  await press(field(wording.smsCode.verifyNumber))
  await press(sendTo)
  await until(() => text().includes(wording.signIn.codeLabel), 'the proving code step')
  expect(sent(service, 'POST /v1/auth/codes').at(-1)).toMatchObject({ identifier: '+18565488780' })
  expect(await violations()).toEqual([])
  await type(field(wording.signIn.codeLabel), GOOD_CODE)
  await press(button(w.proveConfirm))
  await until(() => text().includes(w.newEmailLabel), 'the new address step')
  expect(sent(service, 'POST /v1/me/identifiers/proof')).toEqual([
    { channel: 'PHONE', code: GOOD_CODE },
  ])
  expect(text()).toContain(formatMessage(w.changeEmailTold, { identifier: 'ana@example.test' }, 'en'))
  await type(field(w.newEmailLabel), 'new@example.test')
  await press(button(w.sendCode))
  await until(() => text().includes(wording.signIn.codeLabel), 'the code step')
  await type(field(wording.signIn.codeLabel), GOOD_CODE)
  await press(button(w.confirm))
  await seen(formatMessage(w.added, { identifier: 'new@example.test' }, 'en'))
  expect(sent(service, 'POST /v1/me/identifiers').at(-1)).toEqual({
    identifier: 'new@example.test',
    code: GOOD_CODE,
    proof: GOOD_PROOF,
  })
})

test('an address on another account offers to combine, says what moves, and combines on a deliberate tap', async () => {
  const { service, wording } = await start('/account', ana, 'en', (service) => {
    service.otherAccountAt = 'old@example.test'
  })
  const w = wording.combine
  await seen(wording.identifiers.heading)
  await press(button(wording.identifiers.change))
  await press(
    button(formatMessage(wording.identifiers.proveSend, { identifier: 'ana@example.test' }, 'en')),
  )
  await until(() => text().includes(wording.signIn.codeLabel), 'the proving code step')
  await type(field(wording.signIn.codeLabel), GOOD_CODE)
  await press(button(wording.identifiers.proveConfirm))
  await until(() => text().includes(wording.identifiers.newEmailLabel), 'the new address step')
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

test('an invitation sent to another address says only its kind, and opens once the address typed is added', async () => {
  const { service, wording } = await start(`/en/i#${INVITATION}`, ana, 'en', (service) => {
    service.sentTo = { kind: 'PHONE', replaces: false }
    service.sentToAddress = '+18565488780'
  })
  const w = wording.invitation
  await seen(w.sentToPhone)
  // Nothing of the number is shown.
  expect(text()).not.toContain('8780')
  // No button that would claim it as it is.
  expect(text()).not.toContain(formatMessage(w.respondAs, { name: ana.display_name }, 'en'))
  await type(field(wording.identifiers.newPhoneLabel), '(856) 548-1111')
  expect(button(wording.identifiers.sendCode).disabled).toBe(true)
  await press(field(wording.smsCode.verifyNumber))
  await press(button(wording.identifiers.sendCode))
  await seen(wording.errors.NOT_INVITED_ADDRESS)
  await type(field(wording.identifiers.newPhoneLabel), '(856) 548-8780')
  // The box is ticked for the number beside it: a new number, a new tick.
  await press(field(wording.smsCode.verifyNumber))
  await press(button(wording.identifiers.sendCode))
  await until(() => text().includes(wording.signIn.codeLabel), 'the code step')
  expect(sent(service, 'POST /v1/invitations/address/codes').at(-1)).toMatchObject({
    token: INVITATION,
    identifier: '+18565488780',
  })
  expect(await violations()).toEqual([])
  await type(field(wording.signIn.codeLabel), GOOD_CODE)
  await press(button(w.addAndOpen))
  await until(() => window.location.pathname.startsWith('/exchanges/'), 'the yup opened')
  expect(sent(service, 'POST /v1/invitations/address')[0]).toEqual({
    token: INVITATION,
    identifier: '+18565488780',
    code: GOOD_CODE,
    replace: false,
  })
})

test('an invitation to another account’s address of a kind the account has takes a proof, offers to combine, then opens', async () => {
  const { service, wording } = await start(`/en/i#${INVITATION}`, ana, 'en', (service) => {
    service.sentTo = { kind: 'EMAIL', replaces: true }
    service.sentToAddress = 'ana@old.example.test'
    service.otherAccountAt = 'ana@old.example.test'
  })
  const w = wording.invitation
  await seen(w.sentToEmail)
  expect(text()).toContain(w.sentToReplacesEmail)
  expect(text()).toContain(w.signInInstead)
  // First a code to the account's own address.
  await press(
    button(formatMessage(wording.identifiers.proveSend, { identifier: 'ana@example.test' }, 'en')),
  )
  await until(() => text().includes(wording.signIn.codeLabel), 'the proving code step')
  await type(field(wording.signIn.codeLabel), GOOD_CODE)
  await press(button(wording.identifiers.proveConfirm))
  await until(() => text().includes(wording.identifiers.newEmailLabel), 'the address step')
  await type(field(wording.identifiers.newEmailLabel), 'ana@old.example.test')
  await press(button(wording.identifiers.sendCode))
  await until(() => text().includes(wording.signIn.codeLabel), 'the code step')
  await type(field(wording.signIn.codeLabel), GOOD_CODE)
  await press(button(w.replaceAndOpen))
  await seen(wording.combine.headingEmail)
  expect(sent(service, 'POST /v1/invitations/address')[0]).toEqual({
    token: INVITATION,
    identifier: 'ana@old.example.test',
    code: GOOD_CODE,
    replace: true,
    proof: GOOD_PROOF,
  })
  await press(button(wording.combine.confirm))
  await until(() => window.location.pathname.startsWith('/exchanges/'), 'the yup opened')
  expect(service.sent.map((each) => each.call)).toEqual(
    expect.arrayContaining(['POST /v1/me/combine', 'POST /v1/invitations/claim']),
  )
})

test('a notice with no email to tell is shown once on the list, until dismissed', async () => {
  const { service, wording } = await start('/', {
    ...ana,
    email: null,
    phone: '+18565488780',
    notice: { kind: 'ACCOUNTS_COMBINED', at: '2026-10-22T09:00:00Z' },
  })
  const w = wording.combine
  await seen(w.noticeDismiss)
  expect(text()).toContain('Two of your Yuppers accounts were combined into this one on')
  expect(await violations()).toEqual([])
  await press(button(w.noticeDismiss))
  await until(() => !text().includes(w.noticeDismiss), 'the notice dismissed')
  expect(sent(service, 'PATCH /v1/me')).toEqual([{ dismiss_notice: true }])
})

test('a phone number changed is told in the app, not by text', async () => {
  const { wording } = await start('/', {
    ...ana,
    phone: '+18565488780',
    notice: { kind: 'PHONE_CHANGED', at: '2026-10-22T09:00:00Z' },
  })
  await seen(wording.combine.noticeDismiss)
  expect(text()).toContain('The phone number on your Yuppers account was changed on')
})
