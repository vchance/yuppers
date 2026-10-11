import {
  defaultLanguage,
  helpPath,
  languages,
  invitationPath,
  legalPath,
  legalPathOf,
  templateById,
  type LegalDocument,
} from '@yuppers/shared'

// Reading the token back out of a link is the same on every client.
export { invitationToken } from '@yuppers/shared'

/*
 * The app's addresses. Two of them are fixed points other things depend on:
 * an exchange lives at `/exchanges/{id}`, which notification emails link to,
 * and an invitation link is `/{language}/i#{token}` (DESIGN.md §13.5).
 */

export type Route =
  | { name: 'home' }
  | { name: 'account' }
  /** Starting a yup: the common agreements, the blank form, or a copy of an earlier yup. */
  | { name: 'start' }
  /**
   * The sample yup, readable signed out. `language` is the one the address
   * names (`/es/example`), or `null` for the reader's own.
   */
  | { name: 'example'; language: string | null }
  /** The account's payment options, each added, changed or removed on its own. */
  | { name: 'payments' }
  /** `language` is the sender's: it chose which entry page the link previews with, nothing more. */
  | { name: 'invitation'; language: string }
  | { name: 'exchange'; id: string }
  /**
   * A fresh start, written before the service has an exchange for it: a
   * template's id, or `blank`. It moves to `/exchanges/{id}` (replacing this
   * address) the first time something is changed.
   */
  | { name: 'newDraft'; from: string }
  /** Writing a counteroffer or an amendment. */
  | { name: 'revise'; id: string }
  /** The whole record of an exchange, laid out for reading and printing. */
  | { name: 'record'; id: string }
  /** The help pages: the list of topics, or one topic, which may not exist. */
  | { name: 'help'; topic: string | null }
  /** The privacy policy or the terms, at `/{document}` or `/{language}/{document}`. */
  | { name: 'legal'; document: LegalDocument }
  /** Staff review of abuse reports: the queue. Nothing links here; to anyone but a reviewer it is not found. */
  | { name: 'staff' }
  /** One report, opened for review. */
  | { name: 'staffReport'; id: string }
  | { name: 'notFound' }

const UUID = '[0-9a-fA-F]{8}(?:-[0-9a-fA-F]{4}){3}-[0-9a-fA-F]{12}'
const LANGUAGE_TAG = '[A-Za-z]{2,3}(?:-[A-Za-z0-9]{2,8})*'

const NEW_DRAFT = /^\/new\/([a-z][a-z0-9-]{0,39})$/
const EXCHANGE = new RegExp(`^/exchanges/(${UUID})$`)
const REVISE = new RegExp(`^/exchanges/(${UUID})/revise$`)
const RECORD = new RegExp(`^/exchanges/(${UUID})/record$`)
const INVITATION = new RegExp(`^/(${LANGUAGE_TAG})/i$`)
const EXAMPLE = /^(?:\/([A-Za-z]{2,3}(?:-[A-Za-z0-9]{2,8})*))?\/example$/
const HELP_TOPIC = /^\/help\/([a-z0-9-]+)$/
const STAFF_REPORT = new RegExp(`^/staff/reports/(${UUID})$`)

export function matchRoute(pathname: string): Route {
  // A static host may answer `/en/i` at `/en/i/`.
  const path = pathname.length > 1 ? pathname.replace(/\/+$/, '') : pathname
  if (path === '/' || path === '') return { name: 'home' }
  if (path === '/account') return { name: 'account' }
  if (path === '/new') return { name: 'start' }
  const newDraft = NEW_DRAFT.exec(path)
  if (newDraft && (newDraft[1] === 'blank' || templateById(newDraft[1]))) {
    return { name: 'newDraft', from: newDraft[1] }
  }
  const example = EXAMPLE.exec(path)
  if (example) {
    const wanted = example[1]?.toLowerCase()
    if (wanted === undefined) return { name: 'example', language: null }
    const language = languages.find((info) => info.code.toLowerCase() === wanted)?.code
    if (language) return { name: 'example', language }
  }
  if (path === '/account/payments') return { name: 'payments' }
  if (path === '/help') return { name: 'help', topic: null }
  const help = HELP_TOPIC.exec(path)
  if (help) return { name: 'help', topic: help[1] }
  const legal = legalPathOf(path)
  if (legal) return { name: 'legal', document: legal.document }
  if (path === '/staff') return { name: 'staff' }
  const staffReport = STAFF_REPORT.exec(path)
  if (staffReport) return { name: 'staffReport', id: staffReport[1].toLowerCase() }

  const invitation = INVITATION.exec(path)
  if (invitation) return { name: 'invitation', language: invitation[1] }
  const revise = REVISE.exec(path)
  if (revise) return { name: 'revise', id: revise[1].toLowerCase() }
  const record = RECORD.exec(path)
  if (record) return { name: 'record', id: record[1].toLowerCase() }
  const exchange = EXCHANGE.exec(path)
  if (exchange) return { name: 'exchange', id: exchange[1].toLowerCase() }
  return { name: 'notFound' }
}

export const paths = {
  home: '/',
  account: '/account',
  start: '/new',
  /** A fresh start's composer: a template's id, or `blank`. */
  newDraft: (from: string) => `/new/${from}`,
  /** The sample yup: `/example` in the default language, `/{language}/example` in the others. */
  example: (language: string = defaultLanguage) =>
    language === defaultLanguage ? '/example' : `/${language}/example`,
  payments: '/account/payments',
  exchange: (id: string) => `/exchanges/${id}`,
  revise: (id: string) => `/exchanges/${id}/revise`,
  record: (id: string) => `/exchanges/${id}/record`,
  help: helpPath,
  /** The privacy policy or the terms in a language: `/privacy`, `/terms` in the default one. */
  legal: legalPath,
  staff: '/staff',
  staffReport: (id: string) => `/staff/reports/${id}`,
  /**
   * The link an initiator shares. The token goes in the fragment, which a
   * browser never sends, so it cannot end up in a server log; the path names
   * the sender's language so the link previews in it.
   */
  invitation: invitationPath,
}
