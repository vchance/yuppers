// @vitest-environment jsdom
import type { ExchangeView } from '@yuppers/api-client'
import {
  ApiFailure,
  createI18n,
  wordingFor,
  type WalletApi,
  type WalletPlatform,
} from '@yuppers/shared'
import axe from 'axe-core'
import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, expect, test, vi } from 'vitest'

import { I18nContext } from '../app/context'
import { activeExchange } from '../test/fake-service'
import { WalletButton } from './WalletButton'

/*
 * The Wallet button on the exchange view (DESIGN.md §11, §13.5): the
 * device's wallet, for an agreement in force, when the service issues passes
 * for it; nothing otherwise.
 */

;(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true

interface Stand {
  platforms: WalletPlatform[]
  refuse: boolean
  asked: string[]
}

/** A stand-in for the service's Wallet calls. */
function stand(platforms: WalletPlatform[], refuse: boolean): WalletApi & Stand {
  const service: WalletApi & Stand = {
    platforms,
    refuse,
    asked: [],
    meta: async () => ({
      service: 'yuppers-backend',
      version: '0.0.0',
      commit: 'unknown',
      minimum_client_versions: {},
      push_notifications: false,
      wallet_platforms: service.platforms,
      sign_in_channels: ['email'],
      sms_country_codes: [],
      sms_updates: false,
    }),
    appleWalletLink: async (id: string) => {
      service.asked.push(`apple ${id}`)
      if (service.refuse) throw new ApiFailure('TOO_MANY_REQUESTS')
      return { url: 'https://app.test/v1/wallet/apple/pass?token=t', expires_at: null }
    },
    googleWalletLink: async (id: string) => {
      service.asked.push(`google ${id}`)
      return { url: 'https://pay.google.com/gp/v/save/jwt' }
    },
  }
  return service
}

const current = vi.hoisted(() => ({ api: null as unknown }))

// The page's client, a new one for each showing, so nothing it remembered
// stays.
vi.mock('../lib/api', () => ({
  get api() {
    return current.api
  },
}))

const wording = wordingFor('en')
const i18n = createI18n('en', wording, () => {})
let root: Root | null = null
let service = stand(['APPLE', 'GOOGLE'], false)

beforeEach(() => {
  service = stand(['APPLE', 'GOOGLE'], false)
})

async function unmount() {
  const mounted = root
  root = null
  if (mounted) await act(async () => mounted.unmount())
}

afterEach(unmount)

async function show(
  exchange: ExchangeView,
  device: WalletPlatform | null,
  open: (url: string) => void = () => {},
) {
  await unmount()
  service = stand(service.platforms, service.refuse)
  current.api = service
  document.body.innerHTML = '<main id="root"></main>'
  await act(async () => {
    root = createRoot(document.getElementById('root')!)
    root.render(
      <I18nContext.Provider value={i18n}>
        <WalletButton exchange={exchange} device={device} open={open} />
      </I18nContext.Provider>,
    )
  })
  // Let the platforms arrive.
  await act(async () => {
    await new Promise((resolve) => setTimeout(resolve, 0))
  })
}

const buttons = () => [...document.querySelectorAll('button')].map((b) => b.textContent)

async function pressTheButton() {
  await act(async () => document.querySelector('button')!.click())
}

test('an Apple device is offered Apple Wallet, and pressing it follows the link', async () => {
  const opened: string[] = []
  await show(activeExchange(), 'APPLE', (url) => opened.push(url))
  expect(buttons()).toEqual([wording.wallet.addToApple])
  expect(document.body.textContent).toContain(
    wording.wallet.intro.replace('{productName}', wording.productName),
  )
  // jsdom paints nothing, so colour contrast is `contrast.test.ts`'s.
  const checked = await axe.run(document.body, {
    rules: { 'color-contrast': { enabled: false } },
  })
  expect(checked.violations).toEqual([])

  await pressTheButton()
  expect(service.asked).toEqual([`apple ${activeExchange().id}`])
  expect(opened).toEqual(['https://app.test/v1/wallet/apple/pass?token=t'])
})

test('an Android device is offered Google Wallet', async () => {
  const opened: string[] = []
  await show(activeExchange(), 'GOOGLE', (url) => opened.push(url))
  expect(buttons()).toEqual([wording.wallet.addToGoogle])
  await pressTheButton()
  expect(opened).toEqual(['https://pay.google.com/gp/v/save/jwt'])
})

test('no button where neither wallet applies, the service has none, or nothing is in force', async () => {
  await show(activeExchange(), null)
  expect(buttons()).toEqual([])

  service.platforms = ['GOOGLE']
  await show(activeExchange(), 'APPLE')
  expect(buttons()).toEqual([])

  service.platforms = []
  await show(activeExchange(), 'GOOGLE')
  expect(buttons()).toEqual([])

  service.platforms = ['APPLE', 'GOOGLE']
  for (const state of ['NEGOTIATING', 'CLOSED'] as const) {
    await show({ ...activeExchange(), state }, 'APPLE')
    expect(buttons(), state).toEqual([])
  }
})

test('a refusal is said, and the button stays', async () => {
  service.refuse = true
  const opened: string[] = []
  await show(activeExchange(), 'APPLE', (url) => opened.push(url))
  await pressTheButton()
  expect(opened).toEqual([])
  expect(document.querySelector('.notice-error')?.textContent).toBe(
    wording.errors.TOO_MANY_REQUESTS,
  )
  expect(buttons()).toEqual([wording.wallet.addToApple])
})
