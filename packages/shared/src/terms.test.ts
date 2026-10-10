/// <reference types="node" />
import { readFileSync } from 'node:fs'

import { expect, test } from 'vitest'

import en from '../wording/en.json'
import es from '../wording/es.json'
import { LEGAL_EFFECTIVE_DATES } from './legal-text'
import { TERMS_VERSION, termsAssentPieces } from './terms'

test('the version is the day both documents took effect, and the service knows it', () => {
  expect(LEGAL_EFFECTIVE_DATES).toEqual({ privacy: TERMS_VERSION, terms: TERMS_VERSION })
  const backend = readFileSync(new URL('../../../backend/src/terms.rs', import.meta.url), 'utf8')
  expect(backend).toContain(`pub const TERMS_VERSION: &str = "${TERMS_VERSION}";`)
})

test('the sentence names both documents once, in each language', () => {
  for (const [wording, names] of [
    [en, { terms: en.termsOfUse.link, privacy: en.privacy.policy }],
    [es, { terms: es.termsOfUse.link, privacy: es.privacy.policy }],
  ] as const) {
    const pieces = termsAssentPieces(wording.signIn.agreement, names)
    const linked = pieces.filter((piece) => 'document' in piece)
    expect(linked.map((piece) => ('document' in piece ? piece.document : null))).toEqual([
      'terms',
      'privacy',
    ])
    expect(pieces.map((piece) => piece.text).join('')).not.toContain('{')
  }
  expect(
    termsAssentPieces(en.signIn.agreement, { terms: 'Terms', privacy: 'Privacy policy' })
      .map((piece) => piece.text)
      .join(''),
  ).toBe('By continuing you agree to the Terms and the Privacy policy.')
})
