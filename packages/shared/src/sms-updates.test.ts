/// <reference types="node" />
import { readFileSync } from 'node:fs'

import { describe, expect, test } from 'vitest'

import en from '../wording/en.json'
import es from '../wording/es.json'
import termsEn from '../wording/terms/en.json'
import termsEs from '../wording/terms/es.json'
import { legalInline, type LegalWording } from './legal'
import { consentPieces, maskPhone, SMS_CONSENT_VERSION, usPhone } from './sms-updates'

/*
 * Text updates for an agreement: the number a person types, how it is
 * shown, and the consent wording, which must be the terms' quote word for
 * word in every language.
 */

describe('the consent wording', () => {
  const quoted = (terms: LegalWording, language: string) =>
    terms.sections['text-messages'].blocks
      .flatMap((block) => ('ul' in block ? block.ul : 'p' in block ? [block.p] : []))
      .map((message) =>
        legalInline(message, language, 'terms', terms)
          .map((piece) => ('email' in piece ? piece.email : piece.text))
          .join(''),
      )
      .join('\n')

  test.each([
    ['en', en, termsEn],
    ['es', es, termsEs],
  ] as const)('in %s is the one the terms quote', (language, wording, terms) => {
    expect(quoted(terms as LegalWording, language)).toContain(`“${wording.smsUpdates.consent}”`)
  })

  test('is the owner’s, word for word', () => {
    expect(en.smsUpdates.consent).toBe(
      'Receive text updates from yuppers.app about this agreement, one text per status change. Message frequency varies; there is no fixed maximum. Msg & data rates may apply. Reply HELP for help or STOP to opt out. Terms: https://yuppers.app/terms. Privacy Policy: https://yuppers.app/privacy.',
    )
  })

  test('has its two addresses as links that read as the addresses', () => {
    expect(consentPieces(en.smsUpdates.consent).filter((piece) => 'url' in piece)).toEqual([
      { url: 'https://yuppers.app/terms' },
      { url: 'https://yuppers.app/privacy' },
    ])
    const pieces = consentPieces(es.smsUpdates.consent)
    expect(pieces.map((piece) => ('url' in piece ? piece.url : piece.text)).join('')).toBe(
      es.smsUpdates.consent,
    )
    expect(pieces.at(-1)).toEqual({ text: '.' })
  })

  test('has the version the service checks', () => {
    const backend = readFileSync(
      new URL('../../../backend/src/notifications/sms_updates.rs', import.meta.url),
      'utf8',
    )
    expect(backend).toContain(`pub const CONSENT_VERSION: &str = "${SMS_CONSENT_VERSION}";`)
  })
})

describe('a US number', () => {
  test('is taken as typed, in any usual shape', () => {
    for (const typed of [
      '5552345678',
      '555 234 5678',
      '(555) 234-5678',
      '555.234.5678',
      '+1 555 234 5678',
      '+15552345678',
      '1-555-234-5678',
    ]) {
      expect(usPhone(typed), typed).toBe('+15552345678')
    }
  })

  test('is refused when it is not one', () => {
    for (const typed of [
      '',
      '234 5678',
      '+44 7700 900123',
      '+525512345678',
      '055 234 5678',
      '555 134 5678',
      'call me',
      'ana@example.test',
    ]) {
      expect(usPhone(typed), typed).toBeNull()
    }
  })

  test('is shown with all but its last four digits hidden', () => {
    expect(maskPhone('+15552345678')).toBe('+1 •••-•••-5678')
    expect(maskPhone('+447700900123')).toBe('•••0123')
  })
})
