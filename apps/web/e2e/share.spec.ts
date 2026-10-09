import { expect, test } from './support/fixtures'
import { addItems, reviewAndSend, signUp, startExchange } from './support/flows'
import { en, fill } from './support/wording'

/*
 * Passing the invitation link on from a desktop browser, which has no share
 * sheet of its own (`support/fixtures.ts` takes `navigator.share` away). The
 * service sends nothing: the ways to send the link are real buttons and
 * links on the step after signing, led by the one that reaches the person
 * named, the link copies, and its QR code is drawn on the page, within the
 * Content-Security-Policy the fixtures hold every page to.
 */

test('a first proposal’s link is sent from a desktop browser: an email to the person, copied, and shown as a QR code', async ({
  person,
}) => {
  const ana = await person('Ana')
  const { page } = ana
  await signUp(ana)
  await startExchange(ana)
  await page.getByLabel(en.composer.otherName).fill('Carla')
  // Who it is for, asked beside their name.
  await page.getByLabel(en.invitationLink.forLabel, { exact: true }).fill('carla@example.test')
  await addItems(page, [{ from: 'me', kind: 'ITEM', description: 'A bookshelf' }])
  await reviewAndSend(page)

  // The step that sends it, headed with who it is for.
  await expect(page.getByRole('heading', { name: 'Send it to Carla', level: 1 })).toBeVisible()
  const field = page.getByLabel(en.invitationLink.linkLabel, { exact: true })
  await expect(field).toHaveValue(/\/en\/i#.+/)
  const link = await field.inputValue()
  await expect(page.getByText(en.invitationLink.intro)).toBeVisible()

  // Named by email address: an email to Carla first, with the message and
  // the link in its body, opened by the device, not sent from here.
  const message = fill(en.invitationLink.shareMessage, { link })
  const body = encodeURIComponent(message)
  const subject = encodeURIComponent(en.linkPreview.title)
  const email = page.getByRole('link', { name: en.invitationLink.sendEmail })
  await expect(email).toHaveClass('button primary')
  await expect(email).toHaveAttribute(
    'href',
    `mailto:carla@example.test?subject=${subject}&body=${body}`,
  )
  await expect(page.getByRole('link', { name: en.invitationLink.sendText })).toHaveCount(0)

  // Copying, beside it.
  await page.getByRole('button', { name: en.invitationLink.copy }).click()
  await expect(page.getByText(en.invitationLink.copied, { exact: true })).toBeVisible()
  expect(await page.evaluate(() => navigator.clipboard.readText())).toBe(link)

  // The QR code, drawn on the page for someone in the room to scan.
  await page.getByRole('button', { name: en.invitationLink.shareQr }).click()
  const code = page.getByRole('img', { name: en.invitationLink.qrLabel })
  await expect(code).toBeVisible()
  const box = await code.boundingBox()
  expect(box!.width).toBeGreaterThan(150)
  expect(Math.round(box!.width)).toBe(Math.round(box!.height))
  await expect(page.getByText(en.invitationLink.qrHint)).toBeVisible()
  await page.getByRole('button', { name: en.invitationLink.hideQr }).click()
  await expect(code).toBeHidden()

  // Having opened a way to send it, the person goes on to the exchange.
  await page.getByRole('button', { name: en.invitationLink.sendDone }).click()
  await expect(page.getByRole('heading', { name: 'Yup with Carla', level: 1 })).toBeVisible()
})
