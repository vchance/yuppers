import { decimalForInput, parseDecimal } from '@yuppers/shared'
import { useState } from 'react'

import { useI18n } from '../app/context'
import type { ControlProps } from './ui'

interface DecimalInputProps extends ControlProps {
  /** A plain decimal, empty for none, `null` when what is typed is not a number. */
  value: string | null
  onChange(value: string | null): void
  /** Grey text for an empty field: an example, never a value. */
  placeholder?: string
}

/**
 * A number typed the way the reader's language writes numbers. What is typed
 * stays on screen as typed; the working copy gets the plain form, or `null`
 * while it cannot be read as a number.
 */
export function DecimalInput({ value, onChange, ...control }: DecimalInputProps) {
  const { language } = useI18n()
  const [text, setText] = useState(() => (value ? decimalForInput(value, language) : ''))
  return (
    <input
      {...control}
      type="text"
      inputMode="decimal"
      autoComplete="off"
      value={text}
      onChange={(event) => {
        const typed = event.target.value
        setText(typed)
        onChange(typed.trim() === '' ? '' : parseDecimal(typed, language))
      }}
    />
  )
}
