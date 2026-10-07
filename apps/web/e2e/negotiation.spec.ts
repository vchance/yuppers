import type { Page } from '@playwright/test'

import { expect, test, type Person } from './support/fixtures'
import {
  acceptOpen,
  composerItem,
  history,
  negotiate,
  propose,
  reviewAndSend,
  stateTag,
  type ItemSpec,
} from './support/flows'
import { en, fill } from './support/wording'

const ITEMS: ItemSpec[] = [
  { from: 'me', kind: 'SERVICE', description: 'Two hours of guitar lessons' },
  { from: 'them', kind: 'MONEY', description: 'Lesson fee', amount: '60' },
]

/** The revision waiting to be signed. */
function openRevision(page: Page) {
  return page.getByRole('region', { name: en.exchange.proposalHeading, exact: true })
}

test('a named counterparty proposes changes, and the version they replace is set aside', async ({
  person,
}) => {
  const ana = await person('Ana')
  const bruno = await person('Bruno')
  await negotiate(ana, bruno, ITEMS)

  // Bruno answers with terms of his own: a lower fee.
  await openRevision(bruno.page).getByRole('link', { name: en.exchange.counter }).click()
  await expect(
    bruno.page.getByRole('heading', { name: en.composer.titleCounter, level: 1 }),
  ).toBeVisible()
  await composerItem(bruno.page, 2).getByLabel(/^Amount in /).fill('45')
  await bruno.page.getByLabel(en.composer.noteLabel).fill('Could we make it 45?')
  await reviewAndSend(bruno.page)
  await expect(openRevision(bruno.page)).toContainText(fill(en.exchange.version, { number: 2 }))
  await expect(openRevision(bruno.page)).toContainText(en.exchange.sentByYou)

  // Ana sees version 2, from Bruno, and signs it.
  await ana.page.reload()
  const open = openRevision(ana.page)
  await expect(open).toContainText(fill(en.exchange.version, { number: 2 }))
  await expect(open).toContainText(fill(en.exchange.sentByOther, { name: bruno.name }))
  await expect(open.getByText('Could we make it 45?', { exact: true })).toBeVisible()
  await acceptOpen(ana)
  await expect(stateTag(ana.page)).toHaveText(en.states.ACTIVE)

  // Ana's signature on version 1 was set aside when Bruno sent version 2.
  await expect(
    history(ana.page).getByText(fill(en.record.events.neutral.REVISION_SUPERSEDED, { number: 1 })),
  ).toBeVisible()
  await expect(
    history(ana.page).getByText(
      fill(en.record.events.neutral.AGREEMENT_IN_FORCE, { number: 2 }),
    ),
  ).toBeVisible()

  // The record says the same, version by version.
  await ana.page.getByRole('link', { name: en.record.open }).click()
  const first = ana.page.getByRole('region', {
    name: fill(en.record.versionHeading, { number: 1 }),
    exact: true,
  })
  await expect(first).toContainText(en.record.versionStatus.SUPERSEDED)
  await expect(first.getByText(signedBy(ana.name))).toBeVisible()
  const second = ana.page.getByRole('region', {
    name: fill(en.record.versionHeading, { number: 2 }),
    exact: true,
  })
  await expect(second).toContainText(en.record.versionStatus.IN_FORCE)
  await expect(second.getByText(signedBy(ana.name))).toBeVisible()
  await expect(second.getByText(signedBy(bruno.name))).toBeVisible()
})

/** "Signed by {name} on {date}.", whatever the date. */
function signedBy(name: string): RegExp {
  const [before, after] = fill(en.record.versionSignedBy, { name }).split('{date}')
  const literal = (text: string) => text.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')
  return new RegExp(`^${literal(before)}.+${literal(after)}$`)
}

test('a declined proposal and a withdrawn one both close without agreement, under the other party’s name', async ({
  person,
}) => {
  const ana = await person('Ana')
  const bruno = await person('Bruno')

  // The first: Bruno, once confirmed, declines it.
  const declined = await negotiate(ana, bruno, ITEMS)
  await openRevision(bruno.page).getByRole('button', { name: en.exchange.decline }).click()
  await bruno.page
    .getByRole('group', { name: en.exchange.decline })
    .getByRole('button', { name: en.exchange.confirmDecline })
    .click()
  await expect(stateTag(bruno.page)).toHaveText(en.outcomes.NOT_AGREED)
  await expect(bruno.page.getByText(en.closedReasons.DECLINED)).toBeVisible()

  // The second: Ana withdraws it after Bruno has joined.
  const { id: withdrawn, link } = await propose(ana, bruno, ITEMS)
  await bruno.page.goto(link)
  await bruno.page
    .getByRole('button', { name: fill(en.invitation.respondAs, { name: bruno.name }) })
    .click()
  await bruno.page.waitForURL(/\/exchanges\/[0-9a-f-]{36}$/)
  await ana.page.reload()
  await ana.page.getByRole('button', { name: en.exchange.confirmCounterparty }).click()
  await openRevision(ana.page).getByRole('button', { name: en.exchange.withdraw }).click()
  await ana.page
    .getByRole('group', { name: en.exchange.withdraw })
    .getByRole('button', { name: en.exchange.confirmWithdraw })
    .click()
  await expect(stateTag(ana.page)).toHaveText(en.outcomes.NOT_AGREED)
  await expect(ana.page.getByText(en.closedReasons.WITHDRAWN)).toBeVisible()

  // Each page is titled with the other party's name, and says why it closed.
  for (const [reader, other] of [
    [ana, bruno],
    [bruno, ana],
  ] as const) {
    for (const [id, reason] of [
      [declined, en.closedReasons.DECLINED],
      [withdrawn, en.closedReasons.WITHDRAWN],
    ] as const) {
      await reader.page.goto(`/exchanges/${id}`)
      await expect(
        reader.page.getByRole('heading', {
          name: fill(en.exchange.title, { name: other.name }),
          level: 1,
        }),
      ).toBeVisible()
      await expect(stateTag(reader.page)).toHaveText(en.outcomes.NOT_AGREED)
      await expect(reader.page.getByText(reason)).toBeVisible()
    }
    await checkClosedList(reader, other.name, 2)
  }
})

/** Both exchanges are in the closed group of the list, folded away, and nowhere else. */
async function checkClosedList(reader: Person, otherName: string, count: number): Promise<void> {
  const { page } = reader
  await page.goto('/')
  await expect(page.getByRole('heading', { name: en.home.title, level: 1 })).toBeVisible()
  await expect(page.getByRole('region', { name: en.home.groupOpen })).toHaveCount(0)
  const closed = page.getByRole('region', { name: en.home.groupClosed, exact: true })
  await closed.getByRole('button', { name: fill(en.home.showClosed, { count }) }).click()
  const cards = closed.locator('li.card')
  await expect(cards).toHaveCount(count)
  for (const card of await cards.all()) {
    await expect(card.getByRole('link')).toHaveText(fill(en.home.withParty, { name: otherName }))
    await expect(card.locator('.tag')).toHaveText(en.outcomes.NOT_AGREED)
  }
}
