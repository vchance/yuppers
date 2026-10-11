import { expect, test } from './support/fixtures'
import {
  composerItem,
  exchangeIdOf,
  inviteFor,
  join,
  reviewAndSend,
  signUp,
} from './support/flows'
import { en } from './support/wording'

/*
 * Starting a yup and the sample yup (DESIGN.md sections 4.3 and 4.4), in a
 * real browser against the real service: the example is there before
 * signing in, the first yup starts from a common agreement, and what the
 * service was told about how it began is never shown to anyone.
 */

test('the example can be read before signing in, and nothing in it can be done', async ({
  person,
}) => {
  const visitor = await person('Visitor')
  const { page } = visitor
  await page.goto('/')
  await page.getByRole('link', { name: en.sample.signInLine }).click()
  await expect(page.getByRole('heading', { name: en.sample.title, level: 1 })).toBeVisible()
  await expect(page.getByText(en.sample.banner)).toBeVisible()
  await expect(page.getByText('Dana Reyes').first()).toBeVisible()
  // Every action is there by its name and none of them does anything.
  for (const name of [en.exchange.moves.CLAIM, en.exchange.moneyMoves.CLAIM]) {
    await expect(page.getByRole('button', { name })).toBeDisabled()
  }
})

test('a first yup starts from a common agreement, and how it began is never shown', async ({
  person,
}) => {
  const ana = await person('Ana')
  const bruno = await person('Bruno')
  await signUp(ana)
  const { page } = ana

  // With no yups yet, the card that shows what one looks like.
  await expect(page.getByRole('heading', { name: en.sample.homeHeading })).toBeVisible()
  await page.getByRole('button', { name: en.home.start }).click()
  await expect(page.getByRole('heading', { name: en.templates.chooserTitle, level: 1 })).toBeVisible()
  await expect(page.getByText(en.templates.notFor)).toBeVisible()
  await page
    .getByRole('button', { name: en.templates.entries['selling-something'].name, exact: true })
    .click()
  await page.waitForURL(/\/exchanges\/[0-9a-f-]{36}$/)
  await expect(page.getByRole('heading', { name: en.composer.titleFirst, level: 1 })).toBeVisible()
  const id = exchangeIdOf(page)

  // Two items in place, with their examples in grey and nothing typed.
  const entry = en.templates.entries['selling-something']
  const first = composerItem(page, 1)
  await expect(first.getByLabel(en.composer.descriptionLabel)).toHaveValue('')
  await expect(first.getByLabel(en.composer.descriptionLabel)).toHaveAttribute(
    'placeholder',
    entry.items[0].description,
  )
  await expect(page.getByText(entry.hint)).toBeVisible()

  // Without writing a description, it cannot go on.
  await page.getByRole('button', { name: en.composer.review, exact: true }).click()
  await expect(page.getByText(en.composer.problems.DESCRIPTION_MISSING).first()).toBeVisible()

  await page.getByLabel(en.composer.otherName).fill(bruno.name)
  await inviteFor(page, null)
  await first.getByLabel(en.composer.descriptionLabel).fill('A blue bicycle')
  await first.getByLabel(en.composer.dueLabel).selectOption('DATE')
  await first.getByLabel(en.composer.dateLabel).fill('2030-03-12')
  const second = composerItem(page, 2)
  await second.getByLabel(en.composer.descriptionLabel).fill('Payment for the bicycle')
  await second.getByLabel(/^Amount in /).fill('120')
  await second.getByLabel(en.composer.dueLabel).selectOption('DATE')
  await second.getByLabel(en.composer.dateLabel).fill('2030-03-12')
  await reviewAndSend(page)
  await expect(page.getByRole('heading', { name: `Send it to ${bruno.name}`, level: 1 })).toBeVisible()
  const link = await page.getByLabel(en.invitationLink.linkLabel, { exact: true }).inputValue()
  await page.getByRole('button', { name: en.invitationLink.later, exact: true }).click()

  // What the service holds is what was typed: no example, and not how it began.
  for (const path of [`/v1/exchanges/${id}`, `/v1/exchanges/${id}/history`, `/v1/exchanges/${id}/record`, '/v1/exchanges']) {
    const answer = await page.request.get(path)
    expect(answer.ok(), path).toBe(true)
    const text = await answer.text()
    expect(text, path).not.toContain('started_from')
    expect(text, path).not.toContain('selling-something')
    expect(text, path).not.toContain('e.g.')
  }

  // Bruno reads the yup and sees nothing of the starting point.
  await join(bruno, link)
  await expect(bruno.page.getByText('A blue bicycle').first()).toBeVisible()
  const read = await bruno.page.content()
  expect(read).not.toContain('e.g.')
  expect(read).not.toContain(entry.hint)
})
