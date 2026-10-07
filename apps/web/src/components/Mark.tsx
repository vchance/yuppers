/**
 * The Yuppers mark: two circles, the reader's and the other party's, and
 * where they overlap, agree green. Drawn from the colour tokens
 * (`.mark-*` in `index.css`), so it follows the theme. Decorative: whatever
 * it stands beside says the same in words. With `check`, the overlap carries
 * a check, for an agreement both have signed.
 */
export function Mark({ check = false, className }: { check?: boolean; className?: string }) {
  return (
    <svg className={className} viewBox="0 0 112 80" aria-hidden="true" focusable="false">
      <circle className="mark-you" cx="40" cy="40" r="36" />
      <circle className="mark-them" cx="72" cy="40" r="36" />
      <path className="mark-agree" d="M56 7.75A36 36 0 0 1 56 72.25A36 36 0 0 1 56 7.75Z" />
      {check && <path className="mark-check" d="M49 41l5 5 9-11" />}
    </svg>
  )
}
