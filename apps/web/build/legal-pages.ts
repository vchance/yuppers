import { readFileSync } from 'node:fs'
import { join } from 'node:path'

import {
  LEGAL_DOCUMENTS,
  LEGAL_EFFECTIVE_DATES,
  legalEffectiveDate,
  legalInline,
  legalSections,
  staticPagePath,
  type LegalBlock,
  type LegalDocument,
  type LegalWording,
} from '../../../packages/shared/src/legal-text.ts'

/*
 * The privacy policy and the terms as static pages, one per document and
 * language: `{document}/index.html` in the default language and
 * `{language}/{document}/index.html` in the others, which the service
 * answers at `/{document}` and `/{language}/{document}`
 * (`backend/src/http/web.rs`).
 *
 * The web app is a single-page app, so its own page for each document is
 * written by scripts. Crawlers, the stores' and the SMS provider's
 * reviewers, and anyone with scripts off need the text in the page itself.
 * So each page is the app's entry page, with the same script and
 * stylesheet, and with the document already written inside `#root`: without
 * scripts it reads in full; with them, the app starts and shows the same
 * document, from the same text, as its own page. Nothing is inline, so the
 * Content-Security-Policy stays as it is.
 *
 * The text comes from `packages/shared/wording/{document}/{language}.json`,
 * filled in by the same code the app uses (`legal-text.ts`), so the two
 * cannot say different things.
 */

interface LanguageEntry {
  code: string
  name: string
  direction: string
}

/** What the page around the document says, from the product's wording. */
interface ChromeWording {
  productName: string
  common: { skipToContent: string }
  help: { link: string }
  privacy: { link: string }
  termsOfUse: { link: string }
}

/** One document's page in one language; an entry page (`entry-pages.ts`) with the document in it. */
export interface LegalPage {
  /** Where the page is written, relative to the build output. */
  fileName: string
  document: LegalDocument
  /** The page's address. */
  path: string
  lang: string
  dir: string
  title: string
  description: string
  robots: string
  /** What is written inside `#root`. */
  markup: string
}

/** The address of `document` in `language`, the default language's being `/{document}`. */
export function legalPagePath(
  document: LegalDocument,
  language: string,
  defaultLanguage: string,
): string {
  return language === defaultLanguage ? `/${document}` : `/${language}/${document}`
}

function escapeHtml(text: string): string {
  return text
    .replaceAll('&', '&amp;')
    .replaceAll('<', '&lt;')
    .replaceAll('>', '&gt;')
    .replaceAll('"', '&quot;')
}

interface Context {
  document: LegalDocument
  language: string
  defaultLanguage: string
  wording: LegalWording
}

/** A message of the document, filled in, with its addresses and links as links and its strong text strong. */
function inline(message: string, context: Context): string {
  return legalInline(message, context.language, context.document, context.wording)
    .map((piece) => {
      if ('email' in piece) {
        return `<a href="mailto:${escapeHtml(piece.email)}">${escapeHtml(piece.email)}</a>`
      }
      if ('page' in piece) {
        const path = staticPagePath(piece.page, context.language, context.defaultLanguage)
        return `<a href="${escapeHtml(path)}">${escapeHtml(piece.text)}</a>`
      }
      if ('document' in piece) {
        const path = legalPagePath(piece.document, context.language, context.defaultLanguage)
        const href = `${path}${piece.section ? `#${piece.section}` : ''}`
        return `<a href="${escapeHtml(href)}">${escapeHtml(piece.text)}</a>`
      }
      return piece.strong ? `<strong>${escapeHtml(piece.text)}</strong>` : escapeHtml(piece.text)
    })
    .join('')
}

function block(piece: LegalBlock, context: Context): string {
  if ('h' in piece) return `<h3>${inline(piece.h, context)}</h3>`
  if ('p' in piece) return `<p>${inline(piece.p, context)}</p>`
  return `<ul>${piece.ul.map((item) => `<li>${inline(item, context)}</li>`).join('')}</ul>`
}

/** The effective date's line, with the date itself marked as one. */
function effective(context: Context): string {
  const [before, after = ''] = context.wording.effective.split('{effectiveDate}')
  const date = `<time datetime="${LEGAL_EFFECTIVE_DATES[context.document]}">${escapeHtml(
    legalEffectiveDate(context.document, context.language),
  )}</time>`
  return `${inline(before, context)}${date}${inline(after, context)}`
}

/**
 * `document` in `language`, laid out as the app's page lays it out
 * (`src/screens/LegalPage.tsx`), with the header, skip link and footer the
 * app puts around every page.
 */
export function renderLegalMarkup(
  document: LegalDocument,
  wording: LegalWording,
  language: string,
  languages: readonly LanguageEntry[],
  chrome: ChromeWording,
): string {
  const defaultLanguage = languages[0].code
  const context: Context = { document, language, defaultLanguage, wording }
  const at = (other: LegalDocument) => legalPagePath(other, language, defaultLanguage)
  const sections = legalSections(document, wording)
  const otherLanguages = languages
    .map((other) => {
      const current = other.code === language ? ' aria-current="page"' : ''
      const code = escapeHtml(other.code)
      const href = legalPagePath(document, other.code, defaultLanguage)
      return `<li><a href="${href}" lang="${code}" hreflang="${code}"${current}>${escapeHtml(other.name)}</a></li>`
    })
    .join('')
  const contents = sections
    .map(({ id, section }) => `<li><a href="#${id}">${inline(section.title, context)}</a></li>`)
    .join('')
  const body = sections
    .map(
      ({ id, section }) =>
        `<section aria-labelledby="${id}"><h2 id="${id}" tabindex="-1">${inline(section.title, context)}</h2>${section.blocks.map((piece) => block(piece, context)).join('')}</section>`,
    )
    .join('')
  const footerLink = (other: LegalDocument, words: string) =>
    `<a href="${at(other)}"${other === document ? ' aria-current="page"' : ''}>${escapeHtml(words)}</a>`
  return [
    `<a class="skip" href="#content">${escapeHtml(chrome.common.skipToContent)}</a>`,
    `<header class="site"><a href="/" class="brand">${escapeHtml(chrome.productName)}</a></header>`,
    '<main id="content" tabindex="-1">',
    `<article class="legal">`,
    `<h1 tabindex="-1">${inline(wording.title, context)}</h1>`,
    `<p class="legal-note">${inline(wording.note, context)}</p>`,
    `<p class="hint">${effective(context)}</p>`,
    `<nav aria-label="${escapeHtml(wording.otherLanguages)}" class="legal-languages"><ul class="plain">${otherLanguages}</ul></nav>`,
    `<nav aria-labelledby="legal-contents" class="help-sections"><h2 id="legal-contents">${inline(wording.contents, context)}</h2><ol>${contents}</ol></nav>`,
    body,
    '</article>',
    '</main>',
    `<footer class="site"><a href="/help?lang=${encodeURIComponent(language)}">${escapeHtml(chrome.help.link)}</a>${footerLink('privacy', chrome.privacy.link)}${footerLink('terms', chrome.termsOfUse.link)}</footer>`,
  ].join('')
}

/** The pages to build, one per document and language, from a wording directory. */
export function readLegalPages(wordingDirectory: string): LegalPage[] {
  const read = (name: string) =>
    JSON.parse(readFileSync(join(wordingDirectory, name), 'utf8')) as unknown
  const languages = read('languages.json') as LanguageEntry[]
  const defaultLanguage = languages[0].code
  return LEGAL_DOCUMENTS.flatMap((document) =>
    languages.map((language) => {
      const wording = read(`${document}/${language.code}.json`) as LegalWording
      const chrome = read(`${language.code}.json`) as ChromeWording
      const path = legalPagePath(document, language.code, defaultLanguage)
      return {
        fileName: `${path.slice(1)}/index.html`,
        document,
        path,
        lang: language.code,
        dir: language.direction,
        title: `${wording.title} · ${chrome.productName}`,
        description: wording.description,
        robots: 'index, follow',
        markup: renderLegalMarkup(document, wording, language.code, languages, chrome),
      }
    }),
  )
}

/**
 * The app's entry page, its markers already filled in for `page`
 * (`renderEntryPage`), with the document written inside its empty `#root`.
 */
export function withDocument(html: string, page: LegalPage): string {
  const root = '<div id="root"></div>'
  if (!html.includes(root)) throw new Error('the entry page has no empty #root to fill')
  return html.replace(root, `<div id="root">${page.markup}</div>`)
}
