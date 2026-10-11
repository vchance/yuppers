import type { ExchangeView } from '@yuppers/api-client'

import type { ExchangeApi } from './api'
import type { Draft } from './draft'
import { startedFrom, templateById, type StartChoice, type Template } from './templates'

/*
 * Starting from a choice in the chooser (DESIGN.md §4.4). The draft is made
 * on the service first, as it always was, and then the starting working copy
 * is saved into it, so that leaving and coming back finds the draft and not
 * the chooser.
 */

/** The template an exchange was started from, in this session only, for the band above its items. */
const templatesStarted = new Map<string, string>()

/**
 * What the screen needs to start: the service, this device's time zone, the
 * choice, and the working copy it makes (none for the blank form).
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
