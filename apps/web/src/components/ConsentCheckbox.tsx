import { consentPieces } from '@yuppers/shared'

interface Props {
  /** The consent wording, word for word; its addresses become links. */
  wording: string
  checked: boolean
  onChange(checked: boolean): void
}

/**
 * A box beside consent wording to texts: the box beside a number a code is
 * to be texted to (`smsCode`), and the box that turns on an agreement's
 * text updates (`smsUpdates.consent`). Its name is the wording and nothing
 * else, so its two addresses are links that say only the address, though
 * they open a new tab.
 */
export function ConsentCheckbox({ wording, checked, onChange }: Props) {
  return (
    <label className="check sms-consent">
      <input
        type="checkbox"
        checked={checked}
        onChange={(event) => onChange(event.target.checked)}
      />
      <span>
        {consentPieces(wording).map((piece, index) =>
          'url' in piece ? (
            <a key={index} href={piece.url} target="_blank" rel="noopener">
              {piece.url}
            </a>
          ) : (
            piece.text
          ),
        )}
      </span>
    </label>
  )
}
