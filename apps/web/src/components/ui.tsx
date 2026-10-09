import type { ErrorCode } from '@yuppers/api-client'
import { labelText, type MessageValues } from '@yuppers/shared'
import { useEffect, useId, useRef, type ReactNode } from 'react'

import { useI18n } from '../app/context'
import { hasNavigated } from '../app/router'
import { announce, useAnnouncement } from '../lib/announce'
import { focusLost } from '../lib/focus'

// How many page headings this page load has shown, to tell the first one,
// which leaves the focus where the browser put it, from those that follow.
let headingsShown = 0

/**
 * A page's main heading. It names the page in the browser tab, and takes the
 * focus when the page was reached from another one, so a screen reader
 * announces the change and the keyboard starts from the top. It also takes
 * the focus when it replaces a page whose control had the focus, such as
 * the sign-in form giving way to the page it stood in for: the focus would
 * otherwise be lost to the top of the document.
 */
export function PageHeading({
  children,
  name,
  step,
}: {
  /** The heading. With `name`, a message with a `{name}` placeholder. */
  children: string
  /** Someone's name, written by them, to fill in `{name}` with. */
  name?: string
  step?: boolean
}) {
  const { wording, fmt } = useI18n()
  const heading = useRef<HTMLHeadingElement>(null)
  // Where this heading came in this page load, decided once, so that running
  // the effect again (React does in development) does not change the answer.
  const order = useRef<number | null>(null)
  // A tab's title cannot isolate a name the way `<bdi>` does below, so it
  // has the name without anything that could turn the title around.
  const title = labelText(name === undefined ? children : fmt(children, { name: labelText(name) }))

  useEffect(() => {
    document.title = `${title} · ${wording.productName}`
  }, [title, wording.productName])

  // `step` marks a heading that replaces another without the address
  // changing, such as moving from editing to signing.
  useEffect(() => {
    if (order.current === null) order.current = headingsShown++
    if (step || hasNavigated() || (order.current > 0 && focusLost())) heading.current?.focus()
    if (step) window.scrollTo(0, 0)
  }, [step])

  return (
    <h1 ref={heading} tabIndex={-1}>
      {name === undefined ? children : <WithName message={children} name={name} />}
    </h1>
  )
}

// Stands in for the name while the message around it is filled in. Never in
// any wording, and taken out of the name itself by `labelText`.
const NAME_MARK = '\u0000'

/**
 * A message with someone's name in it, the name isolated in `<bdi>`: a name
 * written right to left, or with characters that change direction, cannot
 * turn the words around it, and nothing in it can steer the rest of the
 * line. Control and direction characters are taken out of it as well. The
 * message's other placeholders, if any, are filled from `values`.
 */
export function WithName({
  message,
  name,
  values,
}: {
  message: string
  name: string
  values?: MessageValues
}) {
  const { fmt } = useI18n()
  const [before, ...after] = fmt(message, { ...values, name: NAME_MARK }).split(NAME_MARK)
  if (after.length === 0) return <>{before}</>
  return (
    <>
      {before}
      <bdi>{labelText(name)}</bdi>
      {after.join(labelText(name))}
    </>
  )
}

/**
 * A section heading that takes the focus when it appears, for a step that
 * opens below what the person pressed and replaces it, such as signing in
 * on the invitation page.
 */
export function StepHeading({ children, id }: { children: string; id?: string }) {
  const heading = useRef<HTMLHeadingElement>(null)
  useEffect(() => {
    heading.current?.focus()
  }, [])
  return (
    <h2 ref={heading} tabIndex={-1} id={id}>
      {children}
    </h2>
  )
}

/**
 * A refusal from the service, in the reader's language. `id` lets the
 * control it is about point to it.
 */
export function Failure({ code, id }: { code: ErrorCode | null; id?: string }) {
  const { errorText } = useI18n()
  const text = code ? errorText(code) : null
  return text ? <ErrorNote id={id}>{text}</ErrorNote> : null
}

/**
 * Something that stops the person, in the product's own words. It is said at
 * once. If the focus was lost on the way, as it is when the button that was
 * pressed is disabled while it works, the focus comes here too, which leaves
 * the keyboard next to what to do about it rather than at the top.
 */
export function ErrorNote({ children, id }: { children: string; id?: string }) {
  const note = useRef<HTMLParagraphElement>(null)
  useEffect(() => {
    announce(children, 'assertive')
    if (focusLost()) note.current?.focus()
  }, [children])
  return (
    <p className="notice notice-error" id={id} tabIndex={-1} ref={note}>
      {children}
    </p>
  )
}

/** Something that happened, announced without interrupting. */
export function Notice({ children }: { children: string }) {
  useAnnouncement(children)
  return <p className="notice">{children}</p>
}

export interface ControlProps {
  id: string
  'aria-describedby': string | undefined
  'aria-invalid': true | undefined
  'aria-required': true | undefined
}

interface FieldProps {
  label: string
  hint?: string
  error?: string | null
  /** A fixed id, when something else needs to find the control. */
  id?: string
  /** Has to be filled in; said to assistive technology, and checked when the form is sent. */
  required?: boolean
  /**
   * Something else on the page that is about this control, such as a refusal
   * from the service shown under the form: its id, to be read with the
   * control, which is then marked invalid.
   */
  problem?: string | null
  children: (control: ControlProps) => ReactNode
}

/** A labelled control with its hint and its error, tied together for assistive technology. */
export function Field({
  label,
  hint,
  error,
  id: fixedId,
  required,
  problem,
  children,
}: FieldProps) {
  const generated = useId()
  const id = fixedId ?? generated
  const described = [
    hint ? `${id}-hint` : null,
    error ? `${id}-error` : null,
    problem ?? null,
  ].filter(Boolean)
  return (
    <div className="field">
      <label htmlFor={id}>{label}</label>
      {hint && (
        <p className="hint" id={`${id}-hint`}>
          {hint}
        </p>
      )}
      {children({
        id,
        'aria-describedby': described.length > 0 ? described.join(' ') : undefined,
        'aria-invalid': error || problem ? true : undefined,
        'aria-required': required ? true : undefined,
      })}
      {error && (
        <p className="field-error" id={`${id}-error`}>
          {error}
        </p>
      )}
    </div>
  )
}

/**
 * Text a person wrote: a name, terms, a description, a note. It is shown
 * exactly as written and never translated (DESIGN.md §4.2), set apart so it
 * cannot be mistaken for the product speaking, and laid out in whichever
 * direction its own script runs.
 */
export function Written({ children, inline }: { children: string; inline?: boolean }) {
  return inline ? (
    <bdi className="written-inline">{children}</bdi>
  ) : (
    <p className="written" dir="auto">
      {children}
    </p>
  )
}
