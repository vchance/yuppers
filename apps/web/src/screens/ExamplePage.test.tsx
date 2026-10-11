// @vitest-environment jsdom
import { wordingFor } from '@yuppers/shared'
import { afterEach, describe, expect, test } from 'vitest'

import { INVITATION, ana } from '../test/fake-service'
import { button, heading, press, start, stop, until, violations } from '../test/harness'

/*
 * The sample yup (DESIGN.md §4.3): a worked example anyone can read, signed
 * out or in, in either language. Read-only, drawn from a fixed document with
 * no service behind it, and unmistakably an example in words.
 */

afterEach(stop)

/** Every request that is about a yup, in the exchanges or in an invitation. */
function aboutYups(sent: { call: string }[]) {
  return sent.filter(
    (request) => request.call.includes('/v1/exchanges') || request.call.includes('/v1/invitations'),
  )
}

describe('the example page', () => {
  test('is readable signed out, says it is an example in words, and asks the service nothing about yups', async () => {
    const { wording, service } = await start('/example', null)
    const w = wording.sample
    await heading(w.title)

    expect(document.body.textContent).toContain(w.banner)
    expect(document.title).toBe(`${w.title} · ${wording.productName}`)
    // The two made-up people and their deal.
    expect(document.body.textContent).toContain('Dana Reyes')
    expect(document.body.textContent).toContain('Sam Okafor')
    expect(document.body.textContent).toContain(w.terms)
    expect(document.body.textContent).toContain('$100.00')
    expect(document.body.textContent).toContain('$300.00')
    // Where the record would show a fingerprint, the example says so.
    expect(document.body.textContent).toContain(w.fingerprint)
    expect(document.body.textContent).not.toContain('PVVS')
    expect(aboutYups(service.sent)).toEqual([])
    expect(await violations()).toEqual([])
  })

  test('shows each action as it would be, by its own name, disabled, and says why', async () => {
    const { wording } = await start('/example', null)
    const w = wording.sample
    await heading(w.title)

    // Nothing in the page can be pressed except to leave it.
    const enabled = [...document.querySelectorAll<HTMLButtonElement>('main button')].filter(
      (found) => !found.disabled,
    )
    expect(enabled).toEqual([])
    const disabled = [...document.querySelectorAll<HTMLButtonElement>('main button')]
    expect(disabled.map((found) => found.textContent)).toEqual([
      wording.exchange.moves.CLAIM,
      wording.exchange.moneyMoves.CLAIM,
    ])
    // Each says why, in words a screen reader reads with the button.
    for (const found of disabled) {
      const why = document.getElementById(found.getAttribute('aria-describedby')!)
      expect(why?.textContent).toContain(w.actionsOff)
    }
    expect(document.body.textContent).toContain(w.wouldTap.replace('{name}', w.firstNameA))
    expect(document.body.textContent).toContain(w.waitsOn)
    // No sign, confirm, dispute, report or block, and no link, code or download.
    for (const absent of [
      wording.exchange.accept,
      wording.exchange.moves.CONFIRM,
      wording.exchange.moves.DISPUTE,
    ]) {
      expect(document.body.textContent).not.toContain(absent)
    }
    expect(document.querySelector('a[href*="/exchanges/"]')).toBeNull()
  })

  test('shows the terms, the plain summary and the history of six steps', async () => {
    const { wording } = await start('/example', null)
    await heading(wording.sample.title)
    expect(document.querySelector('.terms')).not.toBeNull()
    expect(document.querySelector('.record-plain')).not.toBeNull()
    expect(document.querySelectorAll('.history-entry')).toHaveLength(6)
    // The confirmed deposit, the pending repair and the balance waiting on it.
    expect(document.body.textContent).toContain(wording.moneyStatus.ACCEPTED)
    expect(document.body.textContent).toContain(wording.contributionStatus.PENDING)
  })

  test('in Spanish, at /es/example, with the banner, the names and the dollars', async () => {
    const { wording } = await start('/es/example', null, 'es')
    const w = wording.sample
    await heading(w.title)
    expect(document.documentElement.lang).toBe('es')
    expect(document.body.textContent).toContain(w.banner)
    expect(document.body.textContent).toContain('Dana Reyes')
    expect(document.body.textContent).toContain(w.terms)
    expect(await violations()).toEqual([])
  })

  test('an address in Spanish is read in Spanish by someone whose page was in English', async () => {
    await start('/es/example', null, 'en')
    const es = wordingFor('es')
    await until(() => document.documentElement.lang === 'es', 'Spanish')
    await heading(es.sample.title)
  })

  test('is just as read-only for someone signed in', async () => {
    const { wording, service } = await start('/example', ana)
    await heading(wording.sample.title)
    expect(aboutYups(service.sent)).toEqual([])
    expect(
      [...document.querySelectorAll<HTMLButtonElement>('main button')].every((b) => b.disabled),
    ).toBe(true)
    expect(await violations()).toEqual([])
  })
})

describe('the ways in', () => {
  test('the empty home screen shows a card headed “this is what a yup looks like”, with the example', async () => {
    const { wording } = await start('/', ana, 'en', (fake) => {
      fake.noExchanges = true
    })
    const w = wording.sample
    await heading(wording.home.title)
    await until(() => document.querySelector('.example-card') !== null, 'the card')
    const card = document.querySelector('.example-card')!
    expect(card.querySelector('h2')?.textContent).toBe(w.homeHeading)
    expect(card.textContent).toContain(w.cardState)
    expect(await violations()).toEqual([])

    const open = [...card.querySelectorAll('a')].find((link) => link.textContent === w.see)!
    await press(open)
    await heading(w.title)
    expect(window.location.pathname).toBe('/example')
  })

  test('the card is gone once the person has a yup of their own', async () => {
    const { wording } = await start('/', ana)
    await heading(wording.home.title)
    await until(() => document.querySelector('.cards') !== null, 'the list')
    expect(document.querySelector('.example-card')).toBeNull()
    expect(button(wording.home.start)).toBeTruthy()
  })

  test('the sign-in screen has one line below the form that opens it', async () => {
    const { wording } = await start('/', null)
    await heading(wording.signIn.title)
    const line = [...document.querySelectorAll('a')].find(
      (link) => link.textContent === wording.sample.signInLine,
    )!
    expect(line.getAttribute('href')).toBe('/example')
    await press(line)
    await heading(wording.sample.title)
  })

  test('the invitation page does not show it', async () => {
    const { wording } = await start(`/en/i#${INVITATION}`, null)
    await heading(wording.invitation.signedOutTitle)
    expect(document.body.textContent).not.toContain(wording.sample.signInLine)
  })

  test('the account screen keeps the way back to it', async () => {
    const { wording } = await start('/account', ana)
    await heading(wording.profile.title)
    expect(
      [...document.querySelectorAll('a')].some((link) => link.textContent === wording.sample.help),
    ).toBe(true)
  })
})
