import type { Template } from '@yuppers/shared'

import { useI18n } from '../app/context'

/**
 * What the common agreement a draft was started from has to say about
 * itself, above the items (DESIGN.md §4.4): the hint, the note that the grey
 * text is only an example, and, for the job, the warning about New Jersey.
 * It stays until the first edit, then folds away behind a link. The warning
 * does not fold: it is a statement about the law, not about the form.
 */
export function StartingPoint({
  template,
  open,
  onToggle,
}: {
  template: Template
  open: boolean
  onToggle(): void
}) {
  const { wording } = useI18n()
  const w = wording.templates
  const entry = w.entries[template.id]
  return (
    <div>
      {entry.warning && <p className="notice notice-warning">{entry.warning}</p>}
      {open ? (
        <div className="notice" id="starting-point">
          <h2 id="starting-point-heading">{w.bandHeading}</h2>
          <p>{entry.hint}</p>
          <p>{w.bandExamples}</p>
          <div className="actions">
            <button type="button" aria-expanded onClick={onToggle}>
              {w.bandHide}
            </button>
          </div>
        </div>
      ) : (
        <p>
          <button
            type="button"
            className="link"
            aria-expanded={false}
            onClick={onToggle}
          >
            {w.bandShow}
          </button>
        </p>
      )}
    </div>
  )
}

/**
 * Turns every item's provider round in one tap, for when the other person is
 * the one writing: each common agreement is written from one side.
 */
export function SwapSides({ onSwap }: { onSwap(): void }) {
  const { wording } = useI18n()
  const w = wording.templates
  return (
    <div className="swap-sides">
      <button type="button" aria-describedby="swap-sides-hint" onClick={onSwap}>
        {w.swapSides}
      </button>
      <p className="hint" id="swap-sides-hint">
        {w.swapSidesHint}
      </p>
    </div>
  )
}
