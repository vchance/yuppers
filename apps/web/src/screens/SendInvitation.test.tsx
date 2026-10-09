// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, test } from 'vitest'

import { ana, DRAFT, SENT_INVITATION, WAITING } from '../test/fake-service'
import {
  announced,
  button,
  field,
  heading,
  nameInvitee,
  press,
  settle,
  start,
  stop,
  until,
  violations,
} from '../test/harness'

/*
 * Sending the invitation is a step of its own after signing (DESIGN.md §8):
 * Yuppers never sends it, so the person who signed is asked to, plainly,
 * with the way that reaches the person named first, and can only say "I’ll
 * send it later" to go past it. Whatever they open is recorded, and the
 * exchange's page and the list say where the link stands until someone joins.
 */

afterEach(stop)

/** What was copied to the clipboard, which jsdom does not have. */
const copied: string[] = []
/** What a message would have been opened in another app: jsdom cannot leave the page. */
function keepOnPage(event: MouseEvent) {
  const link = (event.target as Element).closest('a[href^="sms:"], a[href^="mailto:"]')
  if (link) event.preventDefault()
}
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
  document.addEventListener('click', keepOnPage)
})
afterEach(() => {
  Reflect.deleteProperty(navigator, 'clipboard')
  document.removeEventListener('click', keepOnPage)
})

function anchor(text: string): HTMLAnchorElement {
  const found = [...document.querySelectorAll('a')].find(
    (candidate) => candidate.textContent?.trim().startsWith(text),
  )
  if (!found) throw new Error(`no link “${text}”`)
  return found
}

/** Signs and sends the draft's first proposal, named for `invitee`, and lands on the step. */
async function signAndSend(invitee: string | null) {
  const started = await start(`/exchanges/${DRAFT}`, ana)
  const { wording } = started
  await heading(wording.composer.titleFirst)
  if (invitee === null) {
    await until(
      () =>
        [...document.querySelectorAll('button')].some(
          (found) => found.textContent === wording.invitationLink.forAnyone,
        ),
      'the way to a link for anyone',
    )
    await press(button(wording.invitationLink.forAnyone))
  } else {
    await nameInvitee(wording.invitationLink.forLabel, invitee)
  }
  await press(button(wording.composer.review))
  await heading(wording.composer.signTitle)
  await press(document.querySelector<HTMLInputElement>('.consent input[type=checkbox]')!)
  await press(button(wording.composer.signAndSend))
  await heading('Send it to Ben Ortiz')
  return started
}

function sharedCalls(service: Awaited<ReturnType<typeof start>>['service']): number {
  return service.sent.filter(
    (request) => request.call === `POST /v1/exchanges/${DRAFT}/invitation/shared`,
  ).length
}

describe('the step that sends the link', () => {
  test('named by phone: a text message to the number first, the rule said plainly', async () => {
    const { wording, service } = await signAndSend('2025550142')
    const w = wording.invitationLink
    // A page of its own, at the exchange's address, with the keyboard on its heading.
    expect(window.location.pathname).toBe(`/exchanges/${DRAFT}`)
    expect(document.activeElement).toBe(document.querySelector('h1'))
    expect(document.body.textContent).toContain(
      'Yuppers doesn’t send it for you. Ben Ortiz gets nothing until you share this link.',
    )
    expect(document.body.textContent).toContain(
      'Only someone who signs in with (202) 555-0142 will be able to use the link.',
    )
    expect((field(w.linkLabel) as HTMLInputElement).value).toBe(
      `${window.location.origin}/en/i#${SENT_INVITATION}`,
    )

    const text = anchor(w.sendText)
    expect(text.className).toBe('button primary')
    const href = text.getAttribute('href')!
    expect(href.startsWith('sms:+12025550142?body=')).toBe(true)
    expect(decodeURIComponent(href.slice(href.indexOf('=') + 1))).toBe(
      `I’ve sent you a yup to review: ${window.location.origin}/en/i#${SENT_INVITATION}`,
    )
    expect(anchor(w.sendWhatsApp).className).toBe('button')
    expect(button(w.copy).className).toBe('')
    // Not sending is a choice of its own, plainly secondary, and said to leave a reminder.
    expect(button(w.later).className).toBe('link')
    expect(document.body.textContent).toContain(
      'You can send it from the yup’s page. It reminds you until Ben Ortiz joins.',
    )
    expect(sharedCalls(service)).toBe(0)
    expect(await violations()).toEqual([])

    // Opening the text message is recorded, once is enough, and the way on
    // appears in place of "later" and takes the keyboard.
    await press(text)
    await settle()
    expect(sharedCalls(service)).toBe(1)
    expect(document.body.textContent).toContain(
      'Once it’s sent, Ben Ortiz can read the yup after signing in. You’ll see on the yup’s page when they join.',
    )
    expect(announced().polite).toContain('Once it’s sent, Ben Ortiz can read the yup')
    expect(document.activeElement?.textContent).toBe(w.sendDone)
    expect(() => button(w.later)).toThrow()
    expect(await violations()).toEqual([])

    // Done: the exchange, whose card says the link was shared, with nothing
    // more asked of the person yet.
    await press(button(w.sendDone))
    await until(() => document.querySelector('.history') !== null, 'the exchange')
    expect(document.querySelector('h1')?.textContent).toBe('Yup with Ben Ortiz')
    const card = document.querySelector('section.card')!
    expect(card.classList.contains('card-reminder')).toBe(false)
    expect(card.querySelector('h2')?.textContent).toBe('Ben Ortiz hasn’t joined yet')
    expect(card.textContent).toContain('You shared the link on')
    expect(card.textContent).toContain(w.unclaimed)
    expect(card.textContent).not.toContain('You haven’t sent them the link')
  })

  test('named by email: an email to the address first, and copying beside it', async () => {
    const { wording } = await signAndSend('carla@example.test')
    const w = wording.invitationLink
    const email = anchor(w.sendEmail)
    expect(email.className).toBe('button primary')
    expect(email.getAttribute('href')!.startsWith('mailto:carla@example.test?subject=')).toBe(true)
    expect(() => anchor(w.sendText)).toThrow()
    expect(() => anchor(w.sendWhatsApp)).toThrow()
    expect(button(w.copy).className).toBe('')
    expect(await violations()).toEqual([])
  })

  test('for anyone: copying first on a browser without a share sheet, the messages beside it', async () => {
    const { wording, service } = await signAndSend(null)
    const w = wording.invitationLink
    expect(document.body.textContent).toContain(w.forAnyoneSummary)
    expect(button(w.copy).className).toBe('primary')
    expect(anchor(w.shareSms).getAttribute('href')!.startsWith('sms:?body=')).toBe(true)
    expect(anchor(w.shareEmail).getAttribute('href')!.startsWith('mailto:?subject=')).toBe(true)
    expect(anchor(w.shareWhatsApp).getAttribute('href')!.startsWith('https://wa.me/?text=')).toBe(
      true,
    )
    expect(await violations()).toEqual([])

    await press(button(w.copy))
    await settle()
    expect(sharedCalls(service)).toBe(1)
    expect(copied).toEqual([`${window.location.origin}/en/i#${SENT_INVITATION}`])
    expect(document.body.textContent).toContain(w.copied)
    expect(document.activeElement?.textContent).toBe(w.sendDone)
  })

  test('“I’ll send it later” goes to the exchange, which reminds until the link is sent', async () => {
    const { wording, service } = await signAndSend('2025550142')
    const w = wording.invitationLink
    await press(button(w.later))
    await until(() => document.querySelector('.history') !== null, 'the exchange')
    expect(sharedCalls(service)).toBe(0)

    // The reminder: the first card, headed with who has not joined, saying
    // that nothing reaches them, and still holding the link and the ways to
    // send it, since the page was never left.
    const card = document.querySelector('section.card')!
    expect(card.classList.contains('card-reminder')).toBe(true)
    expect(card.getAttribute('aria-labelledby')).toBe('invitation-heading')
    expect(card.querySelector('h2')?.textContent).toBe('Ben Ortiz hasn’t joined yet')
    expect(card.querySelector('.notice-warning')?.textContent).toBe(
      'You haven’t sent them the link. Yuppers doesn’t send it for you: Ben Ortiz gets nothing until you do.',
    )
    expect(card.textContent).not.toContain(w.unclaimed)
    expect(card.querySelector('a.button.primary')?.textContent).toBe(w.sendText)
    expect(card.textContent).toContain(w.reissueIntro)
    expect(await violations()).toEqual([])

    // Sending it from here is recorded too, and the reminder goes.
    await press(card.querySelector<HTMLAnchorElement>('a.button.primary')!)
    await settle()
    expect(sharedCalls(service)).toBe(1)
    await until(() => document.querySelector('.card-reminder') === null, 'the reminder to go')
    expect(document.body.textContent).toContain('You shared the link on')
  })
})

describe('the reminder on the exchange’s page, on coming back', () => {
  test('a link never sent: the reminder, and a new link as the way to send it', async () => {
    const { wording } = await start(`/exchanges/${WAITING}`, ana)
    const w = wording.invitationLink
    await until(() => document.querySelector('.history') !== null, 'the exchange')
    const card = document.querySelector('.card-reminder')!
    expect(card.querySelector('h2')?.textContent).toBe('Ben Ortiz hasn’t joined yet')
    expect(card.querySelector('.notice-warning')?.textContent).toContain(
      'You haven’t sent them the link.',
    )
    // The link cannot be shown again: a new one is the way to send it.
    expect(card.textContent).toContain(w.sendAgain)
    expect(card.textContent).not.toContain(w.reissueIntro)
    expect(card.querySelector('a.button')).toBeNull()
    expect(button(w.reissue)).toBeTruthy()
    expect(await violations()).toEqual([])
  })

  test('a link sent long ago: the reminder says when, and to send it again', async () => {
    const { wording } = await start(`/exchanges/${WAITING}`, ana, 'en', (fake) => {
      fake.waitingSharedAt = '2026-09-01T09:00:00Z'
    })
    const w = wording.invitationLink
    await until(() => document.querySelector('.history') !== null, 'the exchange')
    const card = document.querySelector('.card-reminder')!
    expect(card.querySelector('.notice-warning')?.textContent).toMatch(
      /^You shared the link on September 1, 2026 at .*, and nobody has joined through it\. If Ben Ortiz hasn’t seen it, send it again, or create a new link\.$/,
    )
    expect(card.textContent).toContain(w.sendAgain)
  })

  test('a link sent a moment ago: no reminder, only where things stand', async () => {
    const { wording } = await start(`/exchanges/${WAITING}`, ana, 'en', (fake) => {
      fake.waitingSharedAt = new Date().toISOString()
    })
    const w = wording.invitationLink
    await until(() => document.querySelector('.history') !== null, 'the exchange')
    expect(document.querySelector('.card-reminder')).toBeNull()
    const card = document.querySelector('section.card')!
    expect(card.querySelector('h2')?.textContent).toBe('Ben Ortiz hasn’t joined yet')
    expect(card.textContent).toContain('You shared the link on')
    expect(card.textContent).toContain(w.unclaimed)
    expect(card.textContent).toContain(w.reissueIntro)
  })
})

describe('the list', () => {
  test('marks a yup whose link was never sent, and one waiting on the person it was sent to', async () => {
    const { wording } = await start('/', ana)
    await heading(wording.home.title)
    await until(() => document.querySelector('.cards') !== null, 'the list')
    const chips = [...document.querySelectorAll('.cards .chip')]
    expect(chips.map((chip) => chip.textContent)).toEqual([wording.home.notSent])
    expect(chips[0].className).toBe('chip chip-pending')
    // Beside the state, on the card of the yup waiting for its link to be sent.
    const card = chips[0].closest('.card')!
    expect(card.querySelector('.tag')?.textContent).toBe(wording.states.NEGOTIATING)
    expect(card.querySelector('.card-link')?.textContent).toBe('With Ben Ortiz')
    expect(await violations()).toEqual([])
  })

  test('once sent, the chip says who it waits for', async () => {
    const { wording } = await start('/', ana, 'en', (fake) => {
      fake.waitingSharedAt = '2026-10-02T06:35:00Z'
    })
    await heading(wording.home.title)
    await until(() => document.querySelector('.cards .chip') !== null, 'the chip')
    const chip = document.querySelector('.cards .chip')!
    expect(chip.textContent).toBe('Waiting for Ben Ortiz')
    expect(chip.className).toBe('chip chip-claimed')
  })
})
