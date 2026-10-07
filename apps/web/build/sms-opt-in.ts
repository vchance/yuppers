import { readFileSync } from 'node:fs'
import { join } from 'node:path'

import { staticPagePath } from '../../../packages/shared/src/legal-text.ts'

/*
 * "How people opt in to texts from Yuppers.app": the page the SMS
 * provider's reviewers and the carriers are given (docs/deploy-render.md,
 * "Text messages"), at `/sms-opt-in` and `/{language}/sms-opt-in`, open to
 * anyone and readable without scripts.
 *
 * Yuppers.app has one text-message program, "Yuppers.app agreement
 * updates", and the page is first of all about it: each step a person takes
 * to receive the updates, in two parts with a contents list above them: on
 * the website (`#website`), with a picture of each screen taken from the
 * web app by `npm run screenshots:sms` (`e2e/screenshots/`) and kept in
 * `public/sms-opt-in/`; and in the mobile app (`#mobile-app`), with the
 * app's own screens taken by `npm run screenshots:sms:mobile`
 * (`apps/mobile/e2e/screenshots/`) and kept in `public/sms-opt-in/mobile/`.
 * Each part has the updates' form and HELP and STOP (UPDATE_FORMS).
 *
 * One-time codes by text are not part of that program: Twilio Verify sends
 * and checks them, from Twilio's own senders, in Twilio's own template
 * (`backend/src/notifications/verify.rs`). The three forms that text one
 * (CODE_FORMS: signing in by text, confirming a phone number, deleting an
 * account by text) follow in a section of their own, `#one-time-codes`,
 * again on the website and in the app, with their pictures and consent
 * wording, and the message Verify sends (VERIFY_SAMPLE, and for deleting
 * an account VERIFY_DELETION_SAMPLE) in place of a text of ours.
 *
 * Every form has an anchor of its own (`formAnchor`), the same as before
 * the codes moved, so a reviewer can be given a link to one. Beside each
 * picture is the exact wording that screen shows, read here from the same
 * wording files the apps read, so the two cannot drift apart. The texts
 * themselves are shown as text: the confirmation and an update from the
 * service's own wording (`sms` in the wording files), and the HELP and STOP
 * replies that Twilio is configured to send for the updates' number
 * (`wording/sms-opt-in/`, and docs/deploy-render.md).
 *
 * Unlike the privacy policy's and the terms' pages, it is not the app's page
 * with a document in it: the app has no page at this address, so the entry
 * page's script is left out and the page is all there is. It keeps the
 * app's stylesheet and adds one of its own, `/sms-opt-in/page.css`; nothing
 * is inline, so the Content-Security-Policy is the same as every page's,
 * and the pictures come from this origin.
 */

/** The number the pictures sign in with and delete: a US number reserved for fiction (555-01XX). */
export const SAMPLE_PHONE = '+12015550123'

/**
 * The number the pictures add to an account with a code ("Confirming a
 * phone number"): another reserved for fiction, so that the codes the two
 * flows ask for are counted against different numbers (`npm run
 * screenshots:sms`, and the service's limit of codes per number per hour).
 */
export const SAMPLE_PHONE_TO_ADD = '+12015550124'

/** The one-time code the texts show. */
export const SAMPLE_CODE = '123456'

/**
 * The names the two Twilio Verify services are given (docs/deploy-render.md,
 * "One-time codes by Twilio Verify"), which Verify's template puts in its
 * message: one for signing in and confirming a number, and one for
 * deleting an account, so that a deletion code never reads like a sign-in
 * code.
 */
export const VERIFY_SERVICE_NAME = 'Yuppers.app'
export const VERIFY_DELETION_SERVICE_NAME = 'Yuppers.app account deletion'

/**
 * The message Twilio Verify sends with a code, in English: its default SMS
 * template, "Your {{friendly_name}} verification code is: {{code}}", as
 * Twilio's API reference shows it (the Service resource's `AppHash` and
 * `DoNotShareWarningEnabled` examples, "Your AppName verification code is:
 * 1234"). In another language Twilio sends its own translation, which its
 * public documentation does not print.
 */
export const VERIFY_SAMPLE = `Your ${VERIFY_SERVICE_NAME} verification code is: ${SAMPLE_CODE}`

/** The same, from the service for deleting an account. */
export const VERIFY_DELETION_SAMPLE = `Your ${VERIFY_DELETION_SERVICE_NAME} verification code is: ${SAMPLE_CODE}`

/** An exchange's address as an update links to it, with an ID made up for the page. */
export const SAMPLE_LINK = 'https://yuppers.app/exchanges/0f8fad5b-d9cb-469f-a165-70867728950e'

/**
 * The pictures, by step, as `public/sms-opt-in/` holds them for the default
 * language. Signing in: the form with a number entered and the box beside
 * it unticked, the same with the box ticked and "Send code" enabled, and the
 * code sent. Agreement updates: "Text updates" before and after. Confirming
 * a number being added in "Text updates", and deleting an account with a
 * code by text: each with the box unticked, ticked, and the code sent.
 */
export const SCREENSHOTS = {
  signIn: '1-sign-in.webp',
  boxTicked: '2-box-ticked.webp',
  codeSent: '3-code-sent.webp',
  textUpdates: '4-text-updates.webp',
  confirmation: '5-confirmation.webp',
  confirmNumber: '6-confirm-number.webp',
  confirmNumberTicked: '7-confirm-number-ticked.webp',
  confirmNumberCodeSent: '8-confirm-number-code-sent.webp',
  deleteAccount: '9-delete-account.webp',
  deleteAccountTicked: '10-delete-account-ticked.webp',
  deleteAccountCodeSent: '11-delete-account-code-sent.webp',
} as const

export type Screen = keyof typeof SCREENSHOTS

/** A step without a picture: a text as it arrives. */
type TextStep = 'confirmationText'

/** Any step: one with a picture, or a text. */
export type StepName = Screen | TextStep

/** The forms, in the page's order, each with its steps; HELP and STOP has none. */
export const FORMS = {
  agreementUpdates: ['textUpdates', 'confirmation', 'confirmationText'],
  replies: [],
  signIn: ['signIn', 'boxTicked', 'codeSent'],
  confirmNumber: ['confirmNumber', 'confirmNumberTicked', 'confirmNumberCodeSent'],
  deleteAccount: ['deleteAccount', 'deleteAccountTicked', 'deleteAccountCodeSent'],
} as const satisfies Record<string, readonly (Screen | TextStep)[]>

export type Form = keyof typeof FORMS

/** The agreement-updates program's forms: each part's own, first on the page. */
export const UPDATE_FORMS = ['agreementUpdates', 'replies'] as const satisfies readonly Form[]

/** The forms that text a one-time code through Twilio Verify, in the section after. */
export const CODE_FORMS = ['signIn', 'confirmNumber', 'deleteAccount'] as const satisfies readonly Form[]

/** The anchor of the one-time codes' section, and of each part of it. */
export const CODES_ANCHOR = 'one-time-codes'

/** Each form's anchor, after its part's: `#website-sign-in`, `#mobile-app-sign-in`. */
const FORM_SLUGS: Record<Form, string> = {
  signIn: 'sign-in',
  confirmNumber: 'confirm-number',
  deleteAccount: 'delete-account',
  agreementUpdates: 'agreement-updates',
  replies: 'help-and-stop',
}

/** Each step's anchor on the website; in the app, the same after `mobile-`. */
const STEP_IDS: Record<Screen | TextStep, string> = {
  signIn: 'sign-in',
  boxTicked: 'box-ticked',
  codeSent: 'code-sent',
  confirmNumber: 'confirm-number',
  confirmNumberTicked: 'confirm-number-ticked',
  confirmNumberCodeSent: 'confirm-number-code-sent',
  deleteAccount: 'delete-account',
  deleteAccountTicked: 'delete-account-ticked',
  deleteAccountCodeSent: 'delete-account-code-sent',
  textUpdates: 'text-updates',
  confirmation: 'confirmation',
  confirmationText: 'confirmation-text',
}

/**
 * Where the pictures come from: the website (`npm run screenshots:sms`) or
 * the mobile app (`npm run screenshots:sms:mobile`), which has the same
 * steps under the same file names in `mobile/`.
 */
export type Surface = 'web' | 'mobile'

/** The anchor of each part: the website's and the app's. */
export const PART_ANCHORS: Record<Surface, string> = { web: 'website', mobile: 'mobile-app' }

/** The parts, in the page's order. */
export const SURFACES: readonly Surface[] = ['web', 'mobile']

/** A form's anchor in a part, such as `website-confirm-number`. */
export function formAnchor(form: Form, surface: Surface): string {
  return `${PART_ANCHORS[surface]}-${FORM_SLUGS[form]}`
}

/** The anchor of a part of the one-time codes' section: `website-one-time-codes`. */
export function codesAnchor(surface: Surface): string {
  return `${PART_ANCHORS[surface]}-${CODES_ANCHOR}`
}

/** A step's anchor in a part, such as `confirm-number` or `mobile-confirm-number`. */
export function stepAnchor(step: Screen | TextStep, surface: Surface): string {
  return surface === 'mobile' ? `mobile-${STEP_IDS[step]}` : STEP_IDS[step]
}

/** Where a language's pictures are, under the web build's public files. */
export function screenshotPath(
  screen: Screen,
  language: string,
  defaultLanguage: string,
  surface: Surface = 'web',
): string {
  const base = surface === 'mobile' ? '/sms-opt-in/mobile' : '/sms-opt-in'
  return language === defaultLanguage
    ? `${base}/${SCREENSHOTS[screen]}`
    : `${base}/${language}/${SCREENSHOTS[screen]}`
}

/** The pixel size the pictures are taken at: a phone's width, at twice the density. */
export const SCREENSHOT_WIDTH = 390

interface LanguageEntry {
  code: string
  name: string
  direction: string
}

interface Step {
  title: string
  caption: string
  alt?: string
}

/** A form's heading, and what it is for, the same on the website and in the app. */
interface FormWording {
  heading: string
  intro: string
}

/** The page's own words, `wording/sms-opt-in/{language}.json`. */
export interface SmsOptInWording {
  title: string
  description: string
  intro: string
  note: string
  /** The contents list's name: each part, and each form in it. */
  contentsLabel: string
  websiteHeading: string
  websiteIntro: string
  mobileHeading: string
  mobileIntro: string
  /** What the app's release state is, and so where its pictures come from. */
  mobileRelease: string
  /** That the app's consent texts and texts are the website's. */
  mobileSame: string
  wordingHeading: string
  messageFrom: string
  messageTo: string
  /** The program, agreement updates, and nothing else. */
  programsHeading: string
  programs: string[]
  /** That one-time codes are not part of it, and where they are shown. */
  codesNote: string
  /** The one-time codes' section: its heading, what it is, and its two parts. */
  codesHeading: string
  codesIntro: string
  codesWebsiteHeading: string
  codesMobileHeading: string
  forms: Record<Form, FormWording>
  steps: Record<Screen | TextStep, Step>
  /** The same steps in the mobile app. */
  mobileSteps: Record<Screen | TextStep, Step>
  /**
   * Before the message Twilio Verify sends with a code, after the "code
   * sent" screens of signing in and confirming a number.
   */
  verifyText: string
  /** The same, after the "code sent" screen of deleting an account. */
  verifyDeletionText: string
  /** After it: that it is Twilio's template, and how other languages get it. */
  verifyNote: string
  /** Whom that message is from. */
  messageFromVerify: string
  updateText: string
  moreHeading: string
  more: string
  termsLink: string
  privacyLink: string
  /** The replies Twilio sends, as configured there. */
  replies: { help: string; stop: string }
}

/** What the page quotes from the product's wording, `wording/{language}.json`. */
interface ProductWording {
  productName: string
  common: { skipToContent: string; cancel: string }
  help: { link: string }
  privacy: { link: string; policy: string; smsLink: string }
  termsOfUse: { link: string; document: string }
  signIn: {
    title: string
    intro: string
    identifierLabel: string
    identifierHintCountries: string
    sendCode: string
    codeSent: string
    codeLabel: string
    codeHint: string
    resendSoon: string
    resend: string
    submit: string
    changeIdentifier: string
  }
  smsUpdates: {
    heading: string
    intro: string
    addPhoneIntro: string
    phoneLabel: string
    phoneHint: string
    sendCode: string
    codeSent: string
    codeLabel: string
    addPhone: string
    changePhone: string
    consent: string
    save: string
    on: string
    howItWorks: string
  }
  deletion: {
    heading: string
    codeIntro: string
    sendCode: string
    codeSent: string
    continue: string
  }
  smsCode: { signIn: string; deleteAccount: string; verifyNumber: string; tickToSend: string }
  sms: {
    update: string
    optInConfirmation: string
  }
}

/** One language's page. */
export interface SmsOptInPage {
  fileName: string
  path: string
  lang: string
  dir: string
  title: string
  description: string
  robots: string
  markup: string
}

function escapeHtml(text: string): string {
  return text
    .replaceAll('&', '&amp;')
    .replaceAll('<', '&lt;')
    .replaceAll('>', '&gt;')
    .replaceAll('"', '&quot;')
}

/** A message with its `{name}` variables filled in. */
function fill(message: string, values: Record<string, string>): string {
  return message.replace(/\{(\w+)\}/g, (marker, name: string) => values[name] ?? marker)
}

/** The phone number as the screens show it once it is on the account. */
function masked(phone: string): string {
  return `+1 •••-•••-${phone.slice(-4)}`
}

/** Text, with any web address in it made a link that reads as the address. */
function linked(text: string): string {
  let out = ''
  let at = 0
  for (const found of text.matchAll(/https:\/\/\S+/g)) {
    const url = found[0].replace(/[.,;:]+$/, '')
    out += escapeHtml(text.slice(at, found.index))
    out += `<a href="${escapeHtml(url)}">${escapeHtml(url)}</a>`
    at = found.index + url.length
  }
  return out + escapeHtml(text.slice(at))
}

/** A list of what a screen says, word for word, under a heading of `level`. */
function quoted(heading: string, lines: string[], level: number): string {
  const items = lines.map((line) => `<li><q>${linked(line)}</q></li>`).join('')
  return `<div class="wording"><h${level}>${escapeHtml(heading)}</h${level}><ul>${items}</ul></div>`
}

/** A text message, as a phone shows it: from the service, or a reply. */
function bubble(text: string, label: string, reply = false): string {
  return `<figure class="sms${reply ? ' sms-reply' : ''}"><figcaption>${escapeHtml(label)}</figcaption><p>${escapeHtml(text)}</p></figure>`
}

/**
 * The page's markup in `language`, inside the header, skip link and footer
 * the app puts around every page.
 */
export function renderSmsOptInMarkup(
  page: SmsOptInWording,
  product: ProductWording,
  language: string,
  languages: readonly LanguageEntry[],
): string {
  const defaultLanguage = languages[0].code
  const legal = (document: 'privacy' | 'terms', section?: string) =>
    `${language === defaultLanguage ? `/${document}` : `/${language}/${document}`}${section ? `#${section}` : ''}`
  const s = product.signIn
  const u = product.smsUpdates
  const d = product.deletion
  const c = product.smsCode
  const text = (before: string, message: string, from = page.messageFrom) =>
    `<p>${escapeHtml(before)}</p>${bubble(message, from)}`
  // After each "code sent" screen: the message Twilio Verify sends, not a
  // text of ours, from the service for what the code is for.
  const verifyNote = `<p>${escapeHtml(page.verifyNote)}</p>`
  const verifyText = text(page.verifyText, VERIFY_SAMPLE, page.messageFromVerify) + verifyNote
  const verifyDeletionText =
    text(page.verifyDeletionText, VERIFY_DELETION_SAMPLE, page.messageFromVerify) + verifyNote

  // What each screen says, under a heading of `level`, and after a code is
  // sent, the text it comes in. The app's screens say what the website's
  // do, but for their own links: "How we text you" opens this page.
  const wording = (step: Screen | TextStep, surface: Surface, level: number): string => {
    const said = (lines: string[]) => quoted(page.wordingHeading, lines, level)
    switch (step) {
      case 'textUpdates':
        return said([
          u.heading,
          u.intro,
          u.consent,
          u.save,
          ...(surface === 'mobile' ? [u.howItWorks] : []),
        ])
      case 'confirmation':
        return said([fill(u.on, { phone: masked(SAMPLE_PHONE) })])
      case 'confirmationText':
        return bubble(product.sms.optInConfirmation, page.messageFrom)
      // The form with a number entered: the box beside it, unticked, and
      // "Send code" waiting for it; then the box ticked.
      case 'signIn':
        return said([
          s.title,
          s.intro,
          s.identifierLabel,
          fill(s.identifierHintCountries, { codes: '+1' }),
          c.signIn,
          c.tickToSend,
          s.sendCode,
          product.privacy.smsLink,
          product.privacy.policy,
          product.termsOfUse.document,
        ])
      case 'boxTicked':
        return said([c.signIn, s.sendCode])
      case 'codeSent':
        return (
          said([
            fill(s.codeSent, { identifier: SAMPLE_PHONE }),
            s.codeLabel,
            s.codeHint,
            s.resendSoon,
            s.submit,
            s.changeIdentifier,
          ]) + verifyText
        )
      // "Text updates" on an account with no number yet: the number, the box
      // beside it, and the code that checks it.
      case 'confirmNumber':
        return said([
          u.heading,
          u.intro,
          u.addPhoneIntro,
          u.phoneLabel,
          u.phoneHint,
          c.verifyNumber,
          c.tickToSend,
          u.sendCode,
          u.howItWorks,
        ])
      case 'confirmNumberTicked':
        return said([c.verifyNumber, u.sendCode])
      case 'confirmNumberCodeSent':
        return (
          said([
            fill(u.codeSent, { phone: masked(SAMPLE_PHONE_TO_ADD) }),
            u.codeLabel,
            s.codeHint,
            u.addPhone,
            u.changePhone,
          ]) + verifyText
        )
      // Deleting an account whose code goes to its phone number.
      case 'deleteAccount':
        return said([
          fill(d.codeIntro, { identifier: SAMPLE_PHONE }),
          c.deleteAccount,
          c.tickToSend,
          d.sendCode,
          product.common.cancel,
        ])
      case 'deleteAccountTicked':
        return said([c.deleteAccount, d.sendCode])
      case 'deleteAccountCodeSent':
        return (
          said([
            d.heading,
            fill(d.codeSent, { identifier: SAMPLE_PHONE }),
            s.codeLabel,
            s.codeHint,
            d.continue,
            s.resend,
            product.common.cancel,
          ]) + verifyDeletionText
        )
    }
  }

  /**
   * A step, under its form's heading: what it is, its picture, and what it
   * says. Its heading is of `level`, the wording's the next.
   */
  const step = (name: Screen | TextStep, surface: Surface, level: number) => {
    const { title, caption, alt } = (surface === 'mobile' ? page.mobileSteps : page.steps)[name]
    const id = stepAnchor(name, surface)
    const picture =
      name === 'confirmationText'
        ? ''
        : `<div class="shot shot-${surface}"><img src="${screenshotPath(name, language, defaultLanguage, surface)}" alt="${escapeHtml(alt ?? '')}" width="${SCREENSHOT_WIDTH}" loading="lazy" decoding="async"></div>`
    return `<section class="step"><h${level} id="${id}">${escapeHtml(title)}</h${level}><p>${escapeHtml(caption)}</p>${picture}${wording(name, surface, level + 1)}</section>`
  }

  const replies = [
    bubble('HELP', page.messageTo, true),
    bubble(page.replies.help, page.messageFrom),
    bubble('STOP', page.messageTo, true),
    bubble(page.replies.stop, page.messageFrom),
  ].join('')
  const update = text(page.updateText, fill(product.sms.update, { link: SAMPLE_LINK }))

  /**
   * One form in a part, under a heading of `level` of its own that the
   * contents list links to. On the website, HELP and STOP keeps the anchor
   * it had as a step, `#help-and-stop`, on its section. Forms and steps are
   * plain sections found by their headings, not landmarks: their names
   * repeat from one part to the other, and a landmark's must not.
   */
  const form = (name: Form, surface: Surface, level: number) => {
    const id = formAnchor(name, surface)
    const { heading, intro } = page.forms[name]
    const kept = name === 'replies' && surface === 'web' ? ' id="help-and-stop"' : ''
    const body =
      name === 'replies'
        ? replies + update
        : FORMS[name].map((each) => step(each, surface, level + 1)).join('')
    return `<section class="form"${kept}><h${level} id="${id}">${escapeHtml(heading)}</h${level}><p>${escapeHtml(intro)}</p>${body}</section>`
  }
  const forms = (names: readonly Form[], surface: Surface, level: number) =>
    names.map((name) => form(name, surface, level)).join('')

  /** One of the page's two parts, the website or the app: the agreement updates there. */
  const part = (surface: Surface, heading: string, body: string) => {
    const id = PART_ANCHORS[surface]
    return `<section class="part" aria-labelledby="${id}"><h2 id="${id}">${escapeHtml(heading)}</h2>${body}${forms(UPDATE_FORMS, surface, 3)}</section>`
  }

  const website = part('web', page.websiteHeading, `<p>${linked(page.websiteIntro)}</p>`)
  const mobile = part(
    'mobile',
    page.mobileHeading,
    [
      `<p>${escapeHtml(page.mobileIntro)}</p>`,
      `<p class="notice">${escapeHtml(page.mobileRelease)}</p>`,
      `<p>${escapeHtml(page.mobileSame)}</p>`,
    ].join(''),
  )

  /** The one-time codes, sent by Twilio Verify: the website's forms, then the app's. */
  const codesHeading = (surface: Surface) =>
    surface === 'web' ? page.codesWebsiteHeading : page.codesMobileHeading
  const codes = `<section class="part codes" aria-labelledby="${CODES_ANCHOR}"><h2 id="${CODES_ANCHOR}">${escapeHtml(page.codesHeading)}</h2><p>${escapeHtml(page.codesIntro)}</p>${SURFACES.map(
    (surface) =>
      `<section class="surface"><h3 id="${codesAnchor(surface)}">${escapeHtml(codesHeading(surface))}</h3>${forms(CODE_FORMS, surface, 4)}</section>`,
  ).join('')}</section>`

  const list = (names: readonly Form[], surface: Surface) =>
    names
      .map(
        (name) => `<li><a href="#${formAnchor(name, surface)}">${escapeHtml(page.forms[name].heading)}</a></li>`,
      )
      .join('')
  const contents = `<nav aria-label="${escapeHtml(page.contentsLabel)}" class="contents"><ul>${[
    ...SURFACES.map(
      (surface) =>
        `<li><a href="#${PART_ANCHORS[surface]}">${escapeHtml(surface === 'web' ? page.websiteHeading : page.mobileHeading)}</a><ul>${list(UPDATE_FORMS, surface)}</ul></li>`,
    ),
    `<li><a href="#${CODES_ANCHOR}">${escapeHtml(page.codesHeading)}</a><ul>${SURFACES.map(
      (surface) =>
        `<li><a href="#${codesAnchor(surface)}">${escapeHtml(codesHeading(surface))}</a><ul>${list(CODE_FORMS, surface)}</ul></li>`,
    ).join('')}</ul></li>`,
  ].join('')}</ul></nav>`

  const otherLanguages = languages
    .map((other) => {
      const current = other.code === language ? ' aria-current="page"' : ''
      const code = escapeHtml(other.code)
      return `<li><a href="${staticPagePath('sms-opt-in', other.code, defaultLanguage)}" lang="${code}" hreflang="${code}"${current}>${escapeHtml(other.name)}</a></li>`
    })
    .join('')
  const more = fill(escapeHtml(page.more), {
    terms: `<a href="${legal('terms', 'text-messages')}">${escapeHtml(page.termsLink)}</a>`,
    privacy: `<a href="${legal('privacy', 'text-messages')}">${escapeHtml(page.privacyLink)}</a>`,
  })

  return [
    `<a class="skip" href="#content">${escapeHtml(product.common.skipToContent)}</a>`,
    `<header class="site"><a href="/" class="brand">${escapeHtml(product.productName)}</a></header>`,
    '<main id="content" tabindex="-1">',
    // Cloudflare's email obfuscation would turn the support address in the
    // HELP reply into "[email protected]" for anyone reading without
    // scripts, which is everyone here; it leaves this part alone.
    '<!--email_off-->',
    '<article class="sms-opt-in">',
    `<h1>${escapeHtml(page.title)}</h1>`,
    `<p>${escapeHtml(page.intro)}</p>`,
    `<p class="notice">${escapeHtml(page.note)}</p>`,
    `<nav aria-label="${escapeHtml(page.title)}" class="legal-languages"><ul class="plain">${otherLanguages}</ul></nav>`,
    contents,
    `<section aria-labelledby="programs"><h2 id="programs">${escapeHtml(page.programsHeading)}</h2><ul>${page.programs.map((program) => `<li>${escapeHtml(program)}</li>`).join('')}</ul><p>${escapeHtml(page.codesNote)}</p></section>`,
    website,
    mobile,
    codes,
    `<section aria-labelledby="more"><h2 id="more">${escapeHtml(page.moreHeading)}</h2><p>${more}</p></section>`,
    '</article>',
    '<!--/email_off-->',
    '</main>',
    `<footer class="site"><a href="/help?lang=${encodeURIComponent(language)}">${escapeHtml(product.help.link)}</a><a href="${legal('privacy')}">${escapeHtml(product.privacy.link)}</a><a href="${legal('terms')}">${escapeHtml(product.termsOfUse.link)}</a></footer>`,
  ].join('')
}

/** The pages to build, one per language, from a wording directory. */
export function readSmsOptInPages(wordingDirectory: string): SmsOptInPage[] {
  const read = (name: string) =>
    JSON.parse(readFileSync(join(wordingDirectory, name), 'utf8')) as unknown
  const languages = read('languages.json') as LanguageEntry[]
  const defaultLanguage = languages[0].code
  return languages.map((language) => {
    const page = read(`sms-opt-in/${language.code}.json`) as SmsOptInWording
    const product = read(`${language.code}.json`) as ProductWording
    const path = staticPagePath('sms-opt-in', language.code, defaultLanguage)
    return {
      fileName: `${path.slice(1)}/index.html`,
      path,
      lang: language.code,
      dir: language.direction,
      title: `${page.title} · ${product.productName}`,
      description: page.description,
      robots: 'index, follow',
      markup: renderSmsOptInMarkup(page, product, language.code, languages),
    }
  })
}

/**
 * The built entry page made this page: its markers already filled in, the
 * markup in `#root`, the app's script, the theme's and the chunks it preloads
 * left out (so it follows the device's light or dark setting only),
 * and the page's own stylesheet beside the app's.
 */
export function asSmsOptInPage(html: string, page: SmsOptInPage): string {
  const root = '<div id="root"></div>'
  if (!html.includes(root)) throw new Error('the entry page has no empty #root to fill')
  return html
    .replace(/\s*<script\b[^>]*>[^<]*<\/script>/g, '')
    .replace(/\s*<link rel="modulepreload"[^>]*>/g, '')
    .replace('</head>', '  <link rel="stylesheet" href="/sms-opt-in/page.css">\n  </head>')
    .replace(root, `<div id="root">${page.markup}</div>`)
}
