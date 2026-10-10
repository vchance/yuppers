import { termsAssentPieces } from '@yuppers/shared'

import { useI18n } from '../app/context'
import { LegalLink } from './LegalLink'

/**
 * The sentence above the button that signs in: continuing is the assent to
 * the Terms and the Privacy policy (`signIn.agreement`; `terms.ts`). The
 * names of the two documents are links to them, opened in a new tab so that
 * a code half typed in is not lost. No box: the sentence is the assent.
 */
export function TermsAssent() {
  const { wording } = useI18n()
  const names = { terms: wording.termsOfUse.link, privacy: wording.privacy.policy }
  return (
    <p className="terms-assent">
      {termsAssentPieces(wording.signIn.agreement, names).map((piece, index) =>
        'document' in piece ? (
          <LegalLink key={index} document={piece.document} label={piece.text} />
        ) : (
          piece.text
        ),
      )}
    </p>
  )
}
