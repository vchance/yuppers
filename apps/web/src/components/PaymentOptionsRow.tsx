import { paymentOptionsSummary, useSavedPaymentHandles } from '@yuppers/shared'
import { useId } from 'react'

import { useI18n } from '../app/context'
import { Link } from '../app/Link'
import { paths } from '../app/routes'
import { api } from '../lib/api'

/**
 * Payment options on the account screen: one row, named for them, with
 * the apps added ("Venmo, Zelle") or "None added", that opens their own
 * screen (`PaymentOptionsPage`). The whole row can be pressed; the link
 * in its heading is what the keyboard and assistive technology reach.
 */
export function PaymentOptionsRow() {
  const { wording } = useI18n()
  const w = wording.payments
  const saved = useSavedPaymentHandles(api)
  const id = useId()
  // Unknown while loading, and if it cannot be loaded: the row still opens the screen.
  const summary = saved ? paymentOptionsSummary(saved, w.apps, w.noneAdded) : null

  return (
    <section className="card nav-row" aria-labelledby={`${id}-heading`}>
      <div>
        <h2 id={`${id}-heading`}>
          <Link to={paths.payments} aria-describedby={summary ? `${id}-summary` : undefined}>
            {w.heading}
          </Link>
        </h2>
        {summary && (
          <p className="hint" id={`${id}-summary`}>
            {summary}
          </p>
        )}
      </div>
      <span className="nav-row-chevron" aria-hidden="true">
        ›
      </span>
    </section>
  )
}
