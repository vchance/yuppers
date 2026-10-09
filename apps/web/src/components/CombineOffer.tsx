import type { ErrorCode } from '@yuppers/api-client'
import {
  combineAccountLines,
  combineEffects,
  combineHeading,
  type CombineOffer as Offer,
} from '@yuppers/shared'

import { useI18n } from '../app/context'
import { Panel } from './Panel'
import { Failure } from './ui'

interface Props {
  offer: Offer
  busy: boolean
  failure: ErrorCode | null
  onCombine(): void
  onCancel(): void
}

/**
 * The offer to combine another account into this one, once a code showed
 * that the address is on it (README, "Combining accounts"): what that
 * account is, what moves and what is dropped, that it cannot be undone,
 * and one deliberate button.
 */
export function CombineOffer({ offer, busy, failure, onCombine, onCancel }: Props) {
  const { wording, language } = useI18n()
  const w = wording.combine
  return (
    <Panel title={combineHeading(w, offer)}>
      <p>{w.intro}</p>
      <h3>{w.otherHeading}</h3>
      <ul className="combine-account">
        {combineAccountLines(w, offer, language).map((line) => (
          <li key={line}>{line}</li>
        ))}
      </ul>
      <h3>{w.movesHeading}</h3>
      <ul>
        {combineEffects(w, offer, language).map((line) => (
          <li key={line}>{line}</li>
        ))}
      </ul>
      <p>
        <strong>{w.cannotUndo}</strong> {w.expires}
      </p>
      <Failure code={failure} />
      <div className="actions">
        <button type="button" className="primary" disabled={busy} onClick={onCombine}>
          {w.confirm}
        </button>
        <button type="button" disabled={busy} onClick={onCancel}>
          {wording.common.cancel}
        </button>
      </div>
    </Panel>
  )
}
