// @vitest-environment jsdom
import { HELP_TOPICS, languages, PRIVACY_EMAIL, SUPPORT_EMAIL, timeZoneCity } from '@yuppers/shared'
import {
  formattedWords,
  pseudoHelp,
  pseudoPrivacy,
  pseudoTerms,
  pseudoWording,
  untranslated,
} from '@yuppers/shared/testing/pseudo'
import { afterEach, describe, expect, test, vi } from 'vitest'

import {
  ACTIVE,
  AMENDING,
  COUNTER,
  DISPUTED,
  DRAFT,
  ENDED,
  GOOD_CODE,
  INVITATION,
  OFFER,
  REPORT,
  STAND_IN_TEXT,
  ana,
  rita,
} from './test/fake-service'
import { button, field, press, settle, start, stop, type, until } from './test/harness'

/*
 * Text written into a component instead of the wording stays in English
 * whatever language the person reads. Here the app runs in a pseudo-language
 * made from the English wording, with every Latin letter accented
 * (`@yuppers/shared/testing/pseudo`), and the main screens are read for any
 * plain Latin letter that did not come through it.
 *
 * What may still have plain letters, and nothing else:
 *   - what people wrote or chose, as the stand-in service holds it
 *     (`STAND_IN_TEXT`): names, terms, notes, the currency and time zone;
 *   - what the platform formats: month names, day periods, time zone names
 *     and the words joining a date to its time;
 *   - each language's own name, in the language picker, and its tag, as
 *     the record says which language a signature's consent was shown in;
 *   - things that are not words: references, ids, hashes, email addresses.
 */

const pseudo = pseudoWording()
const pseudoHelpPages = pseudoHelp()
const pseudoLegal = { privacy: pseudoPrivacy(), terms: pseudoTerms() }
const ALLOWED = [
  ...STAND_IN_TEXT,
  // The addresses to write to, which the privacy policy and the terms link to.
  PRIVACY_EMAIL,
  SUPPORT_EMAIL,

  ...formattedWords('en', ['America/Chicago']),
  // The exchange's time zone by its city, beside a due date for a device elsewhere.
  timeZoneCity('America/Chicago'),
  ...languages.flatMap((language) => [language.name, language.code]),
]

/** Text a person can read or hear: text, and the attributes that are read out or shown. */
function readable(): string[] {
  const found: string[] = [document.title]
  const walker = document.createTreeWalker(document.body, NodeFilter.SHOW_TEXT)
  for (let node = walker.nextNode(); node; node = walker.nextNode()) {
    const parent = node.parentElement
    if (parent && ['SCRIPT', 'STYLE'].includes(parent.tagName)) continue
    if (node.textContent?.trim()) found.push(node.textContent)
  }
  for (const element of document.body.querySelectorAll('*')) {
    for (const name of ['aria-label', 'placeholder', 'title', 'alt', 'aria-roledescription']) {
      const value = element.getAttribute(name)
      if (value) found.push(value)
    }
    if (element instanceof HTMLInputElement && ['button', 'submit'].includes(element.type)) {
      found.push(element.value)
    }
  }
  return found
}

/** Every piece of readable text with a plain Latin letter that is not allowed, with its words. */
function leaks(): string[] {
  return readable().flatMap((text) => {
    const words = untranslated(text, ALLOWED)
    return words.length > 0 ? [`${JSON.stringify(text.trim())}: ${words.join(' ')}`] : []
  })
}

/** Everything found on every screen visited, so one run lists all of it. */
async function screens(visit: (check: () => Promise<void>) => Promise<void>): Promise<string[]> {
  const found = new Set<string>()
  await visit(async () => {
    await settle(50)
    for (const leak of leaks()) found.add(leak)
  })
  return [...found]
}

const h1 = (text: string) => until(() => document.querySelector('h1')?.textContent === text, text)

afterEach(stop)

describe('every word on the main web screens comes from the wording', () => {
  test('the invitation page, signed in to, read and then answered', async () => {
    const found = await screens(async (check) => {
      await start(`/en/i#${INVITATION}`, null, pseudo)
      await h1(pseudo.invitation.signedOutTitle)
      await check()
      await type(field(pseudo.signIn.identifierLabel), 'ben@example.test')
      await press(button(pseudo.signIn.sendCode))
      await until(() => document.activeElement === field(pseudo.signIn.codeLabel), 'the code field')
      await type(field(pseudo.signIn.codeLabel), GOOD_CODE)
      await press(button(pseudo.signIn.submit))
      await until(() => document.querySelector('.terms') !== null, 'the proposal')
      await check()
      await press(button(pseudo.invitation.respondNew))
      await until(
        () =>
          [...document.querySelectorAll('h2')].some(
            (h) => h.textContent === pseudo.profile.firstTitle,
          ),
        'the profile',
      )
      await check()
    })
    expect(found).toEqual([])
  })

  test('signing in, a wrong code, and a new account’s profile', async () => {
    const found = await screens(async (check) => {
      await start('/', null, pseudo)
      await h1(pseudo.signIn.title)
      await check()
      // A phone number: the box beside it, unticked and then ticked.
      await type(field(pseudo.signIn.identifierLabel), '+12015550123')
      await check()
      await press(field(pseudo.smsCode.signIn))
      await check()
      await type(field(pseudo.signIn.identifierLabel), 'ben@example.test')
      await press(button(pseudo.signIn.sendCode))
      await until(() => document.activeElement === field(pseudo.signIn.codeLabel), 'the code field')
      await check()
      await type(field(pseudo.signIn.codeLabel), '000000')
      await press(button(pseudo.signIn.submit))
      await until(() => document.body.textContent!.includes(pseudo.errors.INVALID_CODE), 'refusal')
      await check()
      await type(field(pseudo.signIn.codeLabel), GOOD_CODE)
      await press(button(pseudo.signIn.submit))
      await h1(pseudo.profile.firstTitle)
      await check()
      await press(button(pseudo.profile.continue))
      await check()
    })
    expect(found).toEqual([])
  })

  test('the composer, writing and then signing', async () => {
    const found = await screens(async (check) => {
      await start(`/exchanges/${DRAFT}`, ana, pseudo)
      await h1(pseudo.composer.titleFirst)
      await check()
      await press(button(pseudo.composer.review))
      await h1(pseudo.composer.signTitle)
      await check()
    })
    expect(found).toEqual([])
  })

  test('who the invitation is for, then the link and the ways to share it', async () => {
    const found = await screens(async (check) => {
      await start(`/exchanges/${DRAFT}`, ana, pseudo)
      await h1(pseudo.composer.titleFirst)
      // What is wrong with it, said under it.
      await until(() => document.body.textContent!.includes(pseudo.invitationLink.forHint), 'the field')
      await type(field(pseudo.invitationLink.forLabel), 'carla@')
      await press(button(pseudo.composer.review))
      await check()
      await type(field(pseudo.invitationLink.forLabel), 'carla@example.test')
      await press(button(pseudo.composer.review))
      await h1(pseudo.composer.signTitle)
      await check()
      await press(document.querySelector<HTMLInputElement>('.consent input[type=checkbox]')!)
      await press(button(pseudo.composer.signAndSend))
      await until(() => document.body.textContent!.includes(pseudo.invitationLink.intro), 'the link')
      await check()
      await press(button(pseudo.invitationLink.share))
      await press(button(pseudo.invitationLink.shareQr))
      await until(() => document.querySelector('svg') !== null, 'the QR code')
      await press(button(pseudo.invitationLink.copy))
      await check()
    })
    expect(found).toEqual([])
  })

  test('the exchange in each state a person meets it in, with its panels open', async () => {
    const found = await screens(async (check) => {
      for (const id of [OFFER, ACTIVE, DISPUTED, AMENDING, COUNTER, ENDED]) {
        await start(`/exchanges/${id}`, ana, pseudo)
        await until(() => document.querySelector('.history') !== null, 'the history')
        await check()
      }
      await start(`/exchanges/${OFFER}`, ana, pseudo)
      await until(() => document.querySelector('.history') !== null, 'the history')
      await press(button(pseudo.exchange.accept))
      await check()

      await start(`/exchanges/${ACTIVE}`, ana, pseudo)
      await until(() => document.querySelector('.history') !== null, 'the history')
      await press(button(pseudo.trouble.open))
      await check()
      for (const situation of Object.values(pseudo.trouble.situations)) {
        const text = situation.replace('{name}', 'Ben Ortiz')
        const choice = [...document.querySelectorAll('button')].find(
          (candidate) => candidate.textContent?.trim() === text,
        )
        if (!choice) continue
        await press(choice)
        await check()
      }
    })
    expect(found).toEqual([])
  })

  test('the record, in force and ended', async () => {
    const found = await screens(async (check) => {
      for (const id of [ACTIVE, ENDED]) {
        await start(`/exchanges/${id}/record`, ana, pseudo)
        await until(() => document.querySelector('.record-plain') !== null, 'the summary')
        await check()
      }
    })
    expect(found).toEqual([])
  })

  test('staff review: the queue, a report and its decision', async () => {
    const found = await screens(async (check) => {
      await start('/staff', rita, pseudo)
      await h1(pseudo.staff.title)
      await until(() => document.querySelectorAll('.card').length >= 3, 'the queue')
      await check()
      await press(button(pseudo.staff.lift))
      await check()
      await start(`/staff/reports/${REPORT}`, rita, pseudo)
      await until(() => document.querySelector('.history') !== null, 'the record')
      await check()
      await press(button(pseudo.staff.outcomes.CONTENT_HIDDEN))
      await press(button(pseudo.staff.confirm))
      await check()
    })
    expect(found).toEqual([])
  })

  test('the list, the account and deleting it, and a page that is not there', async () => {
    const found = await screens(async (check) => {
      await start('/', ana, pseudo)
      await h1(pseudo.home.title)
      await until(() => document.querySelector('.card') !== null, 'the list')
      await check()
      await start('/account', ana, pseudo)
      await h1(pseudo.profile.title)
      await check()
      await press(button(pseudo.deletion.open))
      await check()
      await start('/nowhere', ana, pseudo)
      await h1(pseudo.common.notFoundTitle)
      await check()
    })
    expect(found).toEqual([])
  })

  test('the help pages, the list of topics and every topic', async () => {
    // The help pages' text is a file of its own, fetched by the help page;
    // here it comes in the pseudo-language too.
    vi.doMock('./app/wording', async (original) => ({
      ...(await original<typeof import('./app/wording')>()),
      loadHelp: async () => pseudoHelpPages,
    }))
    try {
      const found = await screens(async (check) => {
        await start('/help', null, pseudo)
        await h1(pseudoHelpPages.title)
        await check()
        for (const topic of HELP_TOPICS) {
          await start(`/help/${topic}`, null, pseudo)
          await h1(pseudoHelpPages.topics[topic].title)
          await check()
        }
        await start('/help/nothing-here', null, pseudo)
        await h1(pseudo.common.notFoundTitle)
        await check()
      })
      expect(found).toEqual([])
    } finally {
      vi.doUnmock('./app/wording')
    }
  })

  test('the privacy policy and the terms', async () => {
    // Their text is a file of its own, fetched by their page; here it comes
    // in the pseudo-language too.
    vi.doMock('./app/wording', async (original) => ({
      ...(await original<typeof import('./app/wording')>()),
      loadLegal: async (document: 'privacy' | 'terms') => pseudoLegal[document],
      loadedLegal: () => null,
    }))
    try {
      const found = await screens(async (check) => {
        for (const document of ['privacy', 'terms'] as const) {
          await start(`/${document}`, null, pseudo)
          await h1(pseudoLegal[document].title)
          await check()
        }
      })
      expect(found).toEqual([])
    } finally {
      vi.doUnmock('./app/wording')
    }
  })
})
