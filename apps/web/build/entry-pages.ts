import { readFileSync } from 'node:fs'
import { join } from 'node:path'

import type { Plugin } from 'vite'

import { preloadedFontHrefs, withFontPreloads } from './font-preload.ts'
import { readLegalPages, withDocument } from './legal-pages.ts'
import { asSmsOptInPage, readSmsOptInPages } from './sms-opt-in.ts'
import { withThemeScript } from './theme-script.ts'

/*
 * One static entry page per language (DESIGN.md §4.2, §13.5).
 *
 * A messaging app builds a link preview from the page behind the link, without
 * running scripts and without knowing who will read it. An invitation link is
 * `/{language}/i#{token}`, in the sender's language, so each language gets its
 * own copy of the entry page at `{language}/i/index.html` with the preview
 * text in that language. The pages differ in nothing else and load the same
 * app.
 *
 * The languages and the text come from `packages/shared/wording`, so adding a
 * language there adds its page here with no change to the web app. The
 * preview text is fixed wording: it never names a person, a term or an
 * amount, and the token is in the fragment, which is never sent to a server.
 */

export interface EntryPage {
  /** Where the page is written, relative to the build output. */
  fileName: string
  lang: string
  dir: string
  title: string
  description: string
  /** What search engines are asked to do with the page. */
  robots: string
}

interface LanguageEntry {
  code: string
  direction: string
}

interface PreviewWording {
  productName: string
  tagline: string
  linkPreview: { title: string; description: string }
}

/** The path of a language's invitation page, as a link to it is written. */
export function invitationPath(language: string): string {
  return `/${language}/i`
}

/**
 * The pages to build: the app's own entry page in the default language,
 * which is the first one listed, and an invitation page for every language.
 */
export function entryPages(
  languages: readonly LanguageEntry[],
  wordingOf: (code: string) => PreviewWording,
): EntryPage[] {
  const [first] = languages
  const home = wordingOf(first.code)
  return [
    {
      fileName: 'index.html',
      lang: first.code,
      dir: first.direction,
      title: home.productName,
      description: home.tagline,
      robots: 'noindex',
    },
    ...languages.map((language) => {
      const { linkPreview } = wordingOf(language.code)
      return {
        fileName: `${language.code}/i/index.html`,
        lang: language.code,
        dir: language.direction,
        title: linkPreview.title,
        description: linkPreview.description,
        robots: 'noindex',
      }
    }),
  ]
}

function escapeHtml(text: string): string {
  return text
    .replaceAll('&', '&amp;')
    .replaceAll('<', '&lt;')
    .replaceAll('>', '&gt;')
    .replaceAll('"', '&quot;')
}

/**
 * Fills the `{{…}}` markers in `index.html` for one page, and puts the script
 * that applies the chosen appearance first in its head (`theme-script.ts`).
 */
export function renderEntryPage(template: string, page: EntryPage): string {
  const values: Record<string, string> = {
    lang: page.lang,
    dir: page.dir,
    title: page.title,
    description: page.description,
    robots: page.robots,
  }
  return withThemeScript(
    template.replace(/\{\{(\w+)\}\}/g, (marker, name: string) =>
      name in values ? escapeHtml(values[name]) : marker,
    ),
  )
}

/** Reads the pages to build from a wording directory. */
export function readEntryPages(wordingDirectory: string): EntryPage[] {
  const read = (name: string) =>
    JSON.parse(readFileSync(join(wordingDirectory, name), 'utf8')) as unknown
  return entryPages(
    read('languages.json') as LanguageEntry[],
    (code) => read(`${code}.json`) as PreviewWording,
  )
}

/**
 * The Vite plugin. The build writes every page; the dev server fills in the
 * page that matches the address asked for, so a preview can be checked
 * without building.
 */
export function entryPagesPlugin(wordingDirectory: string): Plugin {
  let base = '/'
  return {
    name: 'exchange:entry-pages',
    enforce: 'post',

    configResolved(config) {
      base = config.base
    },

    transformIndexHtml(html, context) {
      // The build fills the markers in `generateBundle`, once per page.
      if (!context.server) return html
      const pages = readEntryPages(wordingDirectory)
      const path = (context.originalUrl ?? '/').split(/[?#]/)[0].replace(/\/$/, '')
      const legal = readLegalPages(wordingDirectory).find((candidate) => candidate.path === path)
      if (legal) return withDocument(renderEntryPage(html, legal), legal)
      const page = pages.find((candidate) => `/${candidate.fileName}` === `${path}/index.html`)
      return renderEntryPage(html, page ?? pages[0])
    },

    // A static host has to answer `/{language}/i` with that language's page,
    // not with the app's own entry page as it does for other unknown paths.
    // `vite preview` does not do that unasked, so it is told to here, which
    // keeps a local preview of the build honest about what a link previews as.
    configurePreviewServer(server) {
      const paths = new Set([
        ...readEntryPages(wordingDirectory)
          .slice(1)
          .map((page) => invitationPath(page.lang)),
        ...readLegalPages(wordingDirectory).map((page) => page.path),
        ...readSmsOptInPages(wordingDirectory).map((page) => page.path),
      ])
      server.middlewares.use((request, _response, next) => {
        const [path, query] = (request.url ?? '').split('?')
        const page = path.replace(/\/$/, '')
        if (paths.has(page)) request.url = `${page}/index.html${query ? `?${query}` : ''}`
        next()
      })
    },

    generateBundle(_options, bundle) {
      const built = bundle['index.html']
      if (!built || built.type !== 'asset') return
      const template = String(built.source)
      const [home, ...invitations] = readEntryPages(wordingDirectory)
      built.source = renderEntryPage(template, home)
      // The invitation pages preload the fonts their first screen is drawn
      // in, by the names the build gave them (`font-preload.ts`).
      const fonts = preloadedFontHrefs(
        Object.values(bundle).filter((output) => output.type === 'asset'),
        base,
      )
      for (const page of invitations) {
        this.emitFile({
          type: 'asset',
          fileName: page.fileName,
          source: withFontPreloads(renderEntryPage(template, page), fonts),
        })
      }
      // The privacy policy and the terms, written into their pages (`legal-pages.ts`).
      for (const page of readLegalPages(wordingDirectory)) {
        this.emitFile({
          type: 'asset',
          fileName: page.fileName,
          source: withDocument(renderEntryPage(template, page), page),
        })
      }
      // How people opt in to texts, a page of its own (`sms-opt-in.ts`).
      for (const page of readSmsOptInPages(wordingDirectory)) {
        this.emitFile({
          type: 'asset',
          fileName: page.fileName,
          source: asSmsOptInPage(renderEntryPage(template, page), page),
        })
      }
    },
  }
}
