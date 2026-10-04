// @vitest-environment jsdom
import { afterEach, describe, expect, test } from 'vitest'

import { ana, DRAFT, SENT_INVITATION } from '../test/fake-service'
import { button, field, heading, press, settle, start, stop, type, until } from '../test/harness'

/*
 * Who a first proposal's invitation is for is asked on the composer's first
 * page, beside the other person's name, where it is thought of; checked
 * gently with everything else before the signing step; and named once more
 * on that step, as part of what is about to be sent.
 */

afterEach(stop)

function labels(): string[] {
  return [...document.querySelectorAll('label')].map((label) => label.textContent?.trim() ?? '')
}

describe('who the invitation is for', () => {
  test('is asked beside their name, for an email address where codes go by email only', async () => {
    const { wording } = await start(`/exchanges/${DRAFT}`, ana, 'en', (fake) => {
      fake.phone = false
    })
    const w = wording.invitationLink
    await heading(wording.composer.titleFirst)
    await until(() => labels().includes(w.forLabelEmail), 'the email-only label')

    // In the parties' group, straight after the other person's name.
    const bound = field(w.forLabelEmail) as HTMLInputElement
    const group = bound.closest('fieldset')!
    expect(group.querySelector('legend')?.textContent).toBe(wording.composer.partiesLegend)
    const inGroup = [...group.querySelectorAll('input')]
    const otherName = field(wording.composer.otherName) as HTMLInputElement
    expect(inGroup.indexOf(bound)).toBe(inGroup.indexOf(otherName) + 1)
    expect(bound.type).toBe('email')
    expect(document.body.textContent).toContain(w.forHint)
    expect(labels()).not.toContain(w.forLabel)
  })

  test('something plainly wrong is flagged before signing, and the keyboard taken to it', async () => {
    const { wording, service } = await start(`/exchanges/${DRAFT}`, ana, 'en', (fake) => {
      fake.phone = false
    })
    const w = wording.invitationLink
    await heading(wording.composer.titleFirst)
    await until(() => labels().includes(w.forLabelEmail), 'the field')

    const bound = field(w.forLabelEmail)
    await type(bound, 'carla@example')
    await press(button(wording.composer.review))
    await settle()
    expect(document.querySelector('h1')?.textContent).toBe(wording.composer.titleFirst)
    expect(bound.getAttribute('aria-invalid')).toBe('true')
    const described = bound.getAttribute('aria-describedby')!.split(' ')
    expect(document.getElementById(described.at(-1)!)?.textContent).toBe(w.forInvalidEmail)
    expect(document.activeElement).toBe(bound)

    // A phone number where nobody could sign in with one.
    await type(bound, '+1 202 555 0142')
    expect(document.body.textContent).toContain(w.forEmailOnly)

    // Put right, the signing step names who it is for, and the field is not there.
    await type(bound, 'carla@example.test')
    expect(bound.getAttribute('aria-invalid')).toBeNull()
    await press(button(wording.composer.review))
    await heading(wording.composer.signTitle)
    expect(document.body.textContent).toContain(
      'Only someone who signs in with carla@example.test will be able to use the link.',
    )
    expect(labels()).not.toContain(w.forLabelEmail)

    // Signed and sent, it binds the invitation, and the link it gives
    // addresses an email to them.
    await press(document.querySelector<HTMLInputElement>('.consent input[type=checkbox]')!)
    await press(button(wording.composer.signAndSend))
    await until(() => document.body.textContent!.includes(w.intro), 'the invitation link')
    const sent = service.sent.find(
      (request) => request.call === `POST /v1/exchanges/${DRAFT}/revisions`,
    )
    expect(sent?.body).toMatchObject({ invitation: { bound_to: 'carla@example.test' } })
    expect((field(w.linkLabel) as HTMLInputElement).value).toBe(
      `${window.location.origin}/en/i#${SENT_INVITATION}`,
    )
    await press(button(w.share))
    const email = [...document.querySelectorAll('a')].find(
      (link) => link.textContent === w.shareEmail,
    )
    expect(email?.getAttribute('href')?.startsWith('mailto:carla@example.test?subject=')).toBe(true)
  })

  test('takes a phone number where codes are texted, of a country they are texted to', async () => {
    const { wording } = await start(`/exchanges/${DRAFT}`, ana)
    const w = wording.invitationLink
    await heading(wording.composer.titleFirst)
    await until(() => labels().includes(w.forLabel), 'the email or phone label')

    const bound = field(w.forLabel) as HTMLInputElement
    expect(bound.type).toBe('text')
    await type(bound, '+44 20 7946 0958')
    await press(button(wording.composer.review))
    await settle()
    expect(document.body.textContent).toContain(
      'We can only send sign-in codes to phone numbers starting +1, so they couldn’t use the link with this number.',
    )
    await type(bound, 'carla')
    expect(document.body.textContent).toContain(w.forInvalid)

    await type(bound, '+1 202 555 0142')
    await press(button(wording.composer.review))
    await heading(wording.composer.signTitle)
  })

  test('left empty, it names nobody and nothing is said about it', async () => {
    const { wording } = await start(`/exchanges/${DRAFT}`, ana)
    await heading(wording.composer.titleFirst)
    await press(button(wording.composer.review))
    await heading(wording.composer.signTitle)
    expect(document.body.textContent).not.toContain('will be able to use the link')
  })
})
