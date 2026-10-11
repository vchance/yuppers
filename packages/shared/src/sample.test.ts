import { describe, expect, test } from 'vitest'

import { buildTerms, draftFromTerms } from './draft'
import { languages, wordingFor } from './language'
import { SAMPLE_IDS, sampleYup } from './sample'
import { summarizeRecord } from './summary'

// 12:00 UTC, so that in every zone it is the same calendar day or the next
// one, and the dates below are worked out from it in the zone asked for.
const NOW = new Date('2026-10-10T16:00:00Z')

describe('the sample yup', () => {
  for (const { code } of languages) {
    test(`reads as a current record and as terms that could be sent, in ${code}`, () => {
      const sample = sampleYup(NOW, 'UTC', wordingFor(code).sample, code)
      // The same terms the composer would build from them: nothing the service
      // would refuse.
      const draft = draftFromTerms(sample.terms, '', 2)
      expect(buildTerms(draft, 2).ok).toBe(true)

      expect(sample.record.format).toBe('exchange-record')
      expect(sample.record.format_version).toBe(1)
      expect(sample.record.language).toBe(code)
      const summary = summarizeRecord(sample.record)
      expect(summary.basis).toEqual({ kind: 'AGREEMENT', sequence: 1 })
      expect(summary.signatures.map((signature) => signature.party)).toEqual(['A', 'B'])
      expect(summary.items.map((item) => item.id)).toEqual([
        SAMPLE_IDS.deposit,
        SAMPLE_IDS.repair,
        SAMPLE_IDS.balance,
      ])
    })
  }

  test('names the same two people and the same deal in both languages, in dollars', () => {
    const en = sampleYup(NOW, 'UTC', wordingFor('en').sample, 'en')
    const es = sampleYup(NOW, 'UTC', wordingFor('es').sample, 'es')
    expect([es.terms.party_a_name, es.terms.party_b_name]).toEqual([
      en.terms.party_a_name,
      en.terms.party_b_name,
    ])
    expect(en.terms.party_a_name).toBe('Dana Reyes')
    expect(en.terms.party_b_name).toBe('Sam Okafor')
    for (const sample of [en, es]) {
      expect(sample.currency).toBe('USD')
      expect(sample.terms.contributions.map((item) => item.amount_minor)).toEqual([
        10000,
        null,
        30000,
      ])
      expect(sample.terms.contributions.map((item) => [item.from, item.type])).toEqual([
        ['B', 'MONEY'],
        ['A', 'SERVICE'],
        ['B', 'MONEY'],
      ])
      // The balance waits for the repair; the deposit is due when it is signed.
      expect(sample.terms.contributions[0].due).toEqual({ kind: 'ON_AGREEMENT' })
      expect(sample.terms.contributions[2].due).toEqual({
        kind: 'AFTER_CONTRIBUTION',
        contribution: SAMPLE_IDS.repair,
      })
      expect(sample.statuses.get(SAMPLE_IDS.deposit)).toBe('ACCEPTED')
      expect(sample.statuses.get(SAMPLE_IDS.repair)).toBe('PENDING')
      expect(sample.statuses.get(SAMPLE_IDS.balance)).toBe('PENDING')
      expect(sample.events).toHaveLength(6)
      expect(sample.events.map((event) => event.type)).toEqual([
        'REVISION_SENT',
        'COUNTERPARTY_CLAIMED',
        'REVISION_ACCEPTED',
        'AGREEMENT_IN_FORCE',
        'CONTRIBUTION_CLAIMED',
        'CONTRIBUTION_CONFIRMED',
      ])
    }
  })

  test('has its dates worked out from today: signed three days ago, the repair due in six days', () => {
    const sample = sampleYup(NOW, 'UTC', wordingFor('en').sample, 'en')
    expect(sample.repairDue).toBe('2026-10-16')
    expect(sample.terms.contributions[1].due).toEqual({ kind: 'DATE', date: '2026-10-16' })
    expect(sample.events[0].at.slice(0, 10)).toBe('2026-10-07')
    expect(sample.events[3].at.slice(0, 10)).toBe('2026-10-07')

    const later = sampleYup(new Date('2026-12-30T10:00:00Z'), 'UTC', wordingFor('en').sample, 'en')
    expect(later.repairDue).toBe('2027-01-05')
    expect(later.events[0].at.slice(0, 10)).toBe('2026-12-27')
  })

  test('is not anyone’s: no reference code, no invitation, no fingerprint of its own', () => {
    const sample = sampleYup(NOW, 'UTC', wordingFor('en').sample, 'en')
    expect(sample.record.exchange.display_code).toBe('')
    expect(sample.record.revisions[0].content_hash).toBe('')
    expect(JSON.stringify(sample.record)).not.toMatch(/token|https?:|@/)
  })

  test('says in both languages that it is an example, and why its actions do nothing', () => {
    for (const code of ['en', 'es'] as const) {
      const words = wordingFor(code).sample
      expect(words.banner).toMatch(code === 'en' ? /^Example\. /i : /^Ejemplo\. /i)
      expect(words.actionsOff).not.toBe('')
      expect(words.wouldTap).toContain('{name}')
      expect(words.fingerprint).not.toBe('')
    }
    expect(wordingFor('en').sample.banner).toBe(
      'Example. These people aren’t real, nothing here is saved, and nothing is signed by you.',
    )
  })
})
