/// <reference types="node" />
import { readFileSync } from 'node:fs'

import { describe, expect, test } from 'vitest'

import privacyEn from '../wording/privacy/en.json'
import privacyEs from '../wording/privacy/es.json'
import termsEn from '../wording/terms/en.json'
import termsEs from '../wording/terms/es.json'
import { languages, type Language } from './language'
import {
  LEGAL_DOCUMENTS,
  LEGAL_FIGURES,
  LEGAL_SECTIONS,
  PRIVACY_EMAIL,
  SUPPORT_EMAIL,
  legalAddress,
  legalEffectiveDate,
  legalInline,
  legalPath,
  legalPathOf,
  legalSections,
  type LegalDocument,
  type LegalWording,
  type PrivacyWording,
  type TermsWording,
} from './legal'

// Each language's documents have the shape both pages render. The wording
// check makes the languages match each other; this makes them match the code.
const documents = {
  privacy: { en: privacyEn, es: privacyEs } satisfies Record<Language, PrivacyWording>,
  terms: { en: termsEn, es: termsEs } satisfies Record<Language, TermsWording>,
} as Record<LegalDocument, Record<Language, LegalWording>>

const cases = LEGAL_DOCUMENTS.flatMap((document) =>
  Object.entries(documents[document]).map(
    ([language, wording]) => [document, language, wording] as const,
  ),
)

/** Every message of a document. */
function messages(wording: LegalWording): string[] {
  return [
    wording.title,
    wording.description,
    wording.note,
    wording.effective,
    wording.contents,
    wording.otherLanguages,
    ...Object.values(wording.sections).flatMap((section) => [
      section.title,
      ...section.blocks.flatMap((block) => {
        if ('ul' in block) return block.ul
        return ['h' in block ? block.h : block.p]
      }),
    ]),
  ]
}

const text = (message: string, language: string, document: LegalDocument, wording: LegalWording) =>
  legalInline(message, language, document, wording)
    .map((piece) => ('email' in piece ? piece.email : piece.text))
    .join('')

/** A section's text, filled in, as one string. */
const sectionText = (document: LegalDocument, language: string, wording: LegalWording, id: string) =>
  wording.sections[id].blocks
    .flatMap((block) => ('ul' in block ? block.ul : ['h' in block ? block.h : block.p]))
    .map((message) => text(message, language, document, wording))
    .join('\n')

describe('the documents', () => {
  test('every supported language has each', () => {
    for (const document of LEGAL_DOCUMENTS) {
      expect(Object.keys(documents[document]).sort()).toEqual(
        languages.map((info) => info.code).sort(),
      )
    }
  })

  test.each(cases)('%s in %s has every section, and no other', (document, _, wording) => {
    expect(Object.keys(wording.sections)).toEqual([...LEGAL_SECTIONS[document]])
    expect(legalSections(document, wording).map(({ id }) => id)).toEqual([
      ...LEGAL_SECTIONS[document],
    ])
  })

  test.each(cases)('%s in %s fills in every placeholder it uses', (document, language, wording) => {
    for (const message of messages(wording)) {
      expect(text(message, language, document, wording), message).not.toMatch(/[{}]|\*\*/)
    }
  })

  test.each(cases)('%s in %s starts each section with something to read', (_, __, wording) => {
    for (const section of Object.values(wording.sections)) {
      expect('p' in section.blocks[0] || 'ul' in section.blocks[0], section.title).toBe(true)
    }
  })

  test.each(cases)('%s in %s writes its addresses only as links', (document, language, wording) => {
    const pieces = messages(wording).flatMap((message) =>
      legalInline(message, language, document, wording),
    )
    expect(pieces).toContainEqual({ email: document === 'privacy' ? PRIVACY_EMAIL : SUPPORT_EMAIL })
    // Written out nowhere else: each address has one place, in legal-text.ts.
    for (const message of messages(wording)) expect(message).not.toContain('@')
  })

  test.each(cases)('%s in %s names yuppers.app at the start', (_, __, wording) => {
    const first = Object.values(wording.sections)[0].blocks[0]
    expect('p' in first && first.p).toContain('Yuppers.app (https://yuppers.app)')
  })
})

describe('what the SMS registration asks for', () => {
  const sell =
    'We do not sell your personal information. We do not share your personal information or your SMS opt-in data and consent with third parties or affiliates for marketing or promotional purposes.'
  const mobile =
    'Mobile information will not be shared with third parties or affiliates for marketing or promotional purposes.'

  test('the privacy policy says it in the body and in the section on texts, for both programs', () => {
    const wording = documents.privacy.en
    expect(wording.sections['what-we-collect'].title).toBe('What we collect and how we use it')
    expect(sectionText('privacy', 'en', wording, 'what-we-collect')).toContain(sell)
    const sms = sectionText('privacy', 'en', wording, 'text-messages')
    for (const required of [
      'Yuppers.app sign-in codes',
      'Message frequency: one message for each code you ask for.',
      'Yuppers.app agreement updates',
      'Message frequency varies; there is no fixed maximum: one text per status change',
      'Message and data rates may apply.',
      'Reply HELP for help.',
      'Reply STOP to stop receiving texts from us.',
      'Phone numbers outside +1 are not texted',
      sell,
      mobile,
      'Carriers are not liable for delayed or undelivered messages.',
    ]) {
      expect(sms).toContain(required)
    }
    // The opt-in record of text updates, and what it is used for.
    expect(sectionText('privacy', 'en', wording, 'what-we-collect')).toContain(
      'the consent wording you were shown. We use it only to send you those updates and to show that you agreed to receive them.',
    )
  })

  test('the terms list both programs, each with its name, description and frequency, and the rules once', () => {
    const sms = sectionText('terms', 'en', documents.terms.en, 'text-messages')
    for (const required of [
      'Program name: Yuppers.app sign-in codes.',
      'Yuppers.app (https://yuppers.app) texts a one-time sign-in code to the phone number you enter on the sign-in screen, only when you ask for one.',
      'Message frequency: One message per code request.',
      'Program name: Yuppers.app agreement updates.',
      'If you turn on text updates for an agreement, Yuppers.app texts you when its status changes: for example, when the other person signs, marks something delivered, confirms it, disputes it, or asks to close it.',
      '“Receive text updates from yuppers.app about this agreement, one text per status change. Message frequency varies; there is no fixed maximum. Msg & data rates may apply. Reply HELP for help or STOP to opt out. Terms: https://yuppers.app/terms. Privacy Policy: https://yuppers.app/privacy.”',
      'Message frequency: Message frequency varies; there is no fixed maximum. One text per status change of an agreement you turned updates on for.',
      'Message and data rates may apply.',
      'Reply HELP for help.',
      'Reply STOP to stop receiving texts.',
      'Carriers are not liable for delayed or undelivered messages.',
      SUPPORT_EMAIL,
      'Phone numbers outside +1 are not texted',
      sell,
      mobile,
    ]) {
      expect(sms).toContain(required)
    }
    for (const once of ['Message and data rates may apply.', 'Carriers are not liable']) {
      expect(sms.split(once)).toHaveLength(2)
    }
  })

  test('HELP and STOP are marked strong in every language and both documents', () => {
    for (const [document, language, wording] of cases) {
      const pieces = wording.sections['text-messages'].blocks
        .flatMap((block) => ('ul' in block ? block.ul : 'p' in block ? [block.p] : []))
        .flatMap((message) => legalInline(message, language, document, wording))
      const strong = pieces.filter((piece) => 'text' in piece && 'strong' in piece)
      expect(strong.some((piece) => 'text' in piece && piece.text.includes('HELP'))).toBe(true)
      expect(strong.some((piece) => 'text' in piece && piece.text.includes('STOP'))).toBe(true)
    }
  })
})

describe('inline pieces', () => {
  test('addresses, links to the other document and strong text are pieces of their own', () => {
    const wording = documents.terms.en
    expect(
      legalInline('**Reply STOP.** See {privacyTexts}, or {supportEmail}.', 'en', 'terms', wording),
    ).toEqual([
      { text: 'Reply STOP.', strong: true },
      { text: ' See ' },
      { document: 'privacy', section: 'text-messages', text: wording.links.privacyTexts },
      { text: ', or ' },
      { email: SUPPORT_EMAIL },
      { text: '.' },
    ])
  })

  test('a placeholder the documents do not define is refused', () => {
    expect(() => legalInline('{nothing}', 'en', 'privacy', { links: {} })).toThrow(/nothing/)
    // A link the document gives no words for is not a link it may use.
    expect(() => legalInline('{privacy}', 'en', 'privacy', { links: {} })).toThrow(/privacy/)
  })

  test('the effective date is written in each language', () => {
    expect(legalEffectiveDate('privacy', 'en')).toBe('October 5, 2026')
    expect(legalEffectiveDate('terms', 'es')).toBe('5 de octubre de 2026')
  })
})

describe('the figures the documents state', () => {
  const backend = (file: string) =>
    readFileSync(new URL(`../../../backend/src/${file}`, import.meta.url), 'utf8')
  const rule = (source: string, field: string, unit: string) => {
    const found = new RegExp(`${field}: Duration::${unit}\\((\\d+)\\)`).exec(source)
    if (!found) throw new Error(`no ${field} in the backend's defaults`)
    return Number(found[1])
  }

  test('are the service’s own', () => {
    const rules = backend('domain/mod.rs')
    const auth = backend('auth.rs')
    expect(LEGAL_FIGURES).toEqual({
      networkDays: rule(rules, 'network_metadata_retention', 'days'),
      codeMinutes: rule(auth, 'code_ttl', 'minutes'),
      sessionDays: rule(auth, 'session_ttl', 'days'),
    })
  })
})

describe('addresses', () => {
  test('the default language at /{document}, the others under their own code', () => {
    expect(legalPath('privacy', 'en')).toBe('/privacy')
    expect(legalPath('terms', 'es')).toBe('/es/terms')
    expect(legalAddress('https://yuppers.example/', 'privacy', 'es', 'text-messages')).toBe(
      'https://yuppers.example/es/privacy#text-messages',
    )
    expect(legalAddress('http://localhost:5173', 'terms', 'en')).toBe(
      'http://localhost:5173/terms',
    )
  })

  test('a path names the document it shows and the language', () => {
    expect(legalPathOf('/privacy')).toEqual({ document: 'privacy', language: 'en', named: false })
    expect(legalPathOf('/terms/')).toEqual({ document: 'terms', language: 'en', named: false })
    expect(legalPathOf('/es/privacy')).toEqual({ document: 'privacy', language: 'es', named: true })
    expect(legalPathOf('/ES/terms')).toEqual({ document: 'terms', language: 'es', named: true })
    expect(legalPathOf('/en/terms')).toEqual({ document: 'terms', language: 'en', named: true })
    for (const other of ['/', '/fr/privacy', '/privacy/more', '/help', '/es/i', '/es/help']) {
      expect(legalPathOf(other), other).toBeNull()
    }
  })
})
