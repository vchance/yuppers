import type { RevisionTerms } from '@yuppers/api-client'
import { receivesMoney, useHasPaymentHandles } from '@yuppers/shared'

import { useI18n } from '../app/context'
import { api } from '../lib/api'

interface Props {
  terms: RevisionTerms
  you: string
  /** Whether they are shown on this yup already. */
  shown: boolean
  checked: boolean
  onChange(checked: boolean): void
}

/**
 * Beside signing or sending terms in which the person receives money: a
 * box, unticked, to show their payment options on this yup too
 * (`payments.ts`). Only for someone who has saved some and does not show
 * them here yet. It is not part of what is signed: it is set once the
 * signature has gone, and can be changed on the yup at any time.
 */
export function ShowWhenSigning({ terms, you, shown, checked, onChange }: Props) {
  const { wording } = useI18n()
  const has = useHasPaymentHandles(api)
  if (!has || shown || !receivesMoney(terms, you)) return null
  return (
    <label className="check">
      <input
        type="checkbox"
        checked={checked}
        onChange={(event) => onChange(event.target.checked)}
      />
      <span>{wording.payments.showWhenSigning}</span>
    </label>
  )
}
