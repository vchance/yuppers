import type { ExchangeView } from '@yuppers/api-client'
import { describe, expect, it } from 'vitest'

import {
  canNoteProgress,
  markRestCommand,
  PROGRESS_NOTE_MAX_CHARS,
  progressCommand,
  progressNotesOf,
  restToMarkPaid,
} from './progress'
import type { RecordEvent } from './record'
import { eventMessage, noteKind } from './record'
import { wordingFor } from './language'

describe('who can note progress', () => {
  it('is the provider, while the item is under way', () => {
    expect(canNoteProgress('PENDING', 'PROVIDER')).toBe(true)
    expect(canNoteProgress('CLAIMED', 'PROVIDER')).toBe(true)
    for (const status of ['DISPUTED', 'ACCEPTED', 'WAIVED', 'REMOVED'] as const) {
      expect(canNoteProgress(status, 'PROVIDER')).toBe(false)
    }
    expect(canNoteProgress('PENDING', 'RECIPIENT')).toBe(false)
    expect(canNoteProgress('CLAIMED', 'RECIPIENT')).toBe(false)
  })
})

describe('the command', () => {
  it('sends what was written, trimmed, as a note and nothing else', () => {
    expect(progressCommand('item', '  Posts set, panels Thursday.  ')).toEqual({
      type: 'NOTE_PROGRESS',
      contribution: 'item',
      note: 'Posts set, panels Thursday.',
    })
  })

  it('has no percentage to send', () => {
    const command = progressCommand('item', 'About done') as Record<string, unknown>
    expect(Object.keys(command).sort()).toEqual(['contribution', 'note', 'type'])
  })

  it('is nothing for a blank note or one past the limit', () => {
    expect(progressCommand('item', '   ')).toBeNull()
    expect(progressCommand('item', 'x'.repeat(PROGRESS_NOTE_MAX_CHARS))).not.toBeNull()
    expect(progressCommand('item', 'x'.repeat(PROGRESS_NOTE_MAX_CHARS + 1))).toBeNull()
  })
})

function event(over: Partial<RecordEvent> & Pick<RecordEvent, 'sequence' | 'type'>): RecordEvent {
  return { actor: 'A', at: '2026-10-10T12:00:00Z', ...over } as RecordEvent
}

describe('reading the notes from the history', () => {
  const events = [
    event({
      sequence: 1,
      type: 'PROGRESS_NOTED',
      note: 'Posts set.',
      contribution: { id: 'fence', description: 'Fence' },
    }),
    event({
      sequence: 2,
      type: 'PROGRESS_NOTED',
      note: 'Another item.',
      contribution: { id: 'gate', description: 'Gate' },
    }),
    event({ sequence: 3, type: 'CONTRIBUTION_CLAIMED', contribution: { id: 'fence', description: '' } }),
    event({
      sequence: 4,
      type: 'PROGRESS_NOTED',
      note: 'Panels Thursday.',
      contribution: { id: 'fence', description: 'Fence' },
    }),
  ]

  it('lists the notes on one item, oldest first, without other events', () => {
    expect(progressNotesOf(events, 'fence').map((note) => [note.sequence, note.text])).toEqual([
      [1, 'Posts set.'],
      [4, 'Panels Thursday.'],
    ])
    expect(progressNotesOf(events, 'nothing')).toEqual([])
  })

  it('is worded as the provider’s own statement, not a delivery', () => {
    expect(noteKind(events[0])).toBe('progress')
    const words = wordingFor('en')
    expect(words.record.noteLabels.progress).toMatch(/their own words/)
    const named = eventMessage(events[0], words.record.events, 'B', { A: 'Dana', B: 'Sam' })
    expect(named.message).toBe('{name} noted progress on this:')
    expect(named.values.name).toBe('Dana')
    const own = eventMessage(events[0], words.record.events, 'A', { A: 'Dana', B: 'Sam' })
    expect(own.message).toBe('You noted progress on this:')
  })
})

function exchange(over: Partial<ExchangeView>, items: [string, 'A' | 'B', string, string][]): ExchangeView {
  return {
    id: 'e',
    state: 'ACTIVE',
    you: 'B',
    contributions: items.map(([id, , , status]) => ({ id, status })),
    in_force_revision: {
      id: 'r',
      terms: {
        party_a_name: 'Ana',
        party_b_name: 'Ben',
        terms: '',
        contributions: items.map(([id, from, type]) => ({
          id,
          from,
          type,
          description: id,
          required: true,
        })),
      },
    },
    ...over,
  } as unknown as ExchangeView
}

describe('mark the rest as paid', () => {
  it('is offered on two or more pending payments you owe', () => {
    const view = exchange({}, [
      ['p1', 'B', 'MONEY', 'ACCEPTED'],
      ['p2', 'B', 'MONEY', 'PENDING'],
      ['p3', 'B', 'MONEY', 'PENDING'],
      ['p4', 'A', 'MONEY', 'PENDING'],
      ['job', 'A', 'SERVICE', 'PENDING'],
    ])
    expect(restToMarkPaid(view)).toEqual(['p2', 'p3'])
    expect(markRestCommand(['p2', 'p3'])).toEqual({
      type: 'CLAIM_REST',
      contributions: ['p2', 'p3'],
    })
  })

  it('is not offered for one payment, or payments already marked, or the other party’s', () => {
    expect(
      restToMarkPaid(
        exchange({}, [
          ['p1', 'B', 'MONEY', 'CLAIMED'],
          ['p2', 'B', 'MONEY', 'PENDING'],
        ]),
      ),
    ).toEqual([])
    expect(
      restToMarkPaid(
        exchange({ you: 'A' }, [
          ['p1', 'B', 'MONEY', 'PENDING'],
          ['p2', 'B', 'MONEY', 'PENDING'],
        ]),
      ),
    ).toEqual([])
  })

  it('is not offered once the yup is not under way', () => {
    expect(
      restToMarkPaid(
        exchange({ state: 'CLOSED' }, [
          ['p1', 'B', 'MONEY', 'PENDING'],
          ['p2', 'B', 'MONEY', 'PENDING'],
        ]),
      ),
    ).toEqual([])
  })
})
