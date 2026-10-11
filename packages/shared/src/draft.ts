import type { components, RevisionTerms } from '@yuppers/api-client'

import { fromMinorUnits, toMinorUnits } from './decimal'
import type { SplitGroup } from './instalments'

type Slot = components['schemas']['Slot']
type ContributionType = components['schemas']['ContributionType']
type ContributionDto = components['schemas']['ContributionDto']

/*
 * A working copy of terms that have not been sent: what the composer edits
 * and what the draft endpoint stores. It is looser than a revision, because
 * someone halfway through typing has not produced valid terms yet.
 * `buildTerms` turns it into the revision the API takes, or says what is
 * still missing.
 *
 * The checks here are about shape, for a quick answer next to the field. The
 * service applies the rules again and has the last word (DESIGN.md §13.3).
 */

export type DraftDue =
  | { kind: 'ON_AGREEMENT' }
  | { kind: 'DATE'; date: string }
  | { kind: 'AFTER_CONTRIBUTION'; contribution: string }

/**
 * Grey text shown in an empty field of an item to give an idea of what goes
 * there (DESIGN.md §4.4). It is never a value: a field left at its example is
 * empty, and nothing here is sent, signed or checked. It is kept in the
 * working copy so that it is still there when the person comes back.
 */
export interface ItemExample {
  description?: string
  criteria?: string
  quantity?: string
  unit?: string
}

export interface DraftContribution {
  /** Stable for the life of the exchange; chosen here for a new one. */
  id: string
  from: Slot
  type: ContributionType
  description: string
  /** A plain decimal such as `1.5`, empty for none, `null` if what was typed is not a number. */
  quantity: string | null
  unit: string
  due: DraftDue
  criteria: string
  required: boolean
  /** Money only. A plain decimal in major units, as `quantity`. */
  amount: string | null
  /** Grey examples for the empty fields, from a template. Not part of the terms. */
  example?: ItemExample
}

export interface Draft {
  format: 1
  /** The revision these terms started from, or `null` for a first proposal. */
  base: string | null
  partyA: string
  partyB: string
  terms: string
  /** A message to the other party. Not part of what is signed. */
  note: string
  contributions: DraftContribution[]
  /**
   * What the composer's split sheets made and how to put each back while
   * nothing has been sent (DESIGN.md §7.1, §7.2). Kept with the working copy,
   * never sent and never signed.
   */
  splits?: SplitGroup[]
}

/** The longest note the service accepts, in characters (DESIGN.md §6). */
export const NOTE_MAX_CHARS = 1000

export function emptyDraft(partyA: string): Draft {
  return { format: 1, base: null, partyA, partyB: '', terms: '', note: '', contributions: [] }
}

export function newContribution(id: string, from: Slot): DraftContribution {
  return {
    id,
    from,
    type: 'ITEM',
    description: '',
    quantity: '',
    unit: '',
    due: { kind: 'ON_AGREEMENT' },
    criteria: '',
    required: true,
    amount: '',
  }
}

function fromContribution(
  contribution: ContributionDto,
  fractionDigits: number,
): DraftContribution {
  return {
    id: contribution.id,
    from: contribution.from,
    type: contribution.type,
    description: contribution.description,
    quantity: contribution.quantity?.amount ?? '',
    unit: contribution.quantity?.unit ?? '',
    due: contribution.due,
    criteria: contribution.completion_criteria ?? '',
    required: contribution.required,
    amount:
      contribution.amount_minor == null
        ? ''
        : fromMinorUnits(contribution.amount_minor, fractionDigits),
  }
}

/** A working copy that starts from a revision, for a counteroffer or an amendment. */
export function draftFromTerms(terms: RevisionTerms, base: string, fractionDigits: number): Draft {
  return {
    format: 1,
    base,
    partyA: terms.party_a_name,
    partyB: terms.party_b_name,
    terms: terms.terms,
    note: '',
    contributions: terms.contributions.map((contribution) =>
      fromContribution(contribution, fractionDigits),
    ),
  }
}

/** Whether a contribution in a working copy still says what it said in `original`. */
export function isUnchanged(
  item: DraftContribution,
  original: ContributionDto,
  fractionDigits: number,
): boolean {
  const before = fromContribution(original, fractionDigits)
  const keys = Object.keys(before) as (keyof DraftContribution)[]
  return keys.every((key) => JSON.stringify(item[key]) === JSON.stringify(before[key]))
}

const TYPES: readonly ContributionType[] = ['ITEM', 'SERVICE', 'TASK', 'OTHER', 'MONEY']

function text(value: unknown): string {
  return typeof value === 'string' ? value : ''
}

function readExample(value: unknown): ItemExample | undefined {
  if (typeof value !== 'object' || value === null) return undefined
  const found = value as Record<string, unknown>
  const example: ItemExample = {}
  for (const key of ['description', 'criteria', 'quantity', 'unit'] as const) {
    if (typeof found[key] === 'string' && found[key] !== '') example[key] = found[key]
  }
  return Object.keys(example).length > 0 ? example : undefined
}

function readDue(value: unknown): DraftDue {
  const due = (value ?? {}) as Record<string, unknown>
  if (due.kind === 'DATE') return { kind: 'DATE', date: text(due.date) }
  if (due.kind === 'AFTER_CONTRIBUTION') {
    return { kind: 'AFTER_CONTRIBUTION', contribution: text(due.contribution) }
  }
  return { kind: 'ON_AGREEMENT' }
}

/**
 * Reads back a working copy the service stored. It is stored as given, and
 * may have been written by an older build, so nothing about it is assumed:
 * anything unrecognizable is `null`, and a missing piece is empty.
 */
export function readDraft(stored: unknown): Draft | null {
  if (typeof stored !== 'object' || stored === null) return null
  const draft = stored as Record<string, unknown>
  if (draft.format !== 1 || !Array.isArray(draft.contributions)) return null

  const contributions: DraftContribution[] = []
  for (const entry of draft.contributions as unknown[]) {
    if (typeof entry !== 'object' || entry === null) return null
    const item = entry as Record<string, unknown>
    if (typeof item.id !== 'string' || item.id === '') return null
    const example = readExample(item.example)
    contributions.push({
      id: item.id,
      from: item.from === 'B' ? 'B' : 'A',
      type: TYPES.find((type) => type === item.type) ?? 'ITEM',
      description: text(item.description),
      quantity: text(item.quantity),
      unit: text(item.unit),
      due: readDue(item.due),
      criteria: text(item.criteria),
      required: item.required !== false,
      amount: text(item.amount),
      ...(example ? { example } : {}),
    })
  }

  const splits = readSplits(draft.splits)
  return {
    format: 1,
    base: typeof draft.base === 'string' ? draft.base : null,
    partyA: text(draft.partyA),
    partyB: text(draft.partyB),
    terms: text(draft.terms),
    note: text(draft.note),
    contributions,
    ...(splits.length > 0 ? { splits } : {}),
  }
}

function readItem(entry: unknown): DraftContribution | null {
  if (typeof entry !== 'object' || entry === null) return null
  const item = entry as Record<string, unknown>
  if (typeof item.id !== 'string' || item.id === '') return null
  return {
    id: item.id,
    from: item.from === 'B' ? 'B' : 'A',
    type: TYPES.find((type) => type === item.type) ?? 'ITEM',
    description: text(item.description),
    quantity: text(item.quantity),
    unit: text(item.unit),
    due: readDue(item.due),
    criteria: text(item.criteria),
    required: item.required !== false,
    amount: text(item.amount),
  }
}

/** The split groups a stored working copy remembers; whatever is unreadable is forgotten. */
function readSplits(stored: unknown): SplitGroup[] {
  if (!Array.isArray(stored)) return []
  const groups: SplitGroup[] = []
  for (const entry of stored as unknown[]) {
    if (typeof entry !== 'object' || entry === null) continue
    const group = entry as Record<string, unknown>
    const original = readItem(group.original)
    if (
      !original ||
      (group.kind !== 'INSTALMENTS' && group.kind !== 'STAGES') ||
      !Array.isArray(group.ids) ||
      !group.ids.every((id) => typeof id === 'string')
    ) {
      continue
    }
    groups.push({ kind: group.kind, original, ids: group.ids as string[] })
  }
  return groups
}

export type ProblemCode =
  | 'PARTY_NAME_MISSING'
  | 'NO_CONTRIBUTIONS'
  | 'NO_REQUIRED_CONTRIBUTION'
  | 'NOTE_TOO_LONG'
  | 'DESCRIPTION_MISSING'
  | 'QUANTITY_INVALID'
  | 'AMOUNT_MISSING'
  | 'AMOUNT_INVALID'
  | 'DATE_MISSING'
  | 'DEPENDENCY_MISSING'
  | 'DEPENDENCY_CYCLE'

export type ProblemField =
  | 'partyA'
  | 'partyB'
  | 'note'
  | 'contributions'
  | 'description'
  | 'quantity'
  | 'amount'
  | 'date'
  | 'after'

export interface Problem {
  code: ProblemCode
  field: ProblemField
  /** The contribution the problem is in, when it is in one. */
  contribution?: string
}

export type Built =
  { ok: true; terms: RevisionTerms; note: string | null } | { ok: false; problems: Problem[] }

function isCalendarDate(value: string): boolean {
  const match = /^(\d{4})-(\d{2})-(\d{2})$/.exec(value)
  if (!match) return false
  const [year, month, day] = [Number(match[1]), Number(match[2]), Number(match[3])]
  const date = new Date(Date.UTC(year, month - 1, day))
  return (
    date.getUTCFullYear() === year && date.getUTCMonth() === month - 1 && date.getUTCDate() === day
  )
}

/** Whether following the "due after" links from `start` comes back round. */
function waitsOnItself(start: DraftContribution, all: readonly DraftContribution[]): boolean {
  const visited = new Set([start.id])
  let current = start
  while (current.due.kind === 'AFTER_CONTRIBUTION') {
    const next = current.due.contribution
    const target = all.find((contribution) => contribution.id === next)
    if (!target) return false
    if (next === start.id) return true
    // A circle further along that this one is not part of is reported there.
    if (visited.has(next)) return false
    visited.add(next)
    current = target
  }
  return false
}

/**
 * Turns a working copy into the terms to sign, or lists what has to be fixed
 * first. `fractionDigits` is how many decimal places the exchange's currency
 * has.
 *
 * `base` is the revision the working copy started from, if any. A
 * contribution the person did not touch is sent exactly as that revision has
 * it, character for character: an amendment resets the status of any
 * contribution whose terms differ at all, and refuses to alter an accepted
 * one (DESIGN.md §7).
 */
export function buildTerms(draft: Draft, fractionDigits: number, base?: RevisionTerms): Built {
  const problems: Problem[] = []
  const partyA = draft.partyA.trim()
  const partyB = draft.partyB.trim()
  const note = draft.note.trim()

  if (partyA === '') problems.push({ code: 'PARTY_NAME_MISSING', field: 'partyA' })
  if (partyB === '') problems.push({ code: 'PARTY_NAME_MISSING', field: 'partyB' })
  if ([...note].length > NOTE_MAX_CHARS) problems.push({ code: 'NOTE_TOO_LONG', field: 'note' })
  if (draft.contributions.length === 0) {
    problems.push({ code: 'NO_CONTRIBUTIONS', field: 'contributions' })
  } else if (!draft.contributions.some((contribution) => contribution.required)) {
    problems.push({ code: 'NO_REQUIRED_CONTRIBUTION', field: 'contributions' })
  }

  const contributions: ContributionDto[] = []
  for (const item of draft.contributions) {
    const problem = (code: ProblemCode, field: ProblemField) =>
      problems.push({ code, field, contribution: item.id })

    // Checked even for an untouched contribution: what it waits on may have
    // been removed or re-pointed around it.
    let due: ContributionDto['due'] = { kind: 'ON_AGREEMENT' }
    if (item.due.kind === 'DATE') {
      if (isCalendarDate(item.due.date)) due = item.due
      else problem('DATE_MISSING', 'date')
    } else if (item.due.kind === 'AFTER_CONTRIBUTION') {
      const target = item.due.contribution
      const known = draft.contributions.some((other) => other.id === target)
      if (!known || target === item.id) problem('DEPENDENCY_MISSING', 'after')
      else if (waitsOnItself(item, draft.contributions)) problem('DEPENDENCY_CYCLE', 'after')
      else due = item.due
    }

    const original = base?.contributions.find((contribution) => contribution.id === item.id)
    if (original && isUnchanged(item, original, fractionDigits)) {
      contributions.push(original)
      continue
    }

    const description = item.description.trim()
    if (description === '') problem('DESCRIPTION_MISSING', 'description')

    // Only money has an amount, and it stands in place of a quantity.
    let amountMinor: number | null = null
    let quantity: ContributionDto['quantity'] = null
    if (item.type === 'MONEY') {
      if (item.amount === '') problem('AMOUNT_MISSING', 'amount')
      else {
        amountMinor = item.amount === null ? null : toMinorUnits(item.amount, fractionDigits)
        if (amountMinor === null) problem('AMOUNT_INVALID', 'amount')
      }
    } else {
      const unit = item.unit.trim()
      if (item.quantity === null || (item.quantity === '' && unit !== '')) {
        problem('QUANTITY_INVALID', 'quantity')
      } else if (item.quantity !== '') {
        quantity = { amount: item.quantity, unit: unit === '' ? null : unit }
      }
    }

    const criteria = item.criteria.trim()
    contributions.push({
      id: item.id,
      from: item.from,
      type: item.type,
      description,
      quantity,
      due,
      completion_criteria: criteria === '' ? null : criteria,
      required: item.required,
      amount_minor: amountMinor,
    })
  }

  if (problems.length > 0) return { ok: false, problems }
  return {
    ok: true,
    terms: { party_a_name: partyA, party_b_name: partyB, terms: draft.terms, contributions },
    note: note === '' ? null : note,
  }
}
