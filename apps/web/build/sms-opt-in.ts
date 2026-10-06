import { readFileSync } from 'node:fs'
import { join } from 'node:path'

import { staticPagePath } from '../../../packages/shared/src/legal-text.ts'

/*
 * "How people opt in to texts from Yuppers.app": the page the SMS
 * provider's reviewers and the carriers are given (docs/deploy-render.md,
 * "Text messages"), at `/sms-opt-in` and `/{language}/sms-opt-in`, open to
 * anyone and readable without scripts.
 *
 * It shows each step a person takes to receive texts: a picture of each
 * screen, taken from the web app by `npm run screenshots:sms`
 * (`e2e/screenshots/`) and kept in `public/sms-opt-in/`, and beside it the
 * exact wording that screen shows, read here from the same wording files
 * the app reads, so the two cannot drift apart. The texts themselves are
 * shown as text: the confirmation and an update from the service's own
 * wording (`sms` in the wording files), and the HELP and STOP replies that
 * Twilio is configured to send (`wording/sms-opt-in/`, and
 * docs/deploy-render.md).
 *
 * Unlike the privacy policy's and the terms' pages, it is not the app's page
 * with a document in it: the app has no page at this address, so the entry
 * page's script is left out and the page is all there is. It keeps the
 * app's stylesheet and adds one of its own, `/sms-opt-in/page.css`; nothing
 * is inline, so the Content-Security-Policy is the same as every page's,
 * and the pictures come from this origin.
 */

/** The number the pictures show: a US number reserved for fiction (555-01XX). */
export const SAMPLE_PHONE = '+12015550123'

/** An exchange's address as an update links to it, with an ID made up for the page. */
export const SAMPLE_LINK = 'https://yuppers.app/exchanges/0f8fad5b-d9cb-469f-a165-70867728950e'

/** The pictures, by step, as `public/sms-opt-in/` holds them for the default language. */
export const SCREENSHOTS = {
  signIn: '1-sign-in.webp',
  codeSent: '2-code-sent.webp',
  textUpdates: '3-text-updates.webp',
  confirmation: '4-confirmation.webp',
} as const

export type Screen = keyof typeof SCREENSHOTS

/** Where a language's pictures are, under the web build's public files. */
export function screenshotPath(screen: Screen, language: string, defaultLanguage: string): string {
  return language === defaultLanguage
    ? `/sms-opt-in/${SCREENSHOTS[screen]}`
    : `/sms-opt-in/${language}/${SCREENSHOTS[screen]}`
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

/** The page's own words, `wording/sms-opt-in/{language}.json`. */
export interface SmsOptInWording {
  title: string
  description: string
  intro: string
  note: string
  wordingHeading: string
  messageFrom: string
  messageTo: string
  programsHeading: string
  programs: string[]
  steps: Record<Screen | 'confirmationText' | 'replies', Step>
  codeText: string
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
  common: { skipToContent: string }
  help: { link: string }
  privacy: { link: string; policy: string; sms: string; smsLink: string }
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
    submit: string
    changeIdentifier: string
  }
  smsUpdates: {
    heading: string
    intro: string
    consent: string
    save: string
    on: string
  }
  sms: { signIn: string; update: string; optInConfirmation: string }
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

/** A list of what a screen says, word for word. */
function quoted(heading: string, lines: string[], links = false): string {
  const items = lines
    .map((line) => `<li><q>${links ? linked(line) : escapeHtml(line)}</q></li>`)
    .join('')
  return `<div class="wording"><h3>${escapeHtml(heading)}</h3><ul>${items}</ul></div>`
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
  const steps = page.steps

  const screenshot = (screen: Screen) =>
    `<img src="${screenshotPath(screen, language, defaultLanguage)}" alt="${escapeHtml(steps[screen].alt ?? '')}" width="${SCREENSHOT_WIDTH}" loading="lazy" decoding="async">`
  const step = (id: string, screen: Screen | null, title: string, caption: string, body: string) =>
    `<section class="step" aria-labelledby="${id}"><h2 id="${id}">${escapeHtml(title)}</h2><p>${escapeHtml(caption)}</p>${
      screen ? `<div class="shot">${screenshot(screen)}</div>` : ''
    }${body}</section>`

  const signIn = quoted(page.wordingHeading, [
    s.title,
    s.intro,
    s.identifierLabel,
    fill(s.identifierHintCountries, { codes: '+1' }),
    s.sendCode,
    `${product.privacy.sms} ${product.privacy.smsLink}`,
    product.privacy.policy,
    product.termsOfUse.document,
  ])
  const codeSent =
    quoted(page.wordingHeading, [
      fill(s.codeSent, { identifier: SAMPLE_PHONE }),
      s.codeLabel,
      s.codeHint,
      s.resendSoon,
      s.submit,
      s.changeIdentifier,
    ]) +
    `<p>${escapeHtml(page.codeText)}</p>` +
    bubble(fill(product.sms.signIn, { code: '123456', productName: product.productName }), page.messageFrom)
  const textUpdates = quoted(page.wordingHeading, [u.heading, u.intro, u.consent, u.save], true)
  const confirmation = quoted(page.wordingHeading, [fill(u.on, { phone: masked(SAMPLE_PHONE) })])
  const confirmationText = bubble(product.sms.optInConfirmation, page.messageFrom)
  const replies = [
    bubble('HELP', page.messageTo, true),
    bubble(page.replies.help, page.messageFrom),
    bubble('STOP', page.messageTo, true),
    bubble(page.replies.stop, page.messageFrom),
  ].join('')
  const update = `<p>${escapeHtml(page.updateText)}</p>${bubble(fill(product.sms.update, { link: SAMPLE_LINK }), page.messageFrom)}`

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
    `<section aria-labelledby="programs"><h2 id="programs">${escapeHtml(page.programsHeading)}</h2><ul>${page.programs.map((program) => `<li>${escapeHtml(program)}</li>`).join('')}</ul></section>`,
    step('sign-in', 'signIn', steps.signIn.title, steps.signIn.caption, signIn),
    step('code-sent', 'codeSent', steps.codeSent.title, steps.codeSent.caption, codeSent),
    step('text-updates', 'textUpdates', steps.textUpdates.title, steps.textUpdates.caption, textUpdates),
    step('confirmation', 'confirmation', steps.confirmation.title, steps.confirmation.caption, confirmation),
    step('confirmation-text', null, steps.confirmationText.title, steps.confirmationText.caption, confirmationText),
    step('help-and-stop', null, steps.replies.title, steps.replies.caption, replies + update),
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
 * markup in `#root`, the app's script and the chunks it preloads left out,
 * and the page's own stylesheet beside the app's.
 */
export function asSmsOptInPage(html: string, page: SmsOptInPage): string {
  const root = '<div id="root"></div>'
  if (!html.includes(root)) throw new Error('the entry page has no empty #root to fill')
  return html
    .replace(/\s*<script\b[^>]*>\s*<\/script>/g, '')
    .replace(/\s*<link rel="modulepreload"[^>]*>/g, '')
    .replace('</head>', '  <link rel="stylesheet" href="/sms-opt-in/page.css">\n  </head>')
    .replace(root, `<div id="root">${page.markup}</div>`)
}
