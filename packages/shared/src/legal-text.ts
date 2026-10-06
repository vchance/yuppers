/*
 * The privacy policy and the terms and conditions: the two documents that
 * say what Yuppers.app does with people's information and on what terms
 * Yuppers may be used. Each document's text is
 * `wording/{document}/{language}.json`, one file per language, apart from
 * the product's wording so that only the document's own pages load it, like
 * the help pages. The same text is shown two ways, both made from what is
 * here:
 *
 *   - by the web app's own page, `/{document}`, lazily loaded
 *     (`apps/web/src/screens/LegalPage.tsx`);
 *   - as static pages written by the web build, `/{document}` and
 *     `/{language}/{document}`, which read in full without JavaScript, for
 *     crawlers, the stores' and the SMS provider's reviewers, and anyone
 *     with scripts off (`apps/web/build/legal-pages.ts`).
 *
 * Each document says what the product does today. Whenever that changes,
 * the text changes with it, and so does its date in `LEGAL_EFFECTIVE_DATES`
 * (README, "Privacy policy and terms").
 *
 * This file imports nothing, because the web build reads it while it builds;
 * where the documents' pages are (`legalPath`) is in `legal.ts`, which knows
 * the languages.
 */

/** Where privacy questions and requests go. The one place the address is written. */
export const PRIVACY_EMAIL = 'privacy@yuppers.app'

/** Where questions about the terms, and requests for help, go. The one place the address is written. */
export const SUPPORT_EMAIL = 'support@yuppers.app'

/** The documents, by the path each has: `/privacy`, `/terms`. */
export const LEGAL_DOCUMENTS = ['privacy', 'terms'] as const

export type LegalDocument = (typeof LEGAL_DOCUMENTS)[number]

/** The day each document as written took effect, as an ISO date. */
export const LEGAL_EFFECTIVE_DATES: Record<LegalDocument, string> = {
  privacy: '2026-10-05',
  terms: '2026-10-05',
}

/**
 * The numbers the documents state that are the service's own rules, named
 * in the text as placeholders such as `{networkDays}`. `legal.test.ts` fails
 * if they drift from the backend's.
 */
export const LEGAL_FIGURES = {
  /** How long a signature's IP address and user agent are kept (`Rules::network_metadata_retention`). */
  networkDays: 90,
  /** How long a one-time code works (`AuthRules::code_ttl`). */
  codeMinutes: 10,
  /** How long a session lasts (`AuthRules::session_ttl`). */
  sessionDays: 30,
} as const

/**
 * Each document's sections, in order, by the anchor each has on the page.
 * The anchors are the same in every language, so a link to one, such as
 * `/privacy#text-messages`, works whichever language the page is read in.
 */
export const LEGAL_SECTIONS = {
  privacy: [
    'who-we-are',
    'what-we-collect',
    'cookies-and-storage',
    'who-sees-what',
    'text-messages',
    'how-long-we-keep-it',
    'deleting-your-account',
    'your-choices',
    'children',
    'security',
    'changes',
  ],
  terms: [
    'who-runs-yuppers',
    'who-can-use-it',
    'what-yuppers-is',
    'your-account',
    'acceptable-use',
    'reports-and-review',
    'your-content',
    'records-and-deletion',
    'text-messages',
    'changes-and-availability',
    'disclaimer',
    'ending',
    'contact',
  ],
} as const

export type PrivacySection = (typeof LEGAL_SECTIONS.privacy)[number]
export type TermsSection = (typeof LEGAL_SECTIONS.terms)[number]
export type LegalSection<D extends LegalDocument = LegalDocument> = (typeof LEGAL_SECTIONS)[D][number]

/**
 * Where a link inside a document may lead, by the placeholder that names
 * it: `{terms}` in the privacy policy, `{privacy}` and `{privacyTexts}` in
 * the terms. Each document's `links` gives the words of the link.
 */
export const LEGAL_LINKS = {
  terms: { document: 'terms' },
  privacy: { document: 'privacy' },
  privacyTexts: { document: 'privacy', section: 'text-messages' },
} as const satisfies Record<string, { document: LegalDocument; section?: string }>

export type LegalLinkName = keyof typeof LEGAL_LINKS

/**
 * One piece of a section, in order: a subheading, a paragraph or a list.
 * Every language has the same pieces in the same order, which the wording
 * check enforces.
 */
export type LegalBlock = { h: string } | { p: string } | { ul: string[] }

export interface LegalSectionWording {
  title: string
  blocks: LegalBlock[]
}

/**
 * A document's text in one language. A message may name `{privacyEmail}`
 * and `{supportEmail}`, which become links to write to, `{effectiveDate}`,
 * the figures in `LEGAL_FIGURES`, and the links in `links`, and nothing
 * else.
 */
export interface LegalWording<S extends string = string> {
  title: string
  /** One sentence, for the page's description. */
  description: string
  /** Said at the top, before anything else. */
  note: string
  /** Uses `{effectiveDate}`. */
  effective: string
  /** Heading over the list of sections. */
  contents: string
  /** Names the list of links to the document in each language. */
  otherLanguages: string
  /** The words of each link to another document that the text names. */
  links: Partial<Record<LegalLinkName, string>>
  sections: Record<S, LegalSectionWording>
}

export type PrivacyWording = LegalWording<PrivacySection>
export type TermsWording = LegalWording<TermsSection>

/** A document's effective date, written out in `language`, such as "October 5, 2026". */
export function legalEffectiveDate(document: LegalDocument, language: string): string {
  return new Intl.DateTimeFormat(language, { dateStyle: 'long', timeZone: 'UTC' }).format(
    new Date(`${LEGAL_EFFECTIVE_DATES[document]}T00:00:00Z`),
  )
}

/**
 * A piece of a message: text, an address to write to, or a link to a
 * document (and a section of it), each shown as a link.
 */
export type LegalInline =
  | { text: string; strong?: true }
  | { email: string }
  | { document: LegalDocument; section?: string; text: string }

/**
 * A message of `document`, filled in: every placeholder replaced, and the
 * addresses and links left as pieces of their own for the page to make
 * links of. Text between `**` and `**` is marked strong: the instructions
 * carriers ask to stand out, such as "Reply STOP". A placeholder the
 * documents do not define is an error, so a typo cannot reach the page as
 * `{something}`.
 */
export function legalInline(
  message: string,
  language: string,
  document: LegalDocument,
  wording: Pick<LegalWording, 'links'>,
): LegalInline[] {
  const values: Record<string, string> = {
    effectiveDate: legalEffectiveDate(document, language),
    ...Object.fromEntries(
      Object.entries(LEGAL_FIGURES).map(([name, value]) => [name, String(value)]),
    ),
  }
  const emails: Record<string, string> = { privacyEmail: PRIVACY_EMAIL, supportEmail: SUPPORT_EMAIL }
  const pieces: LegalInline[] = []
  message.split('**').forEach((part, index) => {
    // Every other part is between a pair of `**`.
    const strong = index % 2 === 1
    let text = ''
    let at = 0
    const flush = () => {
      if (text) pieces.push(strong ? { text, strong } : { text })
      text = ''
    }
    for (const found of part.matchAll(/\{(\w+)\}/g)) {
      text += part.slice(at, found.index)
      at = found.index + found[0].length
      const name = found[1]
      const words = wording.links[name as LegalLinkName]
      if (name in emails) {
        flush()
        pieces.push({ email: emails[name] })
      } else if (name in LEGAL_LINKS && words) {
        flush()
        pieces.push({ ...LEGAL_LINKS[name as LegalLinkName], text: words })
      } else if (name in values) {
        text += values[name]
      } else {
        throw new Error(`the ${document} document names no {${name}}`)
      }
    }
    text += part.slice(at)
    flush()
  })
  return pieces
}

/** A document's sections in order, each with its anchor. */
export function legalSections<S extends string>(
  document: LegalDocument,
  wording: LegalWording<S>,
): { id: S; section: LegalSectionWording }[] {
  return (LEGAL_SECTIONS[document] as readonly string[]).map((id) => ({
    id: id as S,
    section: wording.sections[id as S],
  }))
}
