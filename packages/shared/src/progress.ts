import type { Command, components, ExchangeView } from '@yuppers/api-client'

import { NOTE_MAX_CHARS } from './draft'
import { statusesOf, type Role } from './fulfillment'
import type { RecordEvent } from './record'

type Schemas = components['schemas']
type Slot = Schemas['Slot']
type Status = Schemas['Status']

/*
 * Progress notes (DESIGN.md §7.2) and "Mark the rest as paid" (DESIGN.md
 * §7.1): the two commands that are about items rather than about one status
 * change.
 *
 * A progress note is the provider's own statement on an item that is under
 * way. It is not a claim and changes no status; the other person has nothing
 * to confirm. There is no percentage: how far along something is, is how many
 * of its stages are confirmed.
 */

/** The longest progress note, in characters: the same as any note (DESIGN.md §7.2). */
export const PROGRESS_NOTE_MAX_CHARS = NOTE_MAX_CHARS
/** Progress notes one item carries at most. A placeholder from DESIGN.md §7.2; the service holds the limit. */
export const PROGRESS_NOTES_PER_ITEM = 20

/** Whether a progress note can be added to an item in this status by this role. */
export function canNoteProgress(status: Status, role: Role): boolean {
  return role === 'PROVIDER' && (status === 'PENDING' || status === 'CLAIMED')
}

/** The command that adds a progress note, or `null` when there is nothing written. */
export function progressCommand(contribution: string, written: string): Command | null {
  const note = written.trim()
  if (note === '' || [...note].length > PROGRESS_NOTE_MAX_CHARS) return null
  return { type: 'NOTE_PROGRESS', contribution, note }
}

/** One progress note as the history has it. */
export interface ProgressNote {
  /** The event's position in the history. */
  sequence: number
  by: Slot
  at: string
  text: string
}

/** The progress notes on one item, oldest first, from the events read so far. */
export function progressNotesOf(
  events: readonly RecordEvent[],
  contribution: string,
): ProgressNote[] {
  return events.flatMap((event): ProgressNote[] => {
    if (
      event.type !== 'PROGRESS_NOTED' ||
      event.contribution?.id !== contribution ||
      event.actor === 'SYSTEM' ||
      !event.note
    ) {
      return []
    }
    return [{ sequence: event.sequence, by: event.actor, at: event.at, text: event.note }]
  })
}

/**
 * The payments "Mark the rest as paid" would record for `you`: money items
 * of the agreement in force that you owe and have not yet marked paid, in the
 * order the agreement lists them. Offered when there are two or more
 * (DESIGN.md §7.1; the recommendation in section 18, item 44).
 */
export function restToMarkPaid(exchange: ExchangeView): string[] {
  if (exchange.state !== 'ACTIVE') return []
  const inForce = exchange.in_force_revision
  if (!inForce) return []
  const statuses = statusesOf(exchange)
  const pending = inForce.terms.contributions.filter(
    (item) => item.type === 'MONEY' && item.from === exchange.you && statuses.get(item.id) === 'PENDING',
  )
  return pending.length >= 2 ? pending.map((item) => item.id) : []
}

/** The command that marks each of these payments as paid, all or none. */
export function markRestCommand(contributions: readonly string[]): Command {
  return { type: 'CLAIM_REST', contributions: [...contributions] }
}
