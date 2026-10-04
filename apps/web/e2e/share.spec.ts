import { expect, test } from './support/fixtures'
import { addItems, reviewAndSend, signUp, startExchange } from './support/flows'
import { en, fill } from './support/wording'

/*
 * Passing the invitation link on from a desktop browser, which has no share
 * sheet of its own (`support/fixtures.ts` takes `navigator.share` away). The
 * service sends nothing: "Share link" opens the app's own ways to share,
 * the link copies, and its QR code is drawn on the page, within the
 * Content-Security-Policy the fixtures hold every page to.
 */

test('a first proposal’s link is shared from a desktop browser: copied, and shown as a QR code', async ({
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

  const field = page.getByLabel(en.invitationLink.linkLabel, { exact: true })
  await expect(field).toHaveValue(/\/en\/i#.+/)
  const link = await field.inputValue()
  await expect(page.getByText(en.invitationLink.intro)).toBeVisible()

  // No share sheet: one button, which opens the ways to share in place.
  await expect(page.getByRole('button', { name: en.invitationLink.copy })).toHaveCount(0)
  const share = page.getByRole('button', { name: en.invitationLink.share, exact: true })
  await share.click()
  const panel = page.getByRole('group', { name: en.invitationLink.share, exact: true })
  await expect(panel).toBeFocused()

  await panel.getByRole('button', { name: en.invitationLink.copy }).click()
  await expect(page.getByText(en.invitationLink.copied, { exact: true })).toBeVisible()
  expect(await page.evaluate(() => navigator.clipboard.readText())).toBe(link)

  // An email to Carla, with the link in its body; a text message and
  // WhatsApp the same. Each is opened by the device, not sent from here.
  const message = fill(en.invitationLink.shareMessage, { link })
  const body = encodeURIComponent(message)
  const subject = encodeURIComponent(en.linkPreview.title)
  const href = (name: string | RegExp) => panel.getByRole('link', { name }).getAttribute('href')
  expect(await href(en.invitationLink.shareEmail)).toBe(
    `mailto:carla@example.test?subject=${subject}&body=${body}`,
  )
  expect(await href(en.invitationLink.shareSms)).toBe(`sms:?body=${body}`)
  expect(await href(new RegExp(`^${en.invitationLink.shareWhatsApp}`))).toBe(
    `https://wa.me/?text=${body}`,
  )

  // The QR code, drawn on the page for someone in the room to scan.
  await panel.getByRole('button', { name: en.invitationLink.shareQr }).click()
  const code = panel.getByRole('img', { name: en.invitationLink.qrLabel })
  await expect(code).toBeVisible()
  const box = await code.boundingBox()
  expect(box!.width).toBeGreaterThan(150)
  expect(Math.round(box!.width)).toBe(Math.round(box!.height))
  await expect(panel.getByText(en.invitationLink.qrHint)).toBeVisible()

  // Closing gives the keyboard back to "Share link".
  await panel.getByRole('button', { name: en.invitationLink.closeShare }).click()
  await expect(panel).toBeHidden()
  await expect(share).toBeFocused()
})
