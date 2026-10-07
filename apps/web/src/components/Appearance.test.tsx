// @vitest-environment jsdom
import { createI18n, wordingFor } from '@yuppers/shared'
import axe from 'axe-core'
import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, expect, test } from 'vitest'

import { I18nContext } from '../app/context'
import { Appearance, FooterAppearance } from './Appearance'

/*
 * The appearance switch: System, Light or Dark for this device, applied at
 * once as `data-theme` on `<html>` and kept under `yuppers.theme`, which the
 * script in every page's head reads before the first paint.
 */

;(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true

const en = wordingFor('en')
let root: Root | null = null

beforeEach(() => {
  localStorage.clear()
  document.documentElement.removeAttribute('data-theme')
})

afterEach(async () => {
  const mounted = root
  root = null
  if (mounted) await act(async () => mounted.unmount())
})

async function show(element: React.ReactNode): Promise<void> {
  document.body.innerHTML = '<main id="root"></main>'
  const i18n = createI18n('en', en, () => {})
  await act(async () => {
    root = createRoot(document.getElementById('root')!)
    root.render(<I18nContext.Provider value={i18n}>{element}</I18nContext.Provider>)
  })
}

const radio = (label: string) =>
  [...document.querySelectorAll<HTMLInputElement>('input[type="radio"]')].find(
    (input) => input.closest('label')?.textContent === label,
  )!

test('is three radio buttons under one legend, System chosen unless the device chose', async () => {
  await show(<Appearance />)
  const fieldset = document.querySelector('fieldset')!
  expect(fieldset.querySelector('legend')?.textContent).toBe(en.appearance.heading)
  expect(
    [...fieldset.querySelectorAll('label')].map((label) => label.textContent),
  ).toEqual([en.appearance.system, en.appearance.light, en.appearance.dark])
  expect(radio(en.appearance.system).checked).toBe(true)
  const results = await axe.run(document.body, {
    rules: { 'color-contrast': { enabled: false }, region: { enabled: false } },
  })
  expect(results.violations.map((violation) => violation.id)).toEqual([])
})

test('applies a choice at once and keeps it on this device; System forgets it', async () => {
  await show(<Appearance />)
  await act(async () => radio(en.appearance.dark).click())
  expect(document.documentElement.getAttribute('data-theme')).toBe('dark')
  expect(localStorage.getItem('yuppers.theme')).toBe('dark')
  expect(radio(en.appearance.dark).checked).toBe(true)

  await act(async () => radio(en.appearance.system).click())
  expect(document.documentElement.hasAttribute('data-theme')).toBe(false)
  expect(localStorage.getItem('yuppers.theme')).toBeNull()
})

test('shows the choice the page started with', async () => {
  document.documentElement.setAttribute('data-theme', 'light')
  await show(<Appearance />)
  expect(radio(en.appearance.light).checked).toBe(true)
})

test('opens from the footer, in place', async () => {
  await show(<FooterAppearance />)
  const button = document.querySelector('button')!
  expect(button.textContent).toBe(en.appearance.heading)
  expect(button.getAttribute('aria-expanded')).toBe('false')
  expect(document.querySelector('fieldset')).toBeNull()
  await act(async () => button.click())
  expect(button.getAttribute('aria-expanded')).toBe('true')
  expect(document.querySelector('fieldset')).not.toBeNull()
})
