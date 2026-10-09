import { ApiPerson } from './support/api'
import { expect, test } from './support/fixtures'
import { button, signUp, title } from './support/flows'
import { en, fill } from './support/wording'

test('the list groups exchanges in progress, drafts and closed ones, with the closed folded away', async ({
  person,
}) => {
  const ben = await person('Ben')
  const { page } = ben
  await signUp(ben)

  // The same account, signed in a second time to set the scene through the
  // API: one proposal waiting, one draft, and two that are over.
  const benElsewhere = await ApiPerson.signUp('Ben', ben.email)
  await benElsewhere.propose('Cleo', [{ from: 'A', kind: 'ITEM', description: 'A kettle' }])
  await benElsewhere.draft()
  await benElsewhere.discard(await benElsewhere.draft())
  const { id: withdrawn } = await benElsewhere.propose('Dan', [
    { from: 'A', kind: 'TASK', description: 'Walk the dog' },
  ])
  await benElsewhere.withdraw(withdrawn)

  await page.reload()
  await expect(title(page, en.home.title)).toBeVisible()

  // Each is read as where it stands, its reference and when it changed, and
  // then who it is with: the name is the other party's own words, and is
  // never the first thing said.
  const exchanges = page.getByRole('button', { name: /\. Reference / })
  // Cleo's link was issued through the API and never sent from here, which
  // the card says before its reference (`home.notSent`).
  const inProgress = page.getByRole('button', {
    name: new RegExp(
      `^${en.states.NEGOTIATING}\\. ${en.home.notSent}\\. Reference .*\\. ${fill(en.home.withParty, { name: 'Cleo' })}$`,
    ),
  })
  const draft = page.getByRole('button', {
    name: new RegExp(`^${en.states.DRAFT}\\. Reference .*\\. ${en.home.noParty}$`),
  })
  const closed = page.getByRole('button', {
    name: new RegExp(`^${en.outcomes.NOT_AGREED}\\. Reference `),
  })

  // In progress first, then drafts, each under its heading.
  await expect(page.getByRole('heading', { name: en.home.groupOpen, level: 2 })).toBeVisible()
  await expect(page.getByRole('heading', { name: en.home.groupDrafts, level: 2 })).toBeVisible()
  await expect(inProgress).toBeVisible()
  await expect(draft).toBeVisible()
  await expect(exchanges.first()).toHaveAccessibleName(/\. With Cleo$/)

  // What is closed is counted but folded away.
  await expect(page.getByRole('heading', { name: en.home.groupClosed, level: 2 })).toBeVisible()
  const showClosed = button(page, fill(en.home.showClosed, { count: 2 }))
  await expect(showClosed).toHaveAttribute('aria-expanded', 'false')
  await expect(closed).toHaveCount(0)
  await expect(exchanges).toHaveCount(2)

  await showClosed.click()
  await expect(closed).toHaveCount(2)
  await expect(exchanges).toHaveCount(4)
  const hideClosed = button(page, en.home.hideClosed)
  await expect(hideClosed).toHaveAttribute('aria-expanded', 'true')
  await hideClosed.click()
  await expect(closed).toHaveCount(0)

  // Each opens its exchange.
  await inProgress.click()
  await expect(title(page, fill(en.exchange.title, { name: 'Cleo' }))).toBeVisible()
})
