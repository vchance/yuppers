import type { ExchangeView } from '@yuppers/api-client'

import type { ExchangeApi } from './api'
import type { Draft } from './draft'
import { startedFrom, templateById, type StartChoice, type Template } from './templates'

/*
 * Starting from a choice in the chooser (DESIGN.md §4.4). A blank form or a
 * template opens the composer with a working copy kept on the device and
 * nothing on the service: the draft is made, with how it began, the first
 * time the person changes anything from that copy, so that backing out of an
 * untouched composer leaves no empty draft behind. Copying an earlier yup
 * carries content the person chose, so that draft is made at once.
 */

/** The template an exchange was started from, in this session only, for the band above its items. */
const templatesStarted = new Map<string, string>()

/**
 * Starts at once: makes the draft on the service and saves the working copy
 * the choice made into it. For copying an earlier yup. What the screen needs:
 * the service, this device's time zone, the choice, and the working copy it
 * makes (none for the blank form).
 */
export async function beginYup(
  api: Pick<ExchangeApi, 'createExchange' | 'saveDraft'>,
  timezone: string,
  choice: StartChoice,
  draft: Draft | null,
): Promise<ExchangeView> {
  const exchange = await api.createExchange(timezone, startedFrom(choice))
  if (draft) {
    // A draft that could not be filled in is still a draft the person can
    // write: they find the composer, not an error about a starting point.
    try {
      await api.saveDraft(exchange.id, draft)
    } catch {
      // Nothing to do; the composer starts from what the service has.
    }
  }
  if (choice.kind === 'template') templatesStarted.set(exchange.id, choice.template.id)
  else templatesStarted.delete(exchange.id)
  return exchange
}

/** The template the draft was started from, if this session started it from one. */
export function templateStartedFrom(exchangeId: string): Template | undefined {
  const id = templatesStarted.get(exchangeId)
  return id === undefined ? undefined : templateById(id)
}

/** A fresh start, before the service has an exchange for it. */
export interface PendingStart {
  /** A stand-in for the exchange: a draft of the person's own, with no id yet. */
  readonly exchange: ExchangeView
  /** The template the start is from, for the band above the items. */
  readonly template: Template | undefined
  /** The starting working copy, or null for the blank form. */
  readonly draft: Draft | null
  /** The exchange, once it has been made. */
  created(): ExchangeView | null
  /**
   * Makes the draft on the service, once, telling it how the yup began.
   * Asking again gives the same exchange; a failure is not remembered, so
   * the next change tries again.
   */
  ensure(): Promise<ExchangeView>
}

/** A start that makes nothing on the service until `ensure` is called. */
export function pendingStart(
  api: Pick<ExchangeApi, 'createExchange'>,
  timezone: string,
  choice: StartChoice,
  draft: Draft | null,
): PendingStart {
  let made: ExchangeView | null = null
  let making: Promise<ExchangeView> | null = null
  const template = choice.kind === 'template' ? choice.template : undefined
  return {
    exchange: {
      id: '',
      version: 0,
      state: 'DRAFT',
      you: 'A',
      counterparty: 'UNCLAIMED',
      display_code: '',
      currency: 'USD',
      timezone,
      contributions: [],
    },
    template,
    draft,
    created: () => made,
    ensure() {
      making ??= api.createExchange(timezone, startedFrom(choice)).then(
        (exchange) => {
          made = exchange
          if (template) templatesStarted.set(exchange.id, template.id)
          return exchange
        },
        (error: unknown) => {
          making = null
          throw error
        },
      )
      return making
    },
  }
}

/**
 * Whether the working copy differs from the one the start made. A template's
 * grey examples are not in the copy, so they never count.
 */
export function draftChanged(start: Draft, now: Draft): boolean {
  return JSON.stringify(start) !== JSON.stringify(now)
}
