import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'

import type { Page } from '@playwright/test'
import {
  LEGAL_DOCUMENTS,
  LEGAL_SECTIONS,
  type LegalWording,
} from '../../../packages/shared/src/legal-text.ts'

import { expect, test } from './support/fixtures'
import { confirmClaimant, propose, signIn, signUp, stateTag } from './support/flows'
import { repoRoot } from './support/env'
import { es, fill } from './support/wording'

const legalEs = (document: string) =>
  JSON.parse(
    readFileSync(resolve(repoRoot, `packages/shared/wording/${document}/es.json`), 'utf8'),
  ) as LegalWording

/*
 * The main screens in Spanish, which runs longer than English, in a window
 * 320 CSS pixels wide, the narrowest phone a page is expected to fit (WCAG
 * 1.4.10, reflow). On each, nothing may make the page scroll sideways, and
 * no button, link, tag, heading or field may stick out of the window or cut
 * its own text off. jsdom lays nothing out, so this is checked here and not
 * in the unit tests; `packages/shared/scripts/check-lengths.mjs` holds the
 * wording in tight places to lengths that keep this true.
 */

const NARROW = { width: 320, height: 640 }

/** What sticks out on the page as it stands, one line each. */
async function overflow(page: Page): Promise<string[]> {
  return page.evaluate(() => {
    const width = document.documentElement.clientWidth
    const found: string[] = []
    const scrolled = document.documentElement.scrollWidth
    if (scrolled > width) found.push(`the page is ${scrolled} pixels wide in a ${width} window`)
    const describe = (element: Element) =>
      `${element.tagName.toLowerCase()} “${(element.textContent ?? '').trim().slice(0, 60)}”`
    const selector = 'button, a, .tag, h1, h2, h3, label, input, select, textarea'
    for (const element of document.querySelectorAll<HTMLElement>(selector)) {
      // Read by screen readers only, or shown only when it has the focus.
      if (element.closest('.visually-hidden, .skip')) continue
      const box = element.getBoundingClientRect()
      if (box.width === 0 && box.height === 0) continue
      if (box.right > width + 0.5 || box.left < -0.5) {
        found.push(`${describe(element)} runs from ${Math.round(box.left)} to ${Math.round(box.right)}`)
      }
      const clips = element.tagName === 'BUTTON' || element.classList.contains('tag')
      if (clips && element.scrollWidth > element.clientWidth + 1) {
        found.push(`${describe(element)} is cut off: ${element.scrollWidth} in ${element.clientWidth}`)
      }
    }
    return found
  })
}

/** Records what sticks out on this screen, so one run lists every screen's. */
function checker(page: Page) {
  const found: string[] = []
  return {
    found,
    async check(screen: string) {
      for (const problem of await overflow(page)) found.push(`${screen}: ${problem}`)
    },
  }
}

test('the main screens fit a 320-pixel window in Spanish', async ({ person }) => {
  // In different time zones, so each due date names Ana's beside it.
  const ana = await person('Ana', { timezoneId: 'America/Los_Angeles' })
  const carlos = await person('Carlos Domínguez', {
    locale: 'es-ES',
    viewport: NARROW,
    timezoneId: 'America/Mexico_City',
  })
  await signUp(ana)
  const { id, link } = await propose(ana, carlos, [
    { from: 'me', kind: 'ITEM', description: 'Una mesa de roble', due: '2030-03-12' },
    { from: 'them', kind: 'MONEY', description: 'El pago por la mesa', amount: '120' },
  ])

  const { page } = carlos
  const { found, check } = checker(page)

  // The invitation, signed in to, read and then answered.
  await page.goto(link.replace('/en/i#', '/es/i#'))
  await expect(
    page.getByRole('heading', { name: es.invitation.signedOutTitle, level: 1 }),
  ).toBeVisible()
  await expect(page.getByLabel(es.signIn.identifierLabel)).toBeVisible()
  await check('signing in from the invitation')
  await signIn(carlos, es)
  await expect(page.getByRole('heading', { name: es.invitation.title, level: 1 })).toBeVisible()
  await expect(page.locator('.terms')).toBeVisible()
  await expect(page.getByText('Vence el 12 de marzo de 2030 (hora de Los Angeles)')).toBeVisible()
  await check('the invitation')
  await page.getByRole('button', { name: es.invitation.respondNew, exact: true }).click()
  await expect(page.getByRole('heading', { name: es.profile.firstTitle })).toBeVisible()
  await check('the new account’s profile')
  await page.getByLabel(es.profile.nameLabel, { exact: true }).fill(carlos.name)
  await page.getByLabel(es.profile.adultLabel).check()
  await page.getByRole('button', { name: es.profile.continue, exact: true }).click()
  await page.waitForURL(/\/exchanges\/[0-9a-f-]{36}$/)

  // The proposal, and signing it.
  const open = page.getByRole('region', { name: es.exchange.proposalHeading })
  await expect(open).toBeVisible()
  await check('the proposal')
  await open.getByRole('button', { name: es.exchange.accept, exact: true }).click()
  const panel = open.getByRole('group', { name: es.exchange.signHeading })
  await expect(panel).toBeVisible()
  await check('the signing panel')
  await panel.getByLabel(es.consent.agree).check()
  await panel.getByRole('button', { name: es.exchange.accept, exact: true }).click()
  await expect(panel).toBeHidden()

  // The agreement in force, the guide for when something isn't working, and the record.
  await confirmClaimant(ana)
  await page.reload()
  await expect(stateTag(page)).toHaveText(es.states.ACTIVE)
  await check('the agreement in force')
  await page.getByRole('button', { name: es.trouble.open }).click()
  await check('the guide’s question')
  await page.getByRole('button', { name: es.trouble.situations.CANT_DO_MINE }).click()
  await check('the guide’s ways forward')
  await page.goto(`/exchanges/${id}/record`)
  await expect(page.locator('.record-plain')).toBeVisible()
  await check('the record')

  // The list, the account, and deleting it.
  await page.goto('/')
  await expect(page.getByRole('heading', { name: es.home.title, level: 1 })).toBeVisible()
  await expect(page.locator('.card').first()).toBeVisible()
  await check('the list of yups')
  await page.goto('/account')
  await expect(page.getByRole('heading', { name: es.profile.title, level: 1 })).toBeVisible()
  await check('the account')
  await page.getByRole('button', { name: es.deletion.open }).click()
  await expect(page.getByRole('heading', { name: es.deletion.heading })).toBeVisible()
  await check('deleting the account')

  // The privacy policy and the terms, as the app shows them.
  for (const document of LEGAL_DOCUMENTS) {
    await page.goto(`/es/${document}`)
    await expect(
      page.getByRole('heading', { name: legalEs(document).title, level: 1 }),
    ).toBeVisible()
    await expect(page.locator('section > h2')).toHaveCount(LEGAL_SECTIONS[document].length)
    await check(`the ${document} page`)
  }

  // Writing a proposal, and the signing step.
  await page.goto('/')
  await page.getByRole('button', { name: es.home.start }).click()
  await expect(page.getByRole('heading', { name: es.composer.titleFirst, level: 1 })).toBeVisible()
  await page.getByLabel(es.composer.otherName).fill('Ana')
  await page.getByRole('button', { name: es.composer.addYours }).click()
  await page.getByRole('button', { name: es.composer.addTheirs }).click()
  await check('the composer')
  await page.getByRole('button', { name: es.composer.review, exact: true }).click()
  await check('the composer, with what needs fixing')

  // Who it is for, the signing step, then the link and the ways to share it.
  for (const [number, description] of [
    [1, 'Una silla'],
    [2, 'Una lámpara'],
  ] as const) {
    await page
      .getByRole('group', { name: fill(es.composer.itemLegend, { number }), exact: true })
      .getByLabel(es.composer.descriptionLabel)
      .fill(description)
  }
  await page.getByLabel(es.invitationLink.forLabel, { exact: true }).fill('ana@example.test')
  await page.getByRole('button', { name: es.composer.review, exact: true }).click()
  await expect(page.getByRole('heading', { name: es.composer.signTitle, level: 1 })).toBeVisible()
  await check('the signing step')
  await page.getByLabel(es.consent.agree).check()
  await page.getByRole('button', { name: es.composer.signAndSend, exact: true }).click()
  await expect(page.getByLabel(es.invitationLink.linkLabel, { exact: true })).toHaveValue(
    /\/es\/i#/,
  )
  await check('the invitation link')
  await page.getByRole('button', { name: es.invitationLink.share, exact: true }).click()
  await page.getByRole('button', { name: es.invitationLink.shareQr, exact: true }).click()
  await expect(page.getByRole('img', { name: es.invitationLink.qrLabel })).toBeVisible()
  await check('the ways to share the link, with its QR code')

  // Help: the list of topics, and the longest topic with its contents.
  await page.goto('/help')
  await expect(page.locator('nav.help-topics')).toBeVisible()
  await check('help')
  await page.goto('/help/keeping-track')
  await expect(page.locator('nav.help-sections')).toBeVisible()
  await check('a help topic')

  expect(found).toEqual([])
})
