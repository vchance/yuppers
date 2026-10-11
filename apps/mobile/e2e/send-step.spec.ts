import { ApiPerson } from './support/api'
import { expect, test, type Person } from './support/fixtures'
import { button, exchangeIdOf, signUp, title, turnOn } from './support/flows'
import { en, fill } from './support/wording'

/*
 * Sending the invitation is a step of its own after signing (DESIGN.md §8),
 * on the app as on the web. Here the person who starts the exchange uses
 * the app, end to end from the list through the composer; what they open
 * to send the link is noted in place of the other app the phone would open.
 */

const PHONE = '(202) 555-0142'

/** What the app asked the device to open: a text message, an email, WhatsApp. */
async function opened(person: Person): Promise<string[]> {
  return person.page.evaluate(() => (window as unknown as { opened: string[] }).opened ?? [])
}

/** Writes a first proposal for Dana, named by phone number, and lands on the step. */
async function proposeToDana(ben: Person): Promise<string> {
  const { page } = ben
  // React Native's web build opens an outside address in a new window;
  // here the address is kept instead.
  await page.addInitScript(() => {
    const seen = ((window as unknown as { opened: string[] }).opened = [] as string[])
    window.open = (url) => {
      seen.push(String(url))
      return null
    }
  })
  await signUp(ben)
  await button(page, en.home.start).click()
  await button(page, en.templates.blank.name).click()
  await expect(title(page, en.composer.titleFirst)).toBeVisible()
  await page.getByLabel(en.composer.otherName, { exact: true }).fill('Dana')
  await page.getByLabel(en.invitationLink.forLabel, { exact: true }).fill(PHONE)
  await button(page, en.composer.addYours).click()
  await page.getByLabel(en.composer.descriptionLabel, { exact: true }).fill('A standing desk')
  await button(page, en.composer.review).click()
  await expect(title(page, en.composer.signTitle)).toBeVisible()
  await turnOn(page, en.consent.agree)
  await page.getByTestId('consent-sign').click()
  await expect(title(page, 'Send it to Dana')).toBeVisible()
  return exchangeIdOf(page)
}

test('named by phone: “Text the link” opens Messages with the number and the message, and the service records it', async ({
  person,
}) => {
  const ben = await person('Ben')
  const { page } = ben
  const id = await proposeToDana(ben)

  // The rule, who the link is for, and the ways to send it, led by a text
  // message to her number; "later" kept apart as a plain choice.
  await expect(
    page.getByText('Yuppers doesn’t send it for you. Dana gets nothing until you share this link.'),
  ).toBeVisible()
  await expect(
    page.getByText(fill(en.invitationLink.boundSummary, { identifier: PHONE })),
  ).toBeVisible()
  const link = (await page.getByLabel(en.invitationLink.linkLabel, { exact: true }).textContent())!
  expect(link).toMatch(/\/en\/i#.+/)
  await expect(button(page, en.invitationLink.sendText)).toBeVisible()
  await expect(button(page, en.invitationLink.sendWhatsApp)).toBeVisible()
  await expect(button(page, en.invitationLink.copy)).toBeVisible()
  await expect(button(page, en.invitationLink.sendDone)).toHaveCount(0)
  await expect(page.getByRole('link', { name: en.invitationLink.later })).toBeVisible()
  await expect(page.getByText(fill(en.invitationLink.laterHint, { name: 'Dana' }))).toBeVisible()

  // Texting it: Messages is opened with her number and the message, the
  // step says what happens next, and the way on appears.
  await button(page, en.invitationLink.sendText).click()
  const body = encodeURIComponent(fill(en.invitationLink.shareMessage, { link }))
  expect(await opened(ben)).toEqual([`sms:+12025550142?body=${body}`])
  await expect(
    page.getByText(fill(en.invitationLink.sharedNotice, { name: 'Dana' })),
  ).toBeVisible()
  await expect(page.getByRole('link', { name: en.invitationLink.later })).toHaveCount(0)

  // The service recorded it, for this screen and the list.
  const benElsewhere = await ApiPerson.signUp('Ben', ben.email)
  const view = await benElsewhere.view(id)
  expect(typeof view.invitation_shared_at).toBe('string')

  // Done: the exchange, saying the link was shared.
  await button(page, en.invitationLink.sendDone).click()
  await expect(title(page, fill(en.exchange.title, { name: 'Dana' }))).toBeVisible()
  await expect(page.getByText(/^You shared the link on /)).toBeVisible()
  await expect(page.getByText(/You haven’t sent them the link/)).toHaveCount(0)

  // In the list, the yup waits for Dana.
  await page.goto('/')
  await expect(title(page, en.home.title)).toBeVisible()
  await expect(page.getByRole('button', { name: /\. Waiting for Dana\. Reference / })).toBeVisible()
})

test('“I’ll send it later”: the yup reminds until the link is sent, and the list says it was not', async ({
  person,
}) => {
  const ben = await person('Ben')
  const { page } = ben
  const id = await proposeToDana(ben)
  await page.getByRole('link', { name: en.invitationLink.later }).click()

  // The reminder: headed with who has not joined, saying that nothing
  // reaches her, with the link still at hand and the ways to send it.
  await expect(title(page, fill(en.exchange.title, { name: 'Dana' }))).toBeVisible()
  await expect(
    page.getByRole('heading', { name: fill(en.invitationLink.notJoined, { name: 'Dana' }) }),
  ).toBeVisible()
  await expect(
    page.getByText(
      'You haven’t sent them the link. Yuppers doesn’t send it for you: Dana gets nothing until you do.',
    ),
  ).toBeVisible()
  await expect(button(page, en.invitationLink.sendText)).toBeVisible()
  await expect(button(page, en.invitationLink.reissue)).toBeVisible()
  const benElsewhere = await ApiPerson.signUp('Ben', ben.email)
  expect((await benElsewhere.view(id)).invitation_shared_at ?? null).toBeNull()

  // In the list, the yup is marked as not sent.
  await page.goto('/')
  await expect(title(page, en.home.title)).toBeVisible()
  const card = page.getByRole('button', {
    name: new RegExp(`^${en.states.NEGOTIATING}\\. ${en.home.notSent}\\. Reference `),
  })
  await expect(card).toBeVisible()
  await expect(card.getByText(en.home.notSent)).toBeVisible()

  // Opened again, the link is gone, as it is shown once: the reminder
  // stays, and a new link is the way to send it.
  await card.click()
  await expect(
    page.getByRole('heading', { name: fill(en.invitationLink.notJoined, { name: 'Dana' }) }),
  ).toBeVisible()
  await expect(page.getByText(en.invitationLink.sendAgain)).toBeVisible()
  await expect(button(page, en.invitationLink.sendText)).toHaveCount(0)
})
