import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'

import {
  LEGAL_DOCUMENTS,
  LEGAL_SECTIONS,
  PRIVACY_EMAIL,
  SUPPORT_EMAIL,
  type LegalDocument,
  type LegalWording,
} from '../../../packages/shared/src/legal-text.ts'

import { repoRoot } from './support/env'
import { expect, test } from './support/fixtures'
import { en, es } from './support/wording'

/*
 * The privacy policy and the terms, as the service serves them: read in
 * full with scripts off, as a crawler or a reviewer reads them; then with
 * scripts on, where the app shows the same documents as its own pages; and
 * reached from signing in.
 */

const legal = (document: LegalDocument, language: string) =>
  JSON.parse(
    readFileSync(resolve(repoRoot, `packages/shared/wording/${document}/${language}.json`), 'utf8'),
  ) as LegalWording

test('each document reads in full without scripts, in each language, with its headers', async ({
  person,
}) => {
  const reader = await person('Crawler', { javaScriptEnabled: false })
  const { page } = reader

  for (const document of LEGAL_DOCUMENTS) {
    for (const language of ['en', 'es']) {
      const address = language === 'en' ? `/${document}` : `/${language}/${document}`
      const wording = legal(document, language)
      const response = await page.goto(address)
      expect(response?.status(), address).toBe(200)
      expect(response?.headers()['content-security-policy']).toContain("default-src 'self'")
      expect(response?.headers()['cache-control']).toBe('no-cache')
      await expect(page.getByRole('heading', { name: wording.title, level: 1 })).toBeVisible()
      for (const id of LEGAL_SECTIONS[document]) {
        await expect(page.locator(`section > h2#${id}`)).toHaveText(
          (wording.sections as Record<string, { title: string }>)[id].title,
        )
      }
      const email = document === 'privacy' ? PRIVACY_EMAIL : SUPPORT_EMAIL
      await expect(page.locator(`a[href="mailto:${email}"]`).first()).toBeVisible()
      await expect(page.locator('meta[name="robots"]')).toHaveAttribute('content', 'index, follow')
      // HELP and STOP stand out, as carriers ask, without scripts too.
      const texts = page.locator('section:has(> h2#text-messages)')
      await expect(texts.locator('strong', { hasText: 'HELP' })).toHaveCount(1)
      await expect(texts.locator('strong', { hasText: 'STOP' })).toHaveCount(1)
    }
  }

  // The sections on text messages have links of their own, which reviewers are given.
  await page.goto('/terms#text-messages')
  await expect(page.locator('h2#text-messages')).toBeInViewport()
  const texts = page.locator('section:has(> h2#text-messages)')
  await expect(texts.getByText('Message and data rates may apply.')).toBeVisible()
  await expect(texts.locator('a[href="/privacy#text-messages"]')).toBeVisible()
  await texts.locator('a[href="/privacy#text-messages"]').click()
  await expect(page).toHaveURL(/\/privacy#text-messages$/)
  await expect(page.locator('h2#text-messages')).toBeInViewport()
})

test('with scripts, the app shows the same document, and the language picks the address', async ({
  person,
}) => {
  const reader = await person('Reader')
  const { page } = reader
  const privacyEn = legal('privacy', 'en')
  const privacyEs = legal('privacy', 'es')

  await page.goto('/privacy')
  await expect(page.getByRole('heading', { name: privacyEn.title, level: 1 })).toBeVisible()
  // The app's own header has drawn over the page the service sent.
  await expect(page.getByRole('combobox', { name: en.nav.language })).toBeVisible()
  await expect(page.locator('section > h2')).toHaveCount(LEGAL_SECTIONS.privacy.length)

  await page
    .getByRole('navigation', { name: privacyEn.otherLanguages })
    .getByRole('link', { name: 'Español' })
    .click()
  await expect(page.getByRole('heading', { name: privacyEs.title, level: 1 })).toBeVisible()
  await expect(page).toHaveURL(/\/es\/privacy$/)
  await page.reload()
  await expect(page.getByRole('heading', { name: privacyEs.title, level: 1 })).toBeVisible()

  // The contents lead to a section, and the keyboard goes with it.
  const sms = privacyEs.sections['text-messages'].title
  await page.getByRole('navigation', { name: privacyEs.contents }).getByRole('link', { name: sms }).click()
  await expect(page.getByRole('heading', { name: sms, level: 2 })).toBeFocused()
  await expect(page).toHaveURL(/\/es\/privacy#text-messages$/)

  // And on to the terms, in the same language.
  await page.getByRole('contentinfo').getByRole('link', { name: es.termsOfUse.link }).click()
  await expect(page.getByRole('heading', { name: legal('terms', 'es').title, level: 1 })).toBeVisible()
  await expect(page).toHaveURL(/\/es\/terms$/)
})

test('signing in says what texts cost and opens the documents in a new tab', async ({ person }) => {
  const visitor = await person('Visitor')
  const { page, context } = visitor
  await page.goto('/')
  await expect(page.getByRole('heading', { name: en.signIn.title, level: 1 })).toBeVisible()
  await expect(page.getByText(en.privacy.sms)).toBeVisible()

  const [tab] = await Promise.all([
    context.waitForEvent('page'),
    page.getByRole('link', { name: en.privacy.smsLink }).click(),
  ])
  await tab.waitForLoadState()
  expect(new URL(tab.url()).pathname).toBe('/privacy')
  expect(new URL(tab.url()).hash).toBe('#text-messages')
  await expect(tab.getByRole('heading', { name: legal('privacy', 'en').title, level: 1 })).toBeVisible()

  const [terms] = await Promise.all([
    context.waitForEvent('page'),
    page.getByRole('link', { name: en.termsOfUse.document }).click(),
  ])
  await terms.waitForLoadState()
  expect(new URL(terms.url()).pathname).toBe('/terms')
  await expect(terms.getByRole('heading', { name: legal('terms', 'en').title, level: 1 })).toBeVisible()

  // The footer leads there too, in the language on screen.
  await page.getByRole('combobox', { name: en.nav.language }).selectOption('es')
  await page.getByRole('contentinfo').getByRole('link', { name: es.privacy.link }).click()
  await expect(page.getByRole('heading', { name: legal('privacy', 'es').title, level: 1 })).toBeVisible()
  await expect(page).toHaveURL(/\/es\/privacy$/)
})
