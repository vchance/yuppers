import type { components, ExchangeView } from '@yuppers/api-client'

import type { RevisionView, SendRevision } from './api'
import { isUnconfirmedClaimant } from './claimant'
import { consentShown } from './consent'
import { decimalForInput } from './decimal'
import {
  draftFromTerms,
  emptyDraft,
  NOTE_MAX_CHARS,
  readDraft,
  type Built,
  type Draft,
  type DraftDue,
  type Problem,
} from './draft'
import { liveGroups } from './instalments'
import { formatMessage } from './message'
import { invitationOptions } from './share'
import type { Wording } from './wording/types'

type ContributionType = components['schemas']['ContributionType']

/*
 * What the composer does that is not a screen: where a working copy starts,
 * saving it as it is typed, saying what is wrong with it, and turning it into
 * the revision to send. The web app and the mobile app lay the composer out
 * in their own components and share this.
 */

/** The kinds of contribution, in the order a composer offers them. */
export const CONTRIBUTION_TYPES: readonly ContributionType[] = [
  'ITEM',
  'SERVICE',
  'TASK',
  'MONEY',
  'OTHER',
]

/** A first proposal on a draft, a counteroffer during negotiation, or an amendment to an agreement. */
export type ComposerKind = 'first' | 'counter' | 'amend'

export function composerKind(exchange: ExchangeView): ComposerKind {
  return exchange.state === 'DRAFT' ? 'first' : exchange.state === 'ACTIVE' ? 'amend' : 'counter'
}

/** What a counteroffer or an amendment starts from: the terms on the table. */
export function baseRevision(exchange: ExchangeView): RevisionView | null {
  return exchange.open_revision ?? exchange.in_force_revision ?? null
}

/**
 * Whether there is anything to compose: not on a closed exchange, nor one
 * with no terms to start from, nor by a claimant the initiator has not
 * confirmed, who cannot propose changes yet.
 */
export function canCompose(exchange: ExchangeView): boolean {
  if (exchange.state === 'CLOSED' || isUnconfirmedClaimant(exchange)) return false
  return exchange.state === 'DRAFT' || baseRevision(exchange) !== null
}

/**
 * The working copy a composer opens with: the one the service stored, or
 * else the terms on the table, or else nothing but the author's own name.
 */
export function startingDraft(
  exchange: ExchangeView,
  authorName: string,
  fractionDigits: number,
): Draft {
  const stored = readDraft(exchange.draft)
  if (stored) return stored
  const base = baseRevision(exchange)
  return base ? draftFromTerms(base.terms, base.id, fractionDigits) : emptyDraft(authorName)
}

/** An accepted contribution is locked: an amendment may not touch it (DESIGN.md §7). */
export function lockedContributions(exchange: ExchangeView): Set<string> {
  return new Set(
    exchange.state === 'ACTIVE'
      ? exchange.contributions.filter((item) => item.status === 'ACCEPTED').map((item) => item.id)
      : [],
  )
}

/** A due condition of the given kind with nothing chosen yet. */
export function dueOf(kind: DraftDue['kind']): DraftDue {
  if (kind === 'DATE') return { kind, date: '' }
  if (kind === 'AFTER_CONTRIBUTION') return { kind, contribution: '' }
  return { kind: 'ON_AGREEMENT' }
}

/** What to tell the person about something `buildTerms` found wrong. */
export function problemText(problem: Problem, wording: Wording, language: string): string {
  const message = wording.composer.problems[problem.code]
  if (problem.code === 'NOTE_TOO_LONG') {
    return formatMessage(message, { max: NOTE_MAX_CHARS }, language)
  }
  if (problem.code === 'QUANTITY_INVALID') {
    return formatMessage(message, { example: decimalForInput('1.5', language) }, language)
  }
  if (problem.code === 'AMOUNT_INVALID') {
    return formatMessage(message, { example: decimalForInput('25.50', language) }, language)
  }
  return message
}

/**
 * The request that sends, and so signs, the terms built from a working copy
 * (DESIGN.md §6). It names the version of the exchange the author was looking
 * at and the consent wording they were shown. Only a first proposal issues an
 * invitation, which `boundTo` may name a person for.
 */
export function revisionToSend(
  exchange: ExchangeView,
  built: Extract<Built, { ok: true }>,
  language: string,
  boundTo: string,
  splits?: SplitsUsed,
): SendRevision {
  return {
    expected_version: exchange.version,
    terms: built.terms,
    note: built.note,
    consent: consentShown(language),
    invitation: composerKind(exchange) === 'first' ? invitationOptions(boundTo) : null,
    // Counted for the product's measures (DESIGN.md §7.1, §7.2): how many
    // split sheets this was written with, and nothing about what they made.
    ...(splits && (splits.instalments > 0 || splits.stages > 0) ? { splits } : {}),
  }
}

/** How many split sheets a working copy was written with and has not put back. */
export interface SplitsUsed {
  instalments: number
  stages: number
}

export function splitsUsed(draft: Draft): SplitsUsed {
  // A split whose items were all removed since is not one the terms were written with.
  const groups = liveGroups(draft.contributions, draft.splits ?? [])
  return {
    instalments: groups.filter((group) => group.kind === 'INSTALMENTS').length,
    stages: groups.filter((group) => group.kind === 'STAGES').length,
  }
}

export type SaveState = 'idle' | 'saving' | 'saved' | 'failed'

/** How long after the last keystroke the working copy is saved. */
export const SAVE_AFTER_MS = 700

export interface DraftSaver {
  /** A newer working copy. It is saved once the typing pauses. */
  changed(draft: Draft): void
  /**
   * Before sending: stops any save that is waiting and resolves once none is
   * on its way. The service drops the working copy when a revision is sent,
   * and a save still in flight must not put it back afterwards.
   */
  settle(): Promise<void>
  /** Sending failed and nothing was sent: the working copy still needs saving. */
  resume(): void
  /** The revision was sent. Nothing is saved after this. */
  sent(): void
  /** Leaving the composer: saves what was typed in the last moment. */
  leave(): void
}

interface DraftSaverOptions {
  save(draft: Draft): Promise<void>
  onState(state: SaveState): void
  delayMs?: number
}

/**
 * Saves a working copy as it is typed. One save at a time, so an older copy
 * can never land after a newer one; a failed save is tried again with the
 * next change.
 */
export function createDraftSaver({
  save,
  onState,
  delayMs = SAVE_AFTER_MS,
}: DraftSaverOptions): DraftSaver {
  let latest: Draft | null = null
  let dirty = false
  let isSent = false
  let timer: ReturnType<typeof setTimeout> | undefined
  let saving: Promise<void> | null = null

  function schedule() {
    clearTimeout(timer)
    timer = setTimeout(() => void run(), delayMs)
  }

  async function run() {
    if (saving || !dirty || isSent || !latest) return
    dirty = false
    onState('saving')
    let saved = true
    saving = save(latest).catch(() => {
      saved = false
    })
    await saving
    saving = null
    if (!saved) {
      dirty = true
      onState('failed')
    } else if (dirty) schedule()
    else onState('saved')
  }

  return {
    changed(draft) {
      latest = draft
      dirty = true
      schedule()
    },
    async settle() {
      clearTimeout(timer)
      dirty = false
      await saving
    },
    resume() {
      dirty = true
    },
    sent() {
      isSent = true
    },
    leave() {
      clearTimeout(timer)
      if (dirty && !isSent && latest) save(latest).catch(() => {})
    },
  }
}
