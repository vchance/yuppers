import { noticeText } from '@yuppers/shared'
import { useState } from 'react'

import { useI18n, useSession } from '../app/context'
import { api } from '../lib/api'

/**
 * A notice about the account shown once where there was no email address to
 * tell (README, "Email address and phone number"; nothing is texted):
 * another account combined into this one, or its phone number replaced or
 * removed. "Got it" dismisses it for good.
 */
export function CombinedNotice() {
  const { wording, moment, language } = useI18n()
  const { account, setAccount } = useSession()
  const [busy, setBusy] = useState(false)
  const notice = account?.notice
  if (!notice) return null
  return (
    <section className="card notice combined-notice" aria-label={wording.identifiers.heading}>
      <p>{noticeText(wording, notice.kind, moment(notice.at), language)}</p>
      <div className="actions">
        <button
          type="button"
          disabled={busy}
          onClick={() => {
            setBusy(true)
            api.updateMe({ dismiss_notice: true }).then(setAccount, () => setBusy(false))
          }}
        >
          {wording.combine.noticeDismiss}
        </button>
      </div>
    </section>
  )
}
