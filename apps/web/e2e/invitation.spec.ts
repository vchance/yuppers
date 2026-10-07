import { expect, test } from './support/fixtures'
import {
  composerItem,
  join,
  propose,
  reviewAndSend,
  setUpProfile,
  signIn,
  signUp,
  stateTag,
} from './support/flows'
import { en, fill } from './support/wording'

test('a replaced invitation link stops working, and the new one opens the proposal', async ({
  person,
}) => {
  const ana = await person('Ana')
  const bruno = await person('Bruno')
  await signUp(ana)
  const { link: first } = await propose(ana, bruno, [
    { from: 'me', kind: 'ITEM', description: 'A set of garden chairs' },
  ])

  // Ana makes a new link, which replaces the one she had. It asks the same
  // question as the composer, the same way: naming him is expected, and an
  // empty field is not taken for a link for anyone.
  const card = ana.page.getByRole('region', { name: en.invitationLink.heading, exact: true })
  await card.getByRole('button', { name: en.invitationLink.reissue }).click()
  const panel = card.getByRole('group', { name: en.invitationLink.reissue })
  await expect(panel.getByText(en.invitationLink.forNoContact)).toBeVisible()
  await expect(panel.getByRole('button', { name: en.invitationLink.forAnyone })).toBeVisible()
  await panel.getByRole('button', { name: en.invitationLink.reissue }).click()
  await expect(panel.getByText(en.invitationLink.forMissing)).toBeVisible()
  await panel.getByLabel(en.invitationLink.forLabel, { exact: true }).fill(bruno.email)
  await panel.getByRole('button', { name: en.invitationLink.reissue }).click()
  const field = card.getByLabel(en.invitationLink.linkLabel, { exact: true })
  await expect(field).toHaveValue(/\/en\/i#/)
  await expect(field).not.toHaveValue(first)
  const second = await field.inputValue()

  // Signed out, the old link and the new one open the same page: neither
  // says anything about itself until someone signs in. (A fresh page each
  // time: only the fragment differs, which a browser would not reload for.)
  const { page } = bruno
  const signedOut = async (link: string) => {
    await page.goto('about:blank')
    await page.goto(link)
    await expect(
      page.getByRole('heading', { name: en.invitation.signedOutTitle, level: 1 }),
    ).toBeVisible()
    return page.locator('main').innerHTML()
  }
  expect(await signedOut(first)).toBe(await signedOut(second))

  // Signed in, the old one shows nothing of the proposal.
  await signedOut(first)
  await signIn(bruno)
  await expect(page.getByText(en.errors.INVITATION_UNAVAILABLE)).toBeVisible()
  await expect(page.getByText('A set of garden chairs')).toHaveCount(0)
  await expect(page.getByRole('button', { name: en.invitation.respondNew })).toHaveCount(0)

  // The new one works.
  await page.goto('about:blank')
  await page.goto(second)
  await expect(page.getByText('A set of garden chairs', { exact: true })).toBeVisible()
  await page.getByRole('button', { name: en.invitation.respondNew, exact: true }).click()
  await setUpProfile(bruno)
  await page.waitForURL(/\/exchanges\/[0-9a-f-]{36}$/)
  await expect(
    page.getByRole('heading', { name: fill(en.exchange.title, { name: ana.name }), level: 1 }),
  ).toBeVisible()
  await expect(stateTag(page)).toHaveText(en.states.NEGOTIATING)
})

test('a second invitation link pasted into a tab showing one opens its own proposal', async ({
  person,
}) => {
  const ana = await person('Ana')
  const bruno = await person('Bruno')
  await signUp(ana)
  const { link: first } = await propose(ana, bruno, [
    { from: 'me', kind: 'ITEM', description: 'A set of garden chairs' },
  ])
  const { id: secondId, link: second } = await propose(ana, bruno, [
    { from: 'me', kind: 'ITEM', description: 'A folding ladder' },
  ])

  const { page } = bruno
  await page.goto(first)
  await signIn(bruno)
  await expect(page.getByText('A set of garden chairs', { exact: true })).toBeVisible()
  await expect(page).toHaveURL(/\/en\/i$/)

  // The second link, pasted into the same tab: only the fragment differs, so
  // the browser does not load the page again, which the mark set here shows.
  await page.evaluate(() => {
    ;(window as { samePage?: boolean }).samePage = true
  })
  await page.evaluate((link) => {
    window.location.href = link
  }, second)
  await expect(page.getByText('A folding ladder', { exact: true })).toBeVisible()
  await expect(page.getByText('A set of garden chairs', { exact: true })).toHaveCount(0)
  await expect(page).toHaveURL(/\/en\/i$/)
  expect(await page.evaluate(() => (window as { samePage?: boolean }).samePage)).toBe(true)

  // Responding claims the second.
  await page.getByRole('button', { name: en.invitation.respondNew, exact: true }).click()
  await setUpProfile(bruno)
  await page.waitForURL(`**/exchanges/${secondId}`)
})

test('a used link takes the person who used it back to the exchange, and claims nothing for anyone else', async ({
  person,
}) => {
  const ana = await person('Ana')
  const bruno = await person('Bruno')
  const carla = await person('Carla')
  await signUp(ana)
  const { id, link } = await propose(ana, bruno, [
    { from: 'me', kind: 'ITEM', description: 'A set of garden chairs' },
  ])
  await join(bruno, link)

  // Bruno comes back to the message and opens the link again: it takes him
  // to the exchange he joined.
  await bruno.page.goto('about:blank')
  await bruno.page.goto(link)
  await bruno.page.waitForURL(`**/exchanges/${id}`)

  // Carla, signed in, opens the same link: it is spent for her, nothing on
  // the page takes it, and Bruno is still the one Ana is dealing with.
  await signUp(carla)
  await carla.page.goto(link)
  await expect(carla.page.getByText(en.errors.INVITATION_UNAVAILABLE)).toBeVisible()
  await expect(carla.page.getByText('A set of garden chairs')).toHaveCount(0)
  await expect(carla.page.getByRole('button', { name: en.invitation.respondNew })).toHaveCount(0)
  await expect(
    carla.page.getByRole('button', { name: fill(en.invitation.respondAs, { name: carla.name }) }),
  ).toHaveCount(0)
  await expect(carla.page).toHaveURL(/\/en\/i$/)
  await carla.page.goto('/')
  await expect(carla.page.getByRole('heading', { name: en.home.title, level: 1 })).toBeVisible()
  await expect(carla.page.getByText(en.home.empty)).toBeVisible()
})

test('named, as the composer expects: the person it names can propose changes as soon as they sign in, with nobody to confirm them', async ({
  person,
}) => {
  const ana = await person('Ana')
  const bruno = await person('Bruno')
  const carla = await person('Carla')
  await signUp(ana)
  const { id, link } = await propose(
    ana,
    bruno,
    [{ from: 'me', kind: 'ITEM', description: 'A set of garden chairs' }],
    { invitee: bruno.email },
  )

  // Someone signed in with another address is told it is for someone else
  // and how to put that right, and cannot take it.
  await carla.page.goto(link)
  await signIn(carla)
  await carla.page.getByRole('button', { name: en.invitation.respondNew, exact: true }).click()
  await setUpProfile(carla)
  await expect(carla.page.getByText(en.errors.INVITATION_NOT_FOR_YOU)).toBeVisible()
  await expect(carla.page).toHaveURL(/\/en\/i$/)

  // Bruno, signed in with the address Ana gave, is told he can respond in
  // full, and is in the exchange as soon as he responds.
  await bruno.page.goto(link)
  await signIn(bruno)
  await expect(
    bruno.page.getByText(fill(en.invitation.introSignedIn, { name: ana.name })),
  ).toBeVisible()
  await expect(bruno.page.getByText(en.invitation.boundSignedIn)).toBeVisible()
  await bruno.page.getByRole('button', { name: en.invitation.respondNew, exact: true }).click()
  await setUpProfile(bruno)
  await bruno.page.waitForURL(`**/exchanges/${id}`)
  await expect(stateTag(bruno.page)).toHaveText(en.states.NEGOTIATING)
  await expect(bruno.page.getByText(en.claimant.limits)).toHaveCount(0)
  await expect(
    bruno.page.getByText(fill(en.exchange.waitingConfirmation, { name: ana.name })),
  ).toHaveCount(0)

  // He proposes changes straight away; nobody confirmed him.
  await bruno.page
    .getByRole('region', { name: en.exchange.proposalHeading, exact: true })
    .getByRole('link', { name: en.exchange.counter })
    .click()
  await expect(
    bruno.page.getByRole('heading', { name: en.composer.titleCounter, level: 1 }),
  ).toBeVisible()
  await composerItem(bruno.page, 1)
    .getByLabel(en.composer.descriptionLabel)
    .fill('A set of garden chairs, with cushions')
  await reviewAndSend(bruno.page)
  await expect(
    bruno.page
      .getByRole('region', { name: en.exchange.proposalHeading, exact: true })
      .getByText(en.exchange.sentByYou),
  ).toBeVisible()

  // Ana was never asked who opened it, and sees his version.
  await ana.page.goto(`/exchanges/${id}`)
  await expect(ana.page.getByRole('heading', { name: en.exchange.claimedHeading })).toHaveCount(0)
  await expect(
    ana.page.getByRole('button', { name: en.exchange.confirmCounterparty }),
  ).toHaveCount(0)
  await expect(
    ana.page.getByText('A set of garden chairs, with cushions', { exact: true }).first(),
  ).toBeVisible()
})
