import { defaultLanguage, languages } from './language'
import { LEGAL_DOCUMENTS, type LegalDocument } from './legal-text'

/*
 * Where the privacy policy and the terms are. Their text, and what fills
 * it in, is `legal-text.ts`.
 */

export {
  LEGAL_DOCUMENTS,
  LEGAL_EFFECTIVE_DATES,
  LEGAL_FIGURES,
  LEGAL_LINKS,
  LEGAL_SECTIONS,
  PRIVACY_EMAIL,
  STATIC_PAGES,
  SUPPORT_EMAIL,
  legalEffectiveDate,
  legalInline,
  legalSections,
  staticPagePath,
} from './legal-text'
export type {
  LegalBlock,
  LegalDocument,
  LegalInline,
  LegalLinkName,
  LegalSection,
  LegalSectionWording,
  LegalWording,
  PrivacySection,
  PrivacyWording,
  StaticPage,
  TermsSection,
  TermsWording,
} from './legal-text'

/**
 * The path of `document` in `language`: `/{document}` in the default
 * language, which is where anyone who names no language lands, and
 * `/{language}/{document}` in the others. The web build writes a static
 * page at each (`apps/web/build/legal-pages.ts`).
 */
export function legalPath(document: LegalDocument, language: string): string {
  return language === defaultLanguage ? `/${document}` : `/${language}/${document}`
}

/** The full address of a document, or of one of its sections, on the web app at `origin`. */
export function legalAddress(
  origin: string,
  document: LegalDocument,
  language: string,
  section?: string,
): string {
  return `${origin.replace(/\/+$/, '')}${legalPath(document, language)}${section ? `#${section}` : ''}`
}

/**
 * The document a path shows, and the language the path names: the default
 * for `/{document}`, and `{language}` for `/{language}/{document}` when that
 * is a language there is. `null` for any other path.
 */
export function legalPathOf(
  pathname: string,
): { document: LegalDocument; language: string; named: boolean } | null {
  const path = pathname.length > 1 ? pathname.replace(/\/+$/, '') : pathname
  const found = /^(?:\/([^/]+))?\/([^/]+)$/.exec(path)
  if (!found) return null
  const document = LEGAL_DOCUMENTS.find((name) => name === found[2])
  if (!document) return null
  if (found[1] === undefined) return { document, language: defaultLanguage, named: false }
  const wanted = found[1].toLowerCase()
  const language = languages.find((info) => info.code.toLowerCase() === wanted)?.code
  return language ? { document, language, named: true } : null
}
