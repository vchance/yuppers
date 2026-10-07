import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'

import type { HelpWording } from '@yuppers/shared'

import { repoRoot } from './support/env'
import { expect, test } from './support/fixtures'
import { addItems, inviteFor, signUp, startExchange } from './support/flows'
import { en, es } from './support/wording'

/*
 * The help pages: reached from the signing step, where "Learn more" opens
 * the topic in a new tab and leaves the step as it was, and read through
 * their table of contents, in the browser as a person would.
 */

const help = (language: string) =>
  JSON.parse(
    readFileSync(resolve(repoRoot, `packages/shared/wording/help/${language}.json`), 'utf8'),
  ) as HelpWording

const helpEn = help('en')
const helpEs = help('es')

test('help opens from the signing step, in a new tab, and its contents lead around it', async ({
  person,
}) => {
  const ana = await person('Ana')
  await signUp(ana)
  await startExchange(ana)
  await ana.page.getByLabel(en.composer.otherName).fill('Bruno Díaz')
  await inviteFor(ana.page, 'bruno@example.test')
  await addItems(ana.page, [{ from: 'me', kind: 'TASK', description: 'Paint the fence' }])
  await ana.page.getByRole('button', { name: en.composer.review, exact: true }).click()
  await expect(
    ana.page.getByRole('heading', { name: en.composer.signTitle, level: 1 }),
  ).toBeVisible()
  await ana.page.getByLabel(en.consent.agree).check()

  // "Learn more" sits in the signing step and opens the topic in a new tab.
  const learnMore = ana.page.getByRole('link', { name: en.help.learnMore.signing })
  await expect(learnMore).toBeVisible()
  const [tab] = await Promise.all([ana.context.waitForEvent('page'), learnMore.click()])
  await tab.waitForLoadState()
  expect(new URL(tab.url()).pathname).toBe('/help/signing')
  const topic = helpEn.topics.signing
  await expect(tab.getByRole('heading', { name: topic.title, level: 1 })).toBeVisible()
  await expect(tab).toHaveTitle(`${topic.title} · ${en.productName}`)

  // The signing step is where it was, the box still ticked.
  await expect(ana.page.getByLabel(en.consent.agree)).toBeChecked()
  await expect(
    ana.page.getByRole('button', { name: en.composer.signAndSend, exact: true }),
  ).toBeEnabled()

  // "On this page" leads to each section, and the keyboard goes with it.
  const contents = tab.getByRole('navigation', { name: helpEn.onThisPage })
  const recorded = topic.blocks.flatMap((block) => ('h' in block ? [block.h] : []))[0]
  await contents.getByRole('link', { name: recorded }).click()
  const section = tab.getByRole('heading', { name: recorded, level: 2 })
  await expect(section).toBeFocused()
  await expect(section).toBeInViewport()
  expect(new URL(tab.url()).hash).toBe('#section-1')

  // The list of every topic leads to another, without reloading the page.
  const topics = tab.getByRole('navigation', { name: helpEn.topicsHeading })
  await expect(topics.getByRole('link', { name: topic.title })).toHaveAttribute(
    'aria-current',
    'page',
  )
  await tab.evaluate(() => {
    ;(window as unknown as { stayed: boolean }).stayed = true
  })
  await topics.getByRole('link', { name: helpEn.topics.blocking.title }).click()
  await expect(
    tab.getByRole('heading', { name: helpEn.topics.blocking.title, level: 1 }),
  ).toBeFocused()
  expect(new URL(tab.url()).pathname).toBe('/help/blocking')
  expect(await tab.evaluate(() => (window as unknown as { stayed?: boolean }).stayed)).toBe(true)

  // And back to the first page, which lists them all.
  await tab.getByRole('link', { name: helpEn.allTopics }).click()
  await expect(tab.getByRole('heading', { name: helpEn.title, level: 1 })).toBeVisible()
  await expect(
    tab.getByRole('navigation', { name: helpEn.topicsHeading }).getByRole('link'),
  ).toHaveCount(Object.keys(helpEn.topics).length)
  await tab.close()
})

test('the footer leads to help from the invitation page’s front door, and a link can ask for Spanish', async ({
  person,
}) => {
  const visitor = await person('Visitor')
  await visitor.page.goto('/')
  await expect(visitor.page.getByRole('heading', { name: en.signIn.title, level: 1 })).toBeVisible()
  await visitor.page.getByRole('contentinfo').getByRole('link', { name: en.help.link }).click()
  await expect(visitor.page.getByRole('heading', { name: helpEn.title, level: 1 })).toBeVisible()

  // Opened directly, the skip link is the first stop and goes to the help itself.
  await visitor.page.goto('/help/signing')
  await expect(
    visitor.page.getByRole('heading', { name: helpEn.topics.signing.title, level: 1 }),
  ).toBeVisible()
  await visitor.page.keyboard.press('Tab')
  const skip = visitor.page.getByRole('link', { name: en.common.skipToContent })
  await expect(skip).toBeFocused()
  await skip.press('Enter')
  await expect(visitor.page.locator('main')).toBeFocused()

  // The mobile app opens help this way, naming its language.
  await visitor.page.goto('/help/languages?lang=es')
  await expect(
    visitor.page.getByRole('heading', { name: helpEs.topics.languages.title, level: 1 }),
  ).toBeVisible()
  await expect(visitor.page.locator('html')).toHaveAttribute('lang', 'es')
  await expect(visitor.page.getByRole('contentinfo').getByRole('link', { name: es.help.link })).toBeVisible()
})
