import type { LegalDocument } from './legal-text'

/*
 * Acceptance of the Terms and the Privacy policy at sign-in. One sentence
 * above the button (`signIn.agreement`) says that continuing means agreeing
 * to both, with links to them; the sentence is the assent, with no box. The
 * request that completes the sign-in names the version shown
 * (`terms_version`), which the service checks and stores with the account
 * (`backend/src/terms.rs`).
 */

/**
 * The version of the Terms and the Privacy policy: the day they took effect
 * (`LEGAL_EFFECTIVE_DATES`). The service checks it (`TERMS_VERSION` in
 * `backend/src/terms.rs`). Change it together with the effective dates and
 * that constant; tests keep all three in step.
 */
export const TERMS_VERSION = '2026-10-09'

/** The documents the sentence links to, in the order it names them. */
export const TERMS_ASSENT_DOCUMENTS = ['terms', 'privacy'] as const satisfies LegalDocument[]

/** A piece of the sentence: plain text, or the name of a document, shown as a link to it. */
export type TermsAssentPiece = { text: string } | { document: LegalDocument; text: string }

/**
 * The sentence split into its pieces. `agreement` has `{terms}` and
 * `{privacy}` where the documents' names go; `names` says what each is
 * called in the language on screen.
 */
export function termsAssentPieces(
  agreement: string,
  names: Record<LegalDocument, string>,
): TermsAssentPiece[] {
  const pieces: TermsAssentPiece[] = []
  let at = 0
  for (const found of agreement.matchAll(/\{(terms|privacy)\}/g)) {
    if (found.index > at) pieces.push({ text: agreement.slice(at, found.index) })
    const document = found[1] as LegalDocument
    pieces.push({ document, text: names[document] })
    at = found.index + found[0].length
  }
  if (at < agreement.length) pieces.push({ text: agreement.slice(at) })
  return pieces
}
