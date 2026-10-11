import { describe, expect, it } from 'vitest'

import { createI18n } from './i18n'
import { wordingFor } from './language'
import { seriesChips, seriesLine, seriesOf, type SeriesItem } from './series'

const items = (from: 'A' | 'B', type: SeriesItem['type'], statuses: SeriesItem['status'][]) =>
  statuses.map((status) => ({ from, type, status }))

describe('counting a series', () => {
  it('counts payments from one party, two or more', () => {
    const found = seriesOf([
      ...items('B', 'MONEY', ['ACCEPTED', 'CLAIMED', 'PENDING']),
      ...items('A', 'SERVICE', ['PENDING']),
    ])
    expect(found).toEqual([{ kind: 'payments', from: 'B', total: 3, confirmed: 1, disputed: 0 }])
  })

  it('leaves a single payment, and a single job, as they are', () => {
    expect(seriesOf(items('B', 'MONEY', ['ACCEPTED']))).toEqual([])
    expect(seriesOf(items('A', 'TASK', ['PENDING']))).toEqual([])
    expect(seriesOf(items('A', 'ITEM', ['PENDING', 'PENDING']))).toEqual([])
  })

  it('counts stages as services and tasks from one provider, and disputes apart', () => {
    const found = seriesOf([
      ...items('A', 'SERVICE', ['ACCEPTED']),
      ...items('A', 'TASK', ['DISPUTED', 'PENDING']),
      ...items('B', 'MONEY', ['ACCEPTED', 'ACCEPTED']),
    ])
    expect(found).toEqual([
      { kind: 'payments', from: 'B', total: 2, confirmed: 2, disputed: 0 },
      { kind: 'stages', from: 'A', total: 3, confirmed: 1, disputed: 1 },
    ])
  })

  it('does not count what an amendment removed', () => {
    expect(seriesOf(items('B', 'MONEY', ['ACCEPTED', 'REMOVED']))).toEqual([])
  })

  it('keeps each party’s series apart', () => {
    const found = seriesOf([
      ...items('A', 'MONEY', ['PENDING', 'PENDING']),
      ...items('B', 'MONEY', ['ACCEPTED', 'PENDING', 'PENDING']),
    ])
    expect(found.map((series) => [series.from, series.total])).toEqual([
      ['A', 2],
      ['B', 3],
    ])
  })
})

describe('saying it', () => {
  const i18n = (language: 'en' | 'es') => createI18n(language, wordingFor(language), () => {})

  it('writes "2 of 3 payments confirmed" with the name', () => {
    const series = { kind: 'payments' as const, from: 'B' as const, total: 3, confirmed: 2, disputed: 0 }
    expect(seriesLine(series, 'Sam', i18n('en'))).toBe('Sam: 2 of 3 payments confirmed')
    expect(seriesLine({ ...series, disputed: 1 }, 'Sam', i18n('en'))).toBe(
      'Sam: 2 of 3 payments confirmed, 1 disputed',
    )
    expect(seriesLine(series, 'Sam', i18n('es'))).toBe('Sam: 2 de 3 pagos confirmados')
  })

  it('writes stages the same way', () => {
    const series = { kind: 'stages' as const, from: 'A' as const, total: 3, confirmed: 1, disputed: 0 }
    expect(seriesLine(series, 'Dana', i18n('en'))).toBe('Dana: 1 of 3 stages confirmed')
    expect(seriesLine(series, 'Dana', i18n('es'))).toBe('Dana: 1 de 3 etapas confirmadas')
  })

  it('makes chips for the list: counts in words, never an amount', () => {
    expect(
      seriesChips(
        { payments: { total: 3, confirmed: 2, disputed: 1 }, stages: null },
        i18n('en'),
      ),
    ).toEqual(['2 of 3 payments confirmed', '1 disputed'])
    expect(seriesChips({}, i18n('en'))).toEqual([])
    expect(
      seriesChips({ stages: { total: 2, confirmed: 0, disputed: 0 } }, i18n('es')),
    ).toEqual(['0 de 2 etapas confirmadas'])
  })

  it('has no percentage, amount or bar in any of its words', () => {
    for (const language of ['en', 'es'] as const) {
      for (const text of Object.values(wordingFor(language).series)) {
        expect(text).not.toMatch(/%|\$|percent|por ciento/i)
      }
    }
  })
})
