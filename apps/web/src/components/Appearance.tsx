import { useId, useState } from 'react'

import { useI18n } from '../app/context'
import { chooseTheme, THEME_CHOICES, useThemeChoice } from '../lib/theme'

/**
 * System, Light or Dark, for this device (`lib/theme.ts`): three radio
 * buttons in a fieldset, drawn as one segmented control. The choice applies
 * at once, so there is nothing to save.
 */
export function Appearance() {
  const { wording } = useI18n()
  const w = wording.appearance
  const choice = useThemeChoice()
  const name = useId()
  const hint = useId()
  return (
    <fieldset className="appearance" aria-describedby={hint}>
      <legend>{w.heading}</legend>
      <p className="hint" id={hint}>
        {w.hint}
      </p>
      <div className="segmented">
        {THEME_CHOICES.map((option) => (
          <label key={option}>
            <input
              type="radio"
              name={name}
              value={option}
              checked={choice === option}
              onChange={() => chooseTheme(option)}
            />
            <span>{w[option]}</span>
          </label>
        ))}
      </div>
    </fieldset>
  )
}

/**
 * "Appearance" in the footer, which opens the switch in place: for anyone,
 * signed in or not, such as someone reading an invitation who has no
 * account and so no account page.
 */
export function FooterAppearance() {
  const { wording } = useI18n()
  const [open, setOpen] = useState(false)
  return (
    <>
      <button
        type="button"
        className="link"
        aria-expanded={open}
        onClick={() => setOpen((shown) => !shown)}
      >
        {wording.appearance.heading}
      </button>
      {open && <Appearance />}
    </>
  )
}
