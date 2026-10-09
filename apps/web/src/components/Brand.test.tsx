// @vitest-environment jsdom
import { afterEach, describe, expect, test } from 'vitest'

import { GOOD_CODE, INVITATION, ana } from '../test/fake-service'
import {
  button,
  field,
  heading,
  press,
  start,
  stop,
  type,
  until,
  violations,
} from '../test/harness'

/*
 * The brand on the screens that stand for the product itself: signing in and
 * setting up an account open with the mark and the wordmark, and the list of
 * yups with none in it shows the mark with what to do. The mark is decorative
 * and hidden; the mark and the word together are one image named for the
 * product; neither is a heading, so the page's own heading stays the first.
 */

afterEach(stop)

/** The mark and wordmark together, as assistive technology meets them. */
function lockup(): HTMLElement {
  const found = document.querySelector<HTMLElement>('.brand-hero [role="img"]')
  if (!found) throw new Error('no brand lockup on the page')
  return found
}

function expectBrand(productName: string, tagline: string) {
  const image = lockup()
  expect(image.getAttribute('aria-label')).toBe(productName)
  const mark = image.querySelector('svg')
  expect(mark?.getAttribute('aria-hidden')).toBe('true')
  expect(mark?.querySelector('.mark-you')).not.toBeNull()
  expect(mark?.querySelector('.mark-them')).not.toBeNull()
  expect(mark?.querySelector('.mark-agree')).not.toBeNull()
  // The whole word, as the wording spells it; the stylesheet draws it lower case.
  expect(image.querySelector('.wordmark')?.textContent).toBe(productName)
  expect(document.querySelector('.brand-hero .tagline')?.textContent).toBe(tagline)
  // Not a heading, and before the page's own, which stays the first heading.
  expect(document.querySelector('.brand-hero h1, .brand-hero h2, .brand-hero h3')).toBeNull()
  const first = document.querySelector('main h1, main h2, main h3')!
  expect(first.tagName).toBe('H1')
  const hero = document.querySelector('.brand-hero')!
  expect(hero.compareDocumentPosition(first) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy()
}

describe('signing in', () => {
  test('opens with the mark, the wordmark and what Yuppers is', async () => {
    const { wording } = await start('/', null)
    await heading(wording.signIn.title)
    expectBrand(wording.productName, wording.tagline)
    expect(await violations()).toEqual([])
  })

  test('keeps them through the code step and the profile', async () => {
    const { wording } = await start('/', null)
    await heading(wording.signIn.title)
    await type(field(wording.signIn.identifierLabel), 'ben@example.test')
    await press(button(wording.signIn.sendCode))
    await until(() => document.activeElement === field(wording.signIn.codeLabel), 'the code field')
    expectBrand(wording.productName, wording.tagline)
    expect(await violations()).toEqual([])

    await type(field(wording.signIn.codeLabel), GOOD_CODE)
    await press(button(wording.signIn.submit))
    await heading(wording.profile.firstTitle)
    expectBrand(wording.productName, wording.tagline)
    expect(await violations()).toEqual([])
  })

  test('below an invitation, which has a header of its own, opens plain', async () => {
    const { wording } = await start(`/en/i#${INVITATION}`, null)
    await heading(wording.invitation.signedOutTitle)
    await until(
      () => [...document.querySelectorAll('label')].some(
        (label) => label.textContent?.trim() === wording.signIn.identifierLabel,
      ),
      'the sign-in form',
    )
    expect(document.querySelector('.brand-hero')).toBeNull()
  })

  test('in another language, the word is still the product’s name', async () => {
    const { wording } = await start('/', null, 'es')
    await heading(wording.signIn.title)
    expectBrand(wording.productName, wording.tagline)
  })
})

describe('the list of yups', () => {
  test('with none yet: the mark, what to do and the way to start, together', async () => {
    const { wording } = await start('/', ana, 'en', (fake) => {
      fake.noExchanges = true
    })
    await heading(wording.home.title)
    await until(() => document.querySelector('.empty-state') !== null, 'the empty state')
    const empty = document.querySelector('.empty-state')!
    expect(empty.querySelector('svg')?.getAttribute('aria-hidden')).toBe('true')
    expect(empty.querySelector('svg .mark-agree')).not.toBeNull()
    expect(empty.textContent).toContain(wording.home.empty)
    // The one way to start is in it, not above an empty list as well.
    const starts = [...document.querySelectorAll('button')].filter(
      (found) => found.textContent?.trim() === wording.home.start,
    )
    expect(starts).toHaveLength(1)
    expect(empty.contains(starts[0])).toBe(true)
    expect(await violations()).toEqual([])
  })

  test('with yups, the way to start is above them and there is no empty state', async () => {
    const { wording } = await start('/', ana)
    await heading(wording.home.title)
    await until(() => document.querySelector('.cards') !== null, 'the list')
    expect(document.querySelector('.empty-state')).toBeNull()
    expect(document.body.textContent).not.toContain(wording.home.empty)
    const list = document.querySelector('.cards')!
    expect(button(wording.home.start).compareDocumentPosition(list) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy()
  })
})
