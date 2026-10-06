/// <reference types="node" />
import { readFileSync } from 'node:fs'

import { describe, expect, test } from 'vitest'

import en from '../wording/en.json'
import es from '../wording/es.json'
import termsEn from '../wording/terms/en.json'
import termsEs from '../wording/terms/es.json'
import { legalInline, type LegalWording } from './legal'
import { readsAsPhone, SMS_CODE_CONSENT_VERSION, smsCodeConsent } from './sms-code-consent'
import { consentPieces } from './sms-updates'

/*
 * Consent to a code by text: the words beside the box on each form, which
 * must be the owner's in English and the terms' quote in every language,
 * the version the service checks, and when a form shows the box at all.
 */

const purposes = ['signIn', 'deleteAccount', 'verifyNumber'] as const

/** The terms' section on texts as a reader gets it, one line per paragraph or list entry. */
function termsOnTexts(terms: LegalWording, language: string): string {
  return terms.sections['text-messages'].blocks
    .flatMap((block) => ('ul' in block ? block.ul : 'p' in block ? [block.p] : []))
    .map((message) =>
      legalInline(message, language, 'terms', terms)
        .map((piece) => ('email' in piece ? piece.email : piece.text))
        .join(''),
    )
    .join('\n')
}

describe('the words beside the box', () => {
  test('are the owner’s, word for word, on each form', () => {
    const rest =
      ' One message per request. Msg & data rates may apply. Reply HELP for help or STOP to opt out. Terms: https://yuppers.app/terms. Privacy Policy: https://yuppers.app/privacy.'
    expect(en.smsCode.signIn).toBe(
      `Text me a one-time sign-in code from yuppers.app at this number.${rest}`,
    )
    expect(en.smsCode.deleteAccount).toBe(
      `Text me a one-time code to confirm deleting your account from yuppers.app at this number.${rest}`,
    )
    expect(en.smsCode.verifyNumber).toBe(
      `Text me a one-time code to confirm this number from yuppers.app at this number.${rest}`,
    )
  })

  test('differ from one form to the next only where the owner said', () => {
    for (const purpose of purposes.slice(1)) {
      expect(en.smsCode[purpose]).toBe(
        en.smsCode.signIn.replace(
          'a one-time sign-in code',
          purpose === 'deleteAccount'
            ? 'a one-time code to confirm deleting your account'
            : 'a one-time code to confirm this number',
        ),
      )
    }
  })

  test.each([
    ['en', en],
    ['es', es],
  ] as const)('in %s keep HELP, STOP and the two addresses as they are, as links', (_, wording) => {
    for (const purpose of purposes) {
      const label = wording.smsCode[purpose]
      expect(label).toMatch(/\bHELP\b/)
      expect(label).toMatch(/\bSTOP\b/)
      expect(label).toContain('yuppers.app')
      const pieces = consentPieces(label)
      expect(pieces.filter((piece) => 'url' in piece)).toEqual([
        { url: 'https://yuppers.app/terms' },
        { url: 'https://yuppers.app/privacy' },
      ])
      expect(pieces.map((piece) => ('url' in piece ? piece.url : piece.text)).join('')).toBe(label)
    }
  })

  test.each([
    ['en', en, termsEn],
    ['es', es, termsEs],
  ] as const)('in %s are the ones the terms quote for signing in', (language, wording, terms) => {
    const section = termsOnTexts(terms as LegalWording, language)
    expect(section).toContain(`“${wording.smsCode.signIn}”`)
  })

  test.each([
    ['en', en, termsEn, ['a one-time sign-in code', 'a one-time code to confirm deleting your account', 'a one-time code to confirm this number']],
    ['es', es, termsEs, ['para entrar', 'para confirmar la eliminación de mi cuenta', 'para confirmar este número']],
  ] as const)(
    'in %s are, on the other two forms, what the terms say they are',
    (language, wording, terms, [signIn, deleteAccount, verifyNumber]) => {
      const section = termsOnTexts(terms as LegalWording, language)
      for (const words of [signIn, deleteAccount, verifyNumber]) expect(section).toContain(`“${words}”`)
      expect(wording.smsCode.deleteAccount).toBe(wording.smsCode.signIn.replace(signIn, deleteAccount))
      expect(wording.smsCode.verifyNumber).toBe(wording.smsCode.signIn.replace(signIn, verifyNumber))
    },
  )

  test('have the version the service checks', () => {
    const backend = readFileSync(
      new URL('../../../backend/src/code_consent.rs', import.meta.url),
      'utf8',
    )
    expect(backend).toContain(`pub const CODE_CONSENT_VERSION: &str = "${SMS_CODE_CONSENT_VERSION}";`)
    expect(smsCodeConsent('es')).toEqual({ version: SMS_CODE_CONSENT_VERSION, language: 'es' })
  })
})

describe('what reads as a phone number', () => {
  test('is a number as people write one, from its first digit', () => {
    for (const typed of ['+', '+1', '5', '+1 201 555 0123', '(201) 555-0123', '201.555.0123', ' +12015550123 ']) {
      expect(readsAsPhone(typed), typed).toBe(typed.trim() !== '+')
    }
  })

  test('is not an email address, or anything with a letter in it', () => {
    for (const typed of ['', ' ', 'ana@example.test', '2015550123@example.test', 'a', '201555O123', '+1 201 ext 5']) {
      expect(readsAsPhone(typed), typed).toBe(false)
    }
  })
})
