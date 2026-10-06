import type { LegalDocument } from '@yuppers/shared'

import { useI18n } from '../app/context'
import { paths } from '../app/routes'

/**
 * A small link to the privacy policy or the terms, or to one of their
 * sections, from a place that is about them: signing in, the account,
 * deleting the account. It opens in a new tab, as "Learn more" does
 * (`HelpLink`), so that a code half typed in or a panel half filled in is
 * not lost, and says so to screen readers. The address names the language
 * on screen, and is a page the service writes in full, so the new tab reads
 * without scripts too.
 */
export function LegalLink({
  document,
  section,
  label,
}: {
  document: LegalDocument
  section?: string
  label?: string
}) {
  const { wording, language } = useI18n()
  const name = document === 'privacy' ? wording.privacy.policy : wording.termsOfUse.document
  return (
    <a
      href={`${paths.legal(document, language)}${section ? `#${section}` : ''}`}
      target="_blank"
      rel="noopener"
    >
      {label ?? name}
      <span className="visually-hidden"> {wording.help.newTab}</span>
    </a>
  )
}
