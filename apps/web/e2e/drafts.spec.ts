import { expect, test } from './support/fixtures'
import { draftId, signUp, startExchange } from './support/flows'
import { en, fill } from './support/wording'

test('a draft that was never sent can be discarded from the composer, and leaves the list', async ({
  person,
}) => {
  const ana = await person('Ana')
  const { page } = ana
  await signUp(ana)
  await startExchange(ana)

  // Something written, so the draft is made and saved as a working copy.
  await page.getByLabel(en.composer.otherName).fill('Carmen')
  const id = await draftId(page)
  await expect(page.getByText(en.composer.saved, { exact: true })).toBeVisible()

  // It is listed among the drafts.
  await page.getByRole('link', { name: en.nav.exchanges, exact: true }).first().click()
  const drafts = page.getByRole('region', { name: en.home.groupDrafts, exact: true })
  await expect(drafts.locator('li.card')).toHaveCount(1)
  await expect(drafts.getByRole('link')).toHaveAttribute('href', `/exchanges/${id}`)

  // Back in the composer, it is thrown away.
  await drafts.getByRole('link').click()
  await expect(page.getByRole('heading', { name: en.composer.titleFirst, level: 1 })).toBeVisible()
  await expect(page.getByLabel(en.composer.otherName)).toHaveValue('Carmen')
  await page.getByRole('button', { name: en.composer.discard, exact: true }).click()
  const panel = page.getByRole('group', { name: en.composer.discard })
  await expect(panel.getByText(en.composer.discardText)).toBeVisible()
  await panel.getByRole('button', { name: en.composer.confirmDiscard }).click()

  // Back on the list, with no draft and nothing in progress.
  await expect(page).toHaveURL(/\/$/)
  await expect(page.getByRole('heading', { name: en.home.title, level: 1 })).toBeVisible()
  await expect(page.getByRole('region', { name: en.home.groupDrafts })).toHaveCount(0)
  await expect(page.getByRole('region', { name: en.home.groupOpen })).toHaveCount(0)

  // What remains is a closed exchange whose record says it was discarded.
  await page.goto(`/exchanges/${id}`)
  await expect(page.locator('main .tags .tag').first()).toHaveText(en.outcomes.NOT_AGREED)
  await expect(page.getByText(en.closedReasons.DISCARDED)).toBeVisible()
  await page.goto('/')
  const closed = page.getByRole('region', { name: en.home.groupClosed, exact: true })
  await expect(closed.getByRole('button', { name: fill(en.home.showClosed, { count: 1 }) })).toBeVisible()
})
