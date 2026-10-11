import type { Locator, Page } from '@playwright/test'

import { codeFrom } from './codes'
import { expect, type Person } from './fixtures'
import { en, fill } from './wording'

/*
 * The steps most tests go through on the way to what they are about, done
 * the way a person does them: through the screens, in English unless said
 * otherwise.
 */

type Wording = typeof en

export type Kind = 'ITEM' | 'SERVICE' | 'TASK' | 'MONEY' | 'OTHER'

export interface ItemSpec {
  /** Who provides it, from the composer's author's point of view. */
  from: 'me' | 'them'
  kind: Kind
  description: string
  /** For money, in the exchange's currency, as typed. */
  amount?: string
  /** A due date, `YYYY-MM-DD`; due on signing unless given. */
  due?: string
}

const UUID = /\/exchanges\/([0-9a-f-]{36})$/

export function exchangeIdOf(page: Page): string {
  const match = UUID.exec(new URL(page.url()).pathname)
  if (!match) throw new Error(`not on an exchange page: ${page.url()}`)
  return match[1]
}

// ---- Signing in ----------------------------------------------------------------

/**
 * Signs in from a sign-in form already on the page, reading the code from
 * the API's log. The field is labelled for an email address or phone number
 * unless `label` says otherwise.
 */
export async function signIn(
  person: Person,
  w: Wording = en,
  label: string = w.signIn.identifierLabel,
): Promise<void> {
  const { page } = person
  await page.getByLabel(label, { exact: true }).fill(person.email)
  const code = await codeFrom(person.email, 'sign-in', () =>
    page.getByRole('button', { name: w.signIn.sendCode, exact: true }).click(),
  )
  await page.getByLabel(w.signIn.codeLabel).fill(code)
  await page.getByRole('button', { name: w.signIn.submit, exact: true }).click()
}

/** The profile a new account is asked for: a name and being 18 or over. */
export async function setUpProfile(person: Person, w: Wording = en): Promise<void> {
  const { page } = person
  await expect(page.getByRole('heading', { name: w.profile.firstTitle })).toBeVisible()
  await page.getByLabel(w.profile.nameLabel, { exact: true }).fill(person.name)
  await page.getByLabel(w.profile.adultLabel).check()
  await page.getByRole('button', { name: w.profile.continue, exact: true }).click()
}

/** A new account, signed in and set up, on the list of exchanges. */
export async function signUp(person: Person): Promise<void> {
  const { page } = person
  await page.goto('/')
  await expect(page.getByRole('heading', { name: en.signIn.title, level: 1 })).toBeVisible()
  await signIn(person)
  await setUpProfile(person)
  await expect(page.getByRole('heading', { name: en.home.title, level: 1 })).toBeVisible()
}

// ---- Writing and sending terms -------------------------------------------------

/**
 * Opens the composer from the list, on the blank form. Nothing is made on
 * the service yet: the draft is made the first time something is written,
 * and `draftId` waits for that.
 */
export async function startExchange(person: Person): Promise<void> {
  const { page } = person
  await page.goto('/')
  await page.getByRole('button', { name: en.home.start }).click()
  // New yup opens on the choices; the blank form is the composer as it was.
  await page.getByRole('button', { name: en.templates.blank.name, exact: true }).click()
  await page.waitForURL(/\/new\/blank$/)
  await expect(page.getByRole('heading', { name: en.composer.titleFirst, level: 1 })).toBeVisible()
}

/** The id of the draft once something written has made it: the address moves to the exchange. */
export async function draftId(page: Page): Promise<string> {
  await page.waitForURL(UUID)
  return exchangeIdOf(page)
}

/** One item's fieldset in the composer, by its position counting from 1. */
export function composerItem(page: Page, number: number): Locator {
  return page.getByRole('group', { name: fill(en.composer.itemLegend, { number }), exact: true })
}

/** Adds items to the working copy in the composer. */
export async function addItems(page: Page, items: readonly ItemSpec[]): Promise<void> {
  const fieldsets = page.locator('form fieldset').filter({
    has: page.getByRole('combobox', { name: en.composer.typeLabel }),
  })
  for (const item of items) {
    const number = (await fieldsets.count()) + 1
    await page
      .getByRole('button', {
        name: item.from === 'me' ? en.composer.addYours : en.composer.addTheirs,
      })
      .click()
    const fieldset = composerItem(page, number)
    await fieldset.getByLabel(en.composer.typeLabel).selectOption(item.kind)
    await fieldset.getByLabel(en.composer.descriptionLabel).fill(item.description)
    if (item.kind === 'MONEY') {
      await fieldset.getByLabel(/^Amount in /).fill(item.amount ?? '10')
    }
    if (item.due) {
      await fieldset.getByLabel(en.composer.dueLabel).selectOption('DATE')
      await fieldset.getByLabel(en.composer.dateLabel).fill(item.due)
    }
  }
}

/** The signing step: review, tick the consent box, sign and send. */
export async function reviewAndSend(page: Page): Promise<void> {
  await page.getByRole('button', { name: en.composer.review, exact: true }).click()
  await expect(page.getByRole('heading', { name: en.composer.signTitle, level: 1 })).toBeVisible()
  await page.getByLabel(en.consent.agree).check()
  await page.getByRole('button', { name: en.composer.signAndSend, exact: true }).click()
}

export interface Proposal {
  id: string
  /** The invitation link, as shown once to the initiator. */
  link: string
}

/**
 * Who a first proposal's invitation is for, in the composer: naming them by
 * `address`, as the composer expects, or, with `null`, choosing a link for
 * anyone instead, which says what that costs.
 */
export async function inviteFor(page: Page, address: string | null): Promise<void> {
  if (address !== null) {
    await page.getByLabel(en.invitationLink.forLabel, { exact: true }).fill(address)
    return
  }
  await page.getByRole('button', { name: en.invitationLink.forAnyone, exact: true }).click()
  await expect(page.getByText(en.invitationLink.forAnyoneText)).toBeVisible()
}

/**
 * A first proposal, from a new exchange to the invitation link. The link is
 * for anyone, chosen on purpose, unless `invitee` names who it is for. The
 * step that sends the link is left with "I’ll send it later", so the
 * initiator ends on the exchange's page, with the link in hand here: in
 * these tests the other person is given it directly.
 */
export async function propose(
  initiator: Person,
  other: { name: string },
  items: readonly ItemSpec[],
  { invitee = null }: { invitee?: string | null } = {},
): Promise<Proposal> {
  const { page } = initiator
  await startExchange(initiator)
  await page.getByLabel(en.composer.otherName).fill(other.name)
  const id = await draftId(page)
  await inviteFor(page, invitee)
  await addItems(page, items)
  await reviewAndSend(page)
  await expect(
    page.getByRole('heading', { name: fill(en.invitationLink.sendTitle, { name: other.name }) }),
  ).toBeVisible()
  const field = page.getByLabel(en.invitationLink.linkLabel, { exact: true })
  await expect(field).toHaveValue(/\/en\/i#/)
  const link = await field.inputValue()
  await page.getByRole('button', { name: en.invitationLink.later, exact: true }).click()
  await expect(invitationCard(page, other.name)).toBeVisible()
  return { id, link }
}

/**
 * The initiator's card about the invitation while nobody has joined through
 * it, headed with who has not joined.
 */
export function invitationCard(page: Page, otherName: string): Locator {
  return page.getByRole('region', {
    name: fill(en.invitationLink.notJoined, { name: otherName }),
    exact: true,
  })
}

// ---- Joining -------------------------------------------------------------------

/**
 * Opens an invitation link signed out, signs up on the page it opens, reads
 * the proposal, responds and lands on the exchange.
 */
export async function join(person: Person, link: string, w: Wording = en): Promise<void> {
  const { page } = person
  await page.goto(link)
  await expect(
    page.getByRole('heading', { name: w.invitation.signedOutTitle, level: 1 }),
  ).toBeVisible()
  await signIn(person, w)
  await expect(page.getByRole('heading', { name: w.invitation.title, level: 1 })).toBeVisible()
  await page.getByRole('button', { name: w.invitation.respondNew, exact: true }).click()
  await setUpProfile(person, w)
  await page.waitForURL(UUID)
}

/** The initiator says the person who opened the link is the one they meant. */
export async function confirmClaimant(initiator: Person): Promise<void> {
  const { page } = initiator
  await page.reload()
  await page.getByRole('button', { name: en.exchange.confirmCounterparty }).click()
  await expect(page.getByRole('heading', { name: en.exchange.claimedHeading })).toBeHidden()
}

/** Signs the revision waiting on this person. */
export async function acceptOpen(person: Person): Promise<void> {
  const { page } = person
  const open = page.getByRole('region', {
    name: new RegExp(`^(${en.exchange.proposalHeading}|${en.exchange.amendmentHeading})$`),
  })
  await open.getByRole('button', { name: en.exchange.accept, exact: true }).click()
  const panel = open.getByRole('group', { name: en.exchange.signHeading })
  await panel.getByLabel(en.consent.agree).check()
  await panel.getByRole('button', { name: en.exchange.accept, exact: true }).click()
  // Signed: there is nothing left for this person to sign.
  await expect(panel).toBeHidden()
  await expect(open.getByRole('button', { name: en.exchange.accept, exact: true })).toBeHidden()
}

/**
 * Two people with an agreement in force: `initiator` proposes with a link for
 * anyone, `other` joins through it and signs, and `initiator` confirms them,
 * the longer way round that still has to work. Both end on the
 * exchange's page.
 */
export async function agree(
  initiator: Person,
  other: Person,
  items: readonly ItemSpec[],
): Promise<string> {
  await signUp(initiator)
  const { id, link } = await propose(initiator, other, items)
  await join(other, link)
  await acceptOpen(other)
  await confirmClaimant(initiator)
  await expect(stateTag(initiator.page)).toHaveText(en.states.ACTIVE)
  await other.page.reload()
  await expect(stateTag(other.page)).toHaveText(en.states.ACTIVE)
  return id
}

/**
 * Two people negotiating: `initiator` has proposed to `other` by name, as the
 * composer expects, and `other` has joined without signing. Named, they need
 * no confirming, so they can respond in full as soon as they sign in. Both
 * end on the exchange's page.
 */
export async function negotiate(
  initiator: Person,
  other: Person,
  items: readonly ItemSpec[],
): Promise<string> {
  await signUp(initiator)
  const { id, link } = await propose(initiator, other, items, { invitee: other.email })
  await join(other, link)
  await expect(other.page.getByText(en.claimant.limits)).toBeHidden()
  await initiator.page.reload()
  await expect(
    initiator.page.getByRole('heading', { name: en.exchange.claimedHeading }),
  ).toHaveCount(0)
  return id
}

// ---- The exchange page ---------------------------------------------------------

/** The tag saying where the exchange stands. */
export function stateTag(page: Page): Locator {
  return page.locator('main .tags .tag').first()
}

/** The agreement in force. */
export function agreement(page: Page): Locator {
  return page.getByRole('region', { name: en.exchange.agreementHeading, exact: true })
}

/** One item of the agreement in force, by its description. */
export function agreedItem(page: Page, description: string): Locator {
  return agreement(page)
    .locator('li.contribution')
    .filter({ has: page.getByText(description, { exact: true }) })
}

/**
 * Takes a step on an item of the agreement: marking it delivered, confirming
 * it, disputing it. `label` is the button's wording, which is also the
 * panel's title and the button that sends it.
 */
export async function move(
  page: Page,
  description: string,
  label: string,
  note?: string,
): Promise<void> {
  const item = agreedItem(page, description)
  await item.getByRole('button', { name: label, exact: true }).click()
  const panel = item.getByRole('group', { name: label, exact: true })
  if (note !== undefined) await panel.getByRole('textbox').fill(note)
  await panel.getByRole('button', { name: label, exact: true }).click()
  await expect(panel).toBeHidden()
}

/** The history at the foot of the exchange page. */
export function history(page: Page): Locator {
  return page.getByRole('region', { name: en.record.historyHeading, exact: true })
}
