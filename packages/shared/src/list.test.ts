import type { ExchangeSummary } from '@yuppers/api-client'
import { expect, test } from 'vitest'

import { groupExchanges } from './list'

function summary(id: string, state: ExchangeSummary['state'], closed?: ExchangeSummary['closed_outcome']): ExchangeSummary {
  return {
    id,
    display_code: id.toUpperCase(),
    state,
    closed_outcome: closed ?? null,
    you: 'A',
    other_party_name: '',
    updated_at: '2026-10-02T12:00:00Z',
    counterparty: state === 'DRAFT' ? 'UNCLAIMED' : 'CONFIRMED',
  }
}

test('what is in progress comes first, drafts next, and closed exchanges are kept apart', () => {
  const listed = [
    summary('done', 'CLOSED', 'COMPLETED'),
    summary('offer', 'NEGOTIATING'),
    summary('draft', 'DRAFT'),
    summary('dead', 'CLOSED', 'NOT_AGREED'),
    summary('work', 'ACTIVE'),
  ]
  const groups = groupExchanges(listed)
  expect(groups.open.map((e) => e.id)).toEqual(['offer', 'work'])
  expect(groups.drafts.map((e) => e.id)).toEqual(['draft'])
  expect(groups.closed.map((e) => e.id)).toEqual(['done', 'dead'])
  // The service's order, most recently changed first, is kept within each group.
  expect([...groups.open, ...groups.drafts, ...groups.closed]).toHaveLength(listed.length)
})

test('an empty list has empty groups', () => {
  expect(groupExchanges([])).toEqual({ open: [], drafts: [], closed: [] })
})
