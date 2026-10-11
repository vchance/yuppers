import type { ExchangeView } from '@yuppers/api-client'
import { describe, expect, test, vi } from 'vitest'

import { emptyDraft } from './draft'
import { beginYup, draftChanged, pendingStart, templateStartedFrom } from './start'
import { TEMPLATES } from './templates'

const made = { id: '11111111-1111-4111-8111-111111111111' } as ExchangeView

function service() {
  return {
    createExchange: vi.fn(async () => made),
    saveDraft: vi.fn(async () => undefined),
  }
}

describe('a fresh start', () => {
  test('makes nothing on the service until it is asked to', async () => {
    const api = service()
    const template = TEMPLATES[0]
    const start = pendingStart(api, 'America/Chicago', { kind: 'template', template }, null)
    expect(api.createExchange).not.toHaveBeenCalled()
    expect(start.exchange.state).toBe('DRAFT')
    expect(start.exchange.timezone).toBe('America/Chicago')
    expect(start.template).toBe(template)
    expect(start.created()).toBeNull()

    // Asked twice, made once, with how it began; the band is remembered.
    const [first, second] = await Promise.all([start.ensure(), start.ensure()])
    expect(first).toBe(second)
    expect(api.createExchange).toHaveBeenCalledTimes(1)
    expect(api.createExchange).toHaveBeenCalledWith('America/Chicago', 'job-deposit-balance@1')
    expect(start.created()).toBe(made)
    expect(templateStartedFrom(made.id)).toBe(template)
  })

  test('tries again after a failure', async () => {
    const api = service()
    api.createExchange.mockRejectedValueOnce(new Error('down'))
    const start = pendingStart(api, 'UTC', { kind: 'blank' }, null)
    await expect(start.ensure()).rejects.toThrow('down')
    await expect(start.ensure()).resolves.toBe(made)
    expect(api.createExchange).toHaveBeenCalledTimes(2)
    expect(api.createExchange).toHaveBeenLastCalledWith('UTC', 'blank')
  })

  test('counts a change from the starting copy, and nothing else', () => {
    const copy = emptyDraft('Ana')
    expect(draftChanged(copy, { ...copy })).toBe(false)
    expect(draftChanged(copy, { ...copy, partyB: 'Bruno' })).toBe(true)
    expect(draftChanged(copy, { ...copy, terms: 'x' })).toBe(true)
  })
})

describe('copying an earlier yup', () => {
  test('makes the draft at once and saves the copy into it', async () => {
    const api = service()
    const copy = emptyDraft('Ana')
    await beginYup(api, 'UTC', { kind: 'copy' }, copy)
    expect(api.createExchange).toHaveBeenCalledWith('UTC', 'copy')
    expect(api.saveDraft).toHaveBeenCalledWith(made.id, copy)
  })
})
