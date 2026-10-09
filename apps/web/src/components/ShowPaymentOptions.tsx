import type { ExchangeView as Exchange } from '@yuppers/api-client'
import { showOffered, useShowPaymentOptions } from '@yuppers/shared'
import { useCallback, useId } from 'react'

import { useI18n } from '../app/context'
import { Link } from '../app/Link'
import { paths } from '../app/routes'
import { api } from '../lib/api'
import { useAnnouncement } from '../lib/announce'
import { Failure } from './ui'

interface Props {
  exchange: Exchange
  otherName: string
  reload(): Promise<unknown>
}

/**
 * "Your payment options" on a yup, for a party who receives money in it
 * (`payments.ts`): a box that shows their saved payment options to the
 * other party, who sees them only while they owe money here. Off until
 * ticked; unticking takes them away at once. Not part of the agreement.
 */
export function ShowPaymentOptions({ exchange, otherName, reload }: Props) {
  const { wording, fmt } = useI18n()
  const w = wording.payments
  const id = useId()
  const refresh = useCallback(() => void reload(), [reload])
  const control = useShowPaymentOptions(api, exchange, refresh)
  const said =
    control.changed === 'shown'
      ? fmt(w.shownNow, { name: otherName })
      : control.changed === 'hidden'
        ? w.hiddenNow
        : null
  useAnnouncement(said)

  if (!showOffered(exchange) || control.hasHandles === null) return null
  const hintId = `${id}-hint`

  return (
    <section className="card payment-options" aria-labelledby={`${id}-heading`}>
      <h2 id={`${id}-heading`}>{w.showHeading}</h2>
      {control.hasHandles || control.shown ? (
        <>
          <label className="check">
            <input
              type="checkbox"
              checked={control.shown}
              disabled={control.busy}
              aria-describedby={hintId}
              onChange={(event) => void control.set(event.target.checked)}
            />
            <span>{w.showLabel}</span>
          </label>
          <p className="hint" id={hintId}>
            {fmt(w.showHint, { name: otherName })}
          </p>
          <Failure code={control.failure} />
          {said && <p className="notice">{said}</p>}
          <p>
            <Link to={paths.payments}>{w.manage}</Link>
          </p>
        </>
      ) : (
        <p>
          {w.noneSaved} <Link to={paths.payments}>{w.addInAccount}</Link>
        </p>
      )}
    </section>
  )
}
