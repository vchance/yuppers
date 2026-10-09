import { expect, test } from './support/fixtures'
import {
  addItems,
  invitationCard,
  reviewAndSend,
  signUp,
  startExchange,
  stateTag,
} from './support/flows'
import { en, fill } from './support/wording'

/*
 * Sending the invitation is a step of its own after signing (DESIGN.md §8).
 * Yuppers never sends it: the person who signed is asked to, with the way
 * that reaches the person named first, and can only say "I’ll send it
 * later" to go past it. What they open is recorded by the service, and the
 * exchange's page and the list say where the link stands until someone joins.
 */

const PHONE = '(202) 555-0142'
const ITEMS = [{ from: 'me' as const, kind: 'ITEM' as const, description: 'A standing desk' }]

test('named by phone: “Text the link” opens Messages with the number and the message, and the service records it', async ({
  person,
}) => {
  const ana = await person('Ana')
  const { page } = ana
  // A text message is opened by the device; here the browser is kept on the
  // page, and what it would have opened is noted instead.
  await page.addInitScript(() => {
    document.addEventListener('click', (event) => {
      const link = (event.target as Element).closest('a[href^="sms:"], a[href^="mailto:"]')
      if (!link) return
      event.preventDefault()
      ;(window as unknown as { opened: string[] }).opened ??= []
      ;(window as unknown as { opened: string[] }).opened.push(link.getAttribute('href')!)
    })
  })
  await signUp(ana)
  await startExchange(ana)
  await page.getByLabel(en.composer.otherName).fill('Dana')
  await page.getByLabel(en.invitationLink.forLabel, { exact: true }).fill(PHONE)
  await addItems(page, ITEMS)
  await reviewAndSend(page)

  // The step: headed with who it is for, saying the rule, leading with a
  // text message to her number, and "later" kept apart from the ways to send.
  await expect(page.getByRole('heading', { name: 'Send it to Dana', level: 1 })).toBeFocused()
  await expect(
    page.getByText('Yuppers doesn’t send it for you. Dana gets nothing until you share this link.'),
  ).toBeVisible()
  await expect(
    page.getByText(fill(en.invitationLink.boundSummary, { identifier: PHONE })),
  ).toBeVisible()
  const field = page.getByLabel(en.invitationLink.linkLabel, { exact: true })
  const link = await field.inputValue()
  const text = page.getByRole('link', { name: en.invitationLink.sendText, exact: true })
  await expect(text).toHaveClass('button primary')
  const body = encodeURIComponent(fill(en.invitationLink.shareMessage, { link }))
  await expect(text).toHaveAttribute('href', `sms:+12025550142?body=${body}`)
  await expect(page.getByRole('link', { name: /^Send on WhatsApp/ })).toHaveAttribute(
    'href',
    `https://wa.me/?text=${body}`,
  )
  await expect(page.getByRole('button', { name: en.invitationLink.copy })).toBeVisible()
  await expect(page.getByRole('button', { name: en.invitationLink.sendDone })).toHaveCount(0)
  const later = page.getByRole('button', { name: en.invitationLink.later, exact: true })
  await expect(later).toHaveClass('link')
  await expect(page.getByText(fill(en.invitationLink.laterHint, { name: 'Dana' }))).toBeVisible()

  // Every one of them is a comfortable target.
  for (const control of await page.locator('.share-actions > *, .send-later button').all()) {
    const box = await control.boundingBox()
    expect(box!.height).toBeGreaterThanOrEqual(44)
    expect(box!.width).toBeGreaterThanOrEqual(44)
  }

  // Texting it: the device opens Messages; the page says what happens next
  // and offers the way on, which has the keyboard.
  await text.click()
  expect(await page.evaluate(() => (window as unknown as { opened: string[] }).opened)).toEqual([
    `sms:+12025550142?body=${body}`,
  ])
  await expect(
    page.getByText(fill(en.invitationLink.sharedNotice, { name: 'Dana' })),
  ).toBeVisible()
  const done = page.getByRole('button', { name: en.invitationLink.sendDone, exact: true })
  await expect(done).toBeFocused()
  await expect(later).toHaveCount(0)
  await done.click()

  // The exchange, saying the link was shared; and still after a reload,
  // since the service recorded it.
  await expect(page.getByRole('heading', { name: 'Yup with Dana', level: 1 })).toBeVisible()
  const card = invitationCard(page, 'Dana')
  await expect(card.getByText(/^You shared the link on /)).toBeVisible()
  await expect(card).not.toHaveClass(/card-reminder/)
  await page.reload()
  await expect(invitationCard(page, 'Dana').getByText(/^You shared the link on /)).toBeVisible()
  await expect(page.getByText(en.invitationLink.notSentYet.slice(0, 30))).toHaveCount(0)

  // In the list, the yup waits for Dana.
  await page.goto('/')
  const chip = page.locator('.cards .chip')
  await expect(chip).toHaveText('Waiting for Dana')
})

test('“I’ll send it later”: the exchange reminds until the link is sent, and the list says it was not', async ({
  person,
}) => {
  const ana = await person('Ana')
  const { page } = ana
  await signUp(ana)
  await startExchange(ana)
  await page.getByLabel(en.composer.otherName).fill('Dana')
  await page.getByLabel(en.invitationLink.forLabel, { exact: true }).fill(PHONE)
  await addItems(page, ITEMS)
  await reviewAndSend(page)
  await expect(page.getByRole('heading', { name: 'Send it to Dana', level: 1 })).toBeVisible()
  await page.getByRole('button', { name: en.invitationLink.later, exact: true }).click()

  // The reminder: the first card on the exchange, headed with who has not
  // joined, saying that nothing reaches her, with the link still at hand
  // and the ways to send it.
  await expect(page.getByRole('heading', { name: 'Yup with Dana', level: 1 })).toBeVisible()
  await expect(stateTag(page)).toHaveText(en.states.NEGOTIATING)
  const card = invitationCard(page, 'Dana')
  await expect(card).toHaveClass(/card-reminder/)
  await expect(page.locator('main section.card').first()).toHaveAttribute(
    'aria-labelledby',
    'invitation-heading',
  )
  await expect(
    card.getByText(
      'You haven’t sent them the link. Yuppers doesn’t send it for you: Dana gets nothing until you do.',
    ),
  ).toBeVisible()
  await expect(card.getByRole('link', { name: en.invitationLink.sendText })).toHaveClass(
    'button primary',
  )
  await expect(card.getByRole('button', { name: en.invitationLink.reissue })).toBeVisible()

  // In the list, the yup is marked as not sent.
  await page.goto('/')
  const chip = page.locator('.cards .chip')
  await expect(chip).toHaveText(en.home.notSent)
  await expect(chip).toHaveClass('chip chip-pending')

  // Back on the exchange after a reload the link is gone, as it is shown
  // once: the reminder stays, and a new link is the way to send it.
  await page.goBack()
  await expect(invitationCard(page, 'Dana')).toHaveClass(/card-reminder/)
  await expect(page.getByText(en.invitationLink.sendAgain)).toBeVisible()
  await expect(page.getByRole('link', { name: en.invitationLink.sendText })).toHaveCount(0)
  await page.getByRole('button', { name: en.invitationLink.reissue }).click()
  const panel = page.getByRole('group', { name: en.invitationLink.reissue })
  await panel.getByLabel(en.invitationLink.forLabel, { exact: true }).fill(PHONE)
  await panel.getByRole('button', { name: en.invitationLink.reissue }).click()
  await expect(
    invitationCard(page, 'Dana').getByRole('link', { name: en.invitationLink.sendText }),
  ).toHaveClass('button primary')
})
