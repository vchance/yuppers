import { readFile } from 'node:fs/promises'

import { expect, test, type Person } from './support/fixtures'
import {
  acceptOpen,
  agreedItem,
  addItems,
  history,
  inviteFor,
  move,
  reviewAndSend,
  setUpProfile,
  signIn,
  signUp,
  startExchange,
  stateTag,
} from './support/flows'
import { en, fill } from './support/wording'

const BICYCLE = 'A blue bicycle, 54 cm frame, with lights'
const PAYMENT = 'Payment for the bicycle'

test('a first proposal goes from a new draft to a completed exchange both can take a copy of', async ({
  person,
}) => {
  const ana = await person('Ana')
  const bruno = await person('Bruno')

  // Ana writes and sends a first proposal: a bicycle from her, money from Bruno.
  await signUp(ana)
  await startExchange(ana)
  await ana.page.getByLabel(en.composer.otherName).fill(bruno.name)
  // She doesn't name him, which she has to choose to do: the link will be
  // for anyone, and she is told what that costs.
  await expect(ana.page.getByText(en.invitationLink.forIntro)).toBeVisible()
  await expect(ana.page.getByText(en.invitationLink.forNoContact)).toBeVisible()
  await inviteFor(ana.page, null)
  await expect(ana.page.getByLabel(en.invitationLink.forLabel, { exact: true })).toHaveCount(0)
  await addItems(ana.page, [
    { from: 'me', kind: 'ITEM', description: BICYCLE },
    { from: 'them', kind: 'MONEY', description: PAYMENT, amount: '120' },
  ])
  await expect(ana.page.getByText(en.composer.moneyOutside)).toBeVisible()
  await reviewAndSend(ana.page)

  // The link is shown once, and copies from the ways to share it.
  const field = ana.page.getByLabel(en.invitationLink.linkLabel, { exact: true })
  await expect(field).toHaveValue(/^http:\/\/127\.0\.0\.1:\d+\/en\/i#.+/)
  const link = await field.inputValue()
  await ana.page.getByRole('button', { name: en.invitationLink.share, exact: true }).click()
  await ana.page.getByRole('button', { name: en.invitationLink.copy }).click()
  await expect(ana.page.getByText(en.invitationLink.copied, { exact: true })).toBeVisible()
  expect(await ana.page.evaluate(() => navigator.clipboard.readText())).toBe(link)

  // Bruno opens it signed out: it says only that a yup is waiting, and asks
  // him to sign in to read it.
  await bruno.page.goto(link)
  await expect(
    bruno.page.getByRole('heading', { name: en.invitation.signedOutTitle, level: 1 }),
  ).toBeVisible()
  await expect(bruno.page.getByText(en.invitation.signInToRead)).toBeVisible()
  await expect(bruno.page.getByText(BICYCLE)).toHaveCount(0)
  await expect(bruno.page.getByText(ana.name)).toHaveCount(0)
  // The token has left the address bar.
  expect(new URL(bruno.page.url()).hash).toBe('')

  // Signed in, he reads everything before responding.
  await signIn(bruno)
  await expect(
    bruno.page.getByRole('heading', { name: en.invitation.title, level: 1 }),
  ).toBeVisible()
  await expect(
    bruno.page.getByText(fill(en.claimant.invitationIntroSignedIn, { name: ana.name })),
  ).toBeVisible()
  await expect(bruno.page.getByText(BICYCLE, { exact: true })).toBeVisible()
  await expect(bruno.page.getByText(PAYMENT, { exact: true })).toBeVisible()
  await expect(bruno.page.getByText(en.terms.moneyOutside)).toBeVisible()

  await bruno.page.getByRole('button', { name: en.invitation.respondNew, exact: true }).click()
  await setUpProfile(bruno)
  await bruno.page.waitForURL(/\/exchanges\/[0-9a-f-]{36}$/)
  await expect(
    bruno.page.getByText(fill(en.exchange.waitingConfirmation, { name: ana.name })),
  ).toBeVisible()

  // He signs; it takes effect once Ana confirms him.
  await acceptOpen(bruno)
  await expect(
    bruno.page.getByText(fill(en.exchange.waitingConfirmationSigned, { name: ana.name })),
  ).toBeVisible()
  await expect(stateTag(bruno.page)).toHaveText(en.states.NEGOTIATING)

  // Ana is asked whether this is who she invited, and says yes. She is shown
  // enough of his address to recognise it, not all of it.
  await ana.page.reload()
  const masked = `${bruno.email[0]}•••@example.test`
  await expect(
    ana.page.getByText(fill(en.exchange.claimedBody, { name: bruno.name, identifier: masked })),
  ).toBeVisible()
  await expect(ana.page.getByText(en.exchange.claimedSigned)).toBeVisible()
  await ana.page.getByRole('button', { name: en.exchange.confirmCounterparty }).click()
  await expect(stateTag(ana.page)).toHaveText(en.states.ACTIVE)

  // The bicycle: Ana marks it delivered, Bruno confirms.
  await move(ana.page, BICYCLE, en.exchange.moves.CLAIM, 'Left with the concierge.')
  await expect(agreedItem(ana.page, BICYCLE).locator('.status')).toHaveText(
    en.contributionStatus.CLAIMED,
  )
  await bruno.page.reload()
  await expect(stateTag(bruno.page)).toHaveText(en.states.ACTIVE)
  await move(bruno.page, BICYCLE, en.exchange.moves.CONFIRM)
  await expect(agreedItem(bruno.page, BICYCLE).locator('.status')).toHaveText(
    en.contributionStatus.ACCEPTED,
  )

  // The money, paid outside the product: Bruno says he paid, Ana that she received it.
  await move(bruno.page, PAYMENT, en.exchange.moneyMoves.CLAIM)
  await expect(agreedItem(bruno.page, PAYMENT).locator('.status')).toHaveText(
    en.moneyStatus.CLAIMED,
  )
  await ana.page.reload()
  await move(ana.page, PAYMENT, en.exchange.moneyMoves.CONFIRM)

  // Every required item is confirmed, so the exchange is complete.
  await expect(stateTag(ana.page)).toHaveText(en.outcomes.COMPLETED)
  await expect(agreedItem(ana.page, PAYMENT).locator('.status')).toHaveText(
    en.moneyStatus.ACCEPTED,
  )
  await expect(history(ana.page).getByText(en.record.events.closed.COMPLETED)).toBeVisible()
  await bruno.page.reload()
  await expect(stateTag(bruno.page)).toHaveText(en.outcomes.COMPLETED)

  // Both can read the record and take a copy.
  for (const reader of [ana, bruno]) await checkRecord(reader)
})

async function checkRecord(reader: Person): Promise<void> {
  const { page } = reader
  await page.getByRole('link', { name: en.record.open }).click()
  const heading = page.getByRole('heading', { level: 1 })
  await expect(heading).toHaveText(/^Record \S+$/)
  const code = (await heading.textContent())!.replace('Record ', '')
  await expect(page.getByText(BICYCLE, { exact: true }).first()).toBeVisible()
  await expect(page.getByText(en.record.versionStatus.IN_FORCE)).toBeVisible()

  const [download] = await Promise.all([
    page.waitForEvent('download'),
    page.getByRole('button', { name: en.record.download }).click(),
  ])
  expect(download.suggestedFilename()).toBe(`${fill(en.record.fileName, { code })}.json`)
  const copy = JSON.parse(await readFile(await download.path(), 'utf8'))
  expect(copy.format).toBe('exchange-record')
  expect(copy.exchange.display_code).toBe(code)
  expect(copy.exchange.closed_outcome).toBe('COMPLETED')
  expect(copy.parties).toEqual({ A: 'Ana', B: 'Bruno' })
  // Nothing in it says who either party is beyond their names.
  expect(JSON.stringify(copy)).not.toContain('@example.test')
}
