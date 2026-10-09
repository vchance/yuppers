import { useState } from 'react'

import { useI18n, useSession } from '../app/context'
import { api } from '../lib/api'

/**
 * The notice that another account was combined into this one, shown once
 * where neither account had an email address to tell (README, "Combining
 * accounts"; nothing is texted). "Got it" dismisses it for good.
 */
export function CombinedNotice() {
  const { wording, fmt, moment } = useI18n()
  const { account, setAccount } = useSession()
  const [busy, setBusy] = useState(false)
  const at = account?.combined_notice
  if (!at) return null
  const w = wording.combine
  return (
    <section className="card notice combined-notice" aria-label={wording.identifiers.heading}>
      <p>{fmt(w.noticeBanner, { date: moment(at) })}</p>
      <div className="actions">
        <button
          type="button"
          disabled={busy}
          onClick={() => {
            setBusy(true)
            api.updateMe({ dismiss_combined_notice: true }).then(setAccount, () => setBusy(false))
          }}
        >
          {w.noticeDismiss}
        </button>
      </div>
    </section>
  )
}
