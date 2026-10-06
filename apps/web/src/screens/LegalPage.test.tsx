// @vitest-environment jsdom
import {
  languages,
  LEGAL_DOCUMENTS,
  LEGAL_SECTIONS,
  legalInline,
  PRIVACY_EMAIL,
  SUPPORT_EMAIL,
  type LegalDocument,
  type LegalWording,
} from '@yuppers/shared'
import { afterEach, describe, expect, test } from 'vitest'

import { loadHelp, loadLegal } from '../app/wording'
import { ana } from '../test/fake-service'
import { heading, press, start, stop, until } from '../test/harness'
import { smsRequirements } from '../test/sms-requirements'

/*
 * The privacy policy and the terms, run in the whole app: every section in
 * every language at its own address, what the SMS registration asks of
 * them, the ways between them, and the links the sign-in, account and
 * deletion screens give to them. The static pages the build writes at the
 * same addresses are `build/legal-pages.test.ts`.
 */

afterEach(stop)

const codes = languages.map((info) => info.code)
const cases = LEGAL_DOCUMENTS.flatMap((document) => codes.map((code) => [document, code] as const))
const address = (document: LegalDocument, language: string) =>
  language === 'en' ? `/${document}` : `/${language}/${document}`

/** A link whose text, without what only a screen reader hears, is exactly `text`. */
function link(text: string, within: ParentNode = document): HTMLAnchorElement {
  const found = [...within.querySelectorAll('a')].find((candidate) => {
    const copy = candidate.cloneNode(true) as HTMLElement
    for (const hidden of copy.querySelectorAll('.visually-hidden')) hidden.remove()
    return copy.textContent?.trim() === text
  })
  if (!found) throw new Error(`no link “${text}”`)
  return found
}

const filled = (message: string, language: string, kind: LegalDocument, wording: LegalWording) =>
  legalInline(message, language, kind, wording)
    .map((piece) => ('email' in piece ? piece.email : piece.text))
    .join('')

describe.each(cases)('%s in %s', (kind, language) => {
  test('every section is on the page, under its anchor, in order', async () => {
    const wording = await loadLegal(kind, language)
    const started = await start(address(kind, language), null, language)
    await heading(wording.title)

    expect(document.title).toBe(`${wording.title} · ${started.wording.productName}`)
    expect(document.documentElement.lang).toBe(language)
    const main = document.querySelector('main')!
    const shown = main.textContent!
    expect(shown).toContain(wording.note)
    const anchors = [...main.querySelectorAll('section > h2')].map((h2) => h2.id)
    expect(anchors).toEqual([...LEGAL_SECTIONS[kind]])
    for (const id of LEGAL_SECTIONS[kind]) {
      const section = (wording.sections as Record<string, LegalWording['sections'][string]>)[id]
      expect(document.getElementById(id)?.textContent).toBe(section.title)
      for (const block of section.blocks) {
        const parts = 'h' in block ? [block.h] : 'p' in block ? [block.p] : block.ul
        for (const part of parts) expect(shown).toContain(filled(part, language, kind, wording))
      }
    }
    expect(shown).not.toMatch(/[{}]|\*\*/)

    // The contents lead to each section.
    const contents = [...main.querySelectorAll('nav.help-sections a')]
    expect(contents.map((a) => a.getAttribute('href'))).toEqual(
      LEGAL_SECTIONS[kind].map((id) => `#${id}`),
    )
    // Where to write is a link to write to.
    const email = kind === 'privacy' ? PRIVACY_EMAIL : SUPPORT_EMAIL
    expect(main.querySelector(`a[href="mailto:${email}"]`)?.textContent).toBe(email)
    expect(main.querySelector('time')?.getAttribute('datetime')).toBe('2026-10-06')
    // The address stays the one the page is read at, and the footer marks it.
    expect(window.location.pathname).toBe(address(kind, language))
    const footer = document.querySelector('footer')!
    const words = kind === 'privacy' ? started.wording.privacy.link : started.wording.termsOfUse.link
    expect(link(words, footer).getAttribute('aria-current')).toBe('page')
    expect(link(words, footer).getAttribute('href')).toBe(address(kind, language))
  })

  test('it says what the SMS registration asks for', async () => {
    const wording = await loadLegal(kind, language)
    await start(address(kind, language), null, language)
    await heading(wording.title)
    expect(smsRequirements(document, kind, language)).toEqual([])
  })
})

describe('the requirements check itself', () => {
  test('finds HELP and STOP missing from <strong>, and a missing privacy link', async () => {
    const wording = await loadLegal('terms', 'en')
    await start('/terms', null)
    await heading(wording.title)
    const texts = document.getElementById('text-messages')!.closest('section')!
    for (const strong of texts.querySelectorAll('strong')) strong.replaceWith(strong.textContent!)
    for (const a of texts.querySelectorAll('a[href*="privacy"]')) a.remove()
    expect(smsRequirements(document, 'terms', 'en')).toEqual([
      '#text-messages: HELP is not inside a <strong>',
      '#text-messages: STOP is not inside a <strong>',
      '#text-messages: no link to /privacy#text-messages',
    ])
  })
})

describe('getting around', () => {
  test('a link to a section arrives there once the text has', async () => {
    const wording = await loadLegal('privacy', 'en')
    await start('/privacy#text-messages', null)
    await heading(wording.title)
    await until(
      () => document.activeElement?.id === 'text-messages',
      'the focus on the text messages section',
    )
    expect(window.location.hash).toBe('#text-messages')
  })

  test('/privacy read in Spanish moves to /es/privacy, keeping the section', async () => {
    const wording = await loadLegal('privacy', 'es')
    await start('/privacy?lang=es#text-messages', null, 'es')
    await heading(wording.title)
    expect(window.location.pathname).toBe('/es/privacy')
    expect(window.location.search).toBe('')
    expect(window.location.hash).toBe('#text-messages')
  })

  test('the link to another language shows the page in it, at its address', async () => {
    const english = await loadLegal('terms', 'en')
    const spanish = await loadLegal('terms', 'es')
    await start('/terms', null)
    await heading(english.title)
    const nav = document.querySelector(`nav[aria-label="${english.otherLanguages}"]`)!
    const español = [...nav.querySelectorAll('a')].find((a) => a.lang === 'es')!
    expect(español.getAttribute('href')).toBe('/es/terms')
    await press(español)
    await heading(spanish.title)
    expect(window.location.pathname).toBe('/es/terms')
    expect(document.documentElement.lang).toBe('es')
  })

  test('the terms lead to the privacy policy’s section on texts, and the policy back to the terms', async () => {
    const terms = await loadLegal('terms', 'en')
    const privacy = await loadLegal('privacy', 'en')
    await start('/terms', null)
    await heading(terms.title)
    const texts = document.getElementById('text-messages')!.closest('section')!
    await press(link(terms.links.privacyTexts!, texts))
    await heading(privacy.title)
    expect(window.location.pathname).toBe('/privacy')
    expect(window.location.hash).toBe('#text-messages')
    await press(link(privacy.links.terms!, document.querySelector('main')!))
    await heading(terms.title)
    expect(window.location.pathname).toBe('/terms')
  })
})

describe('the ways to them', () => {
  test('from the footer, without reloading', async () => {
    const { wording } = await start('/', null)
    await heading(wording.signIn.title)
    await press(link(wording.privacy.link))
    await heading((await loadLegal('privacy', 'en')).title)
    expect(window.location.pathname).toBe('/privacy')
    await press(link(wording.termsOfUse.link))
    await heading((await loadLegal('terms', 'en')).title)
    expect(window.location.pathname).toBe('/terms')
  })

  test('from the list of help topics', async () => {
    const help = await loadHelp('es')
    const { wording } = await start('/help', null, 'es')
    await heading(help.title)
    expect(link(wording.privacy.policy).getAttribute('href')).toBe('/es/privacy')
    expect(link(wording.termsOfUse.document).getAttribute('href')).toBe('/es/terms')
  })

  test('signing in links to both, in a new tab, and says nothing of texts while codes go by email only', async () => {
    const { wording } = await start('/', null, 'en', (service) => {
      service.phone = false
    })
    await heading(wording.signIn.title)
    await until(
      () => document.querySelector('main')!.textContent!.includes(wording.signIn.introEmail),
      'the form for email addresses only',
    )
    const privacy = link(wording.privacy.policy)
    expect(privacy.getAttribute('href')).toBe('/privacy')
    expect(privacy.getAttribute('target')).toBe('_blank')
    expect(privacy.textContent).toContain(wording.help.newTab)
    expect(link(wording.termsOfUse.document).getAttribute('href')).toBe('/terms')
    expect(document.querySelector('main')!.textContent).not.toContain(wording.privacy.smsLink)
    expect(document.querySelector('input[type="checkbox"]')).toBeNull()
  })

  test('where codes can go to phone numbers, signing in links to the section on texts', async () => {
    const { wording } = await start('/', null, 'es', (service) => {
      service.phone = true
    })
    await heading(wording.signIn.title)
    await until(
      () => document.querySelector('main')!.textContent!.includes(wording.privacy.smsLink),
      'the link on text messages',
    )
    expect(link(wording.privacy.smsLink).getAttribute('href')).toBe('/es/privacy#text-messages')
    expect(link(wording.privacy.policy).getAttribute('href')).toBe('/es/privacy')
    expect(link(wording.termsOfUse.document).getAttribute('href')).toBe('/es/terms')
  })

  test('the account page links to both, and deleting the account to what stays', async () => {
    const { wording } = await start('/account', ana)
    await heading(wording.profile.title)
    expect(link(wording.privacy.policy).getAttribute('href')).toBe('/privacy')
    expect(link(wording.termsOfUse.document).getAttribute('href')).toBe('/terms')
    const deletion = link(wording.privacy.deletion)
    expect(deletion.getAttribute('href')).toBe('/privacy#deleting-your-account')
    expect(deletion.getAttribute('target')).toBe('_blank')
  })
})
