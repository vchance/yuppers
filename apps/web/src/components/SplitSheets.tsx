import {
  addMonths,
  MAX_SPLIT,
  planInstalments,
  planStages,
  splitIntoInstalments,
  splitIntoStages,
  splitRoom,
  todayIn,
  type AmountChoice,
  type DraftContribution,
  type Every,
  type InstalmentsProblem,
  type SplitGroup,
  type StagesProblem,
} from '@yuppers/shared'
import { useState, type KeyboardEvent, type ReactNode } from 'react'

import { useI18n } from '../app/context'
import { DecimalInput } from './DecimalInput'
import { Panel } from './Panel'
import { Field } from './ui'

/*
 * The sheets that split one item of the composer into several (DESIGN.md
 * §7.1, §7.2): a money item into instalments, a service or task item into
 * stages. They only write items into the working copy; what they make is
 * ordinary, and the author can edit any of it. Each shows every amount and
 * every date before anything is added.
 *
 * A sheet sits inside the composer's form, so Enter in one of its fields
 * adds the items rather than reviewing the whole of the terms.
 */

export interface SheetProps {
  /** The item being split. */
  item: DraftContribution
  /** How many items the working copy holds now, this one included. */
  existing: number
  /** Decimal places of the exchange's currency. */
  digits: number
  currency: string
  /** The exchange's time zone, which dates are read in. */
  timezone: string
  /** The items to put in the place of `item`, and how to put them back. */
  onAdd(items: DraftContribution[], group: SplitGroup): void
  onCancel(): void
}

/** Enter adds, instead of sending the composer's form. */
function addOnEnter(add: () => void) {
  return (event: KeyboardEvent) => {
    if (event.key !== 'Enter' || (event.target as HTMLElement).tagName === 'BUTTON') return
    event.preventDefault()
    add()
  }
}

function whole(text: string): number {
  return /^\d+$/.test(text.trim()) ? Number(text.trim()) : Number.NaN
}

/** The amount fields both sheets use: share an amount out, or the same amount for each. */
function AmountFields(props: {
  idPrefix: string
  currency: string
  mode: AmountChoice['mode']
  onMode(mode: AmountChoice['mode']): void
  initial: string
  onAmount(amount: string | null): void
  error: string | null
}) {
  const { wording, fmt } = useI18n()
  const w = wording.composer.split.amounts
  return (
    <>
      <fieldset>
        <legend>{w.howLegend}</legend>
        {(['SHARE', 'EACH'] as const).map((mode) => (
          <label className="check" key={mode}>
            <input
              type="radio"
              name={`${props.idPrefix}-how`}
              checked={props.mode === mode}
              onChange={() => props.onMode(mode)}
            />
            <span>{mode === 'SHARE' ? w.howShare : w.howEach}</span>
          </label>
        ))}
      </fieldset>
      <Field
        label={fmt(props.mode === 'SHARE' ? w.shareLabel : w.eachLabel, {
          currency: props.currency,
        })}
        hint={props.mode === 'SHARE' ? w.remainderHint : undefined}
        id={`${props.idPrefix}-amount`}
        required
        error={props.error}
      >
        {(control) => (
          <DecimalInput {...control} value={props.initial} onChange={props.onAmount} />
        )}
      </Field>
    </>
  )
}

function CountField(props: {
  id: string
  label: string
  hint: string
  value: string
  onChange(value: string): void
  error: string | null
}) {
  return (
    <Field label={props.label} hint={props.hint} id={props.id} required error={props.error}>
      {(control) => (
        <input
          {...control}
          type="text"
          inputMode="numeric"
          autoComplete="off"
          value={props.value}
          onChange={(event) => props.onChange(event.target.value)}
        />
      )}
    </Field>
  )
}

function focusSoon(id: string) {
  window.setTimeout(() => document.getElementById(id)?.focus())
}

// ---- Instalments ---------------------------------------------------------------

export function InstalmentsSheet({
  item,
  existing,
  digits,
  currency,
  timezone,
  onAdd,
  onCancel,
}: SheetProps) {
  const { wording, fmt, day, money } = useI18n()
  const w = wording.composer.split
  const s = w.instalments
  const room = splitRoom(existing)

  const [count, setCount] = useState('3')
  const [mode, setMode] = useState<AmountChoice['mode']>('SHARE')
  const [amount, setAmount] = useState<string | null>(item.amount)
  // Sheets open at 3, monthly, the first date a month ahead (DESIGN.md §18, item 44).
  const [first, setFirst] = useState(() => addMonths(todayIn(timezone), 1))
  const [everyKind, setEveryKind] = useState<Every['kind']>('MONTH')
  const [days, setDays] = useState('30')
  const [checked, setChecked] = useState(false)

  const every: Every =
    everyKind === 'DAYS' ? { kind: 'DAYS', days: whole(days) } : ({ kind: everyKind } as Every)
  const plan = planInstalments(
    { count: whole(count), amounts: { mode, amount }, first, every },
    digits,
    room,
  )
  const problems: readonly InstalmentsProblem[] = checked && !plan.ok ? plan.problems : []
  const problemText = (problem: InstalmentsProblem) =>
    fmt(s.problems[problem], { max: Math.min(room, MAX_SPLIT), example: '100.00' })
  const errorFor = (...codes: InstalmentsProblem[]) => {
    const found = problems.find((problem) => codes.includes(problem))
    return found ? problemText(found) : null
  }

  const base = item.description.trim()
  const describe = (number: number, total: number) =>
    base
      ? fmt(w.ordinal, { description: base, number, count: total })
      : fmt(w.ordinalBlank, { number, count: total })

  function add() {
    if (!plan.ok) {
      setChecked(true)
      const first = plan.problems[0]
      focusSoon(
        first === 'COUNT'
          ? 'split-count'
          : first === 'DATE'
            ? 'split-first'
            : first === 'DAYS'
              ? 'split-days'
              : 'split-amount',
      )
      return
    }
    const made = splitIntoInstalments(item, plan.rows, describe, () => crypto.randomUUID(), digits)
    onAdd(made.items, made.group)
  }

  return (
    <Panel title={s.title}>
      <div role="presentation" onKeyDown={addOnEnter(add)}>
        <p>{s.intro}</p>
        <CountField
          id="split-count"
          label={s.countLabel}
          hint={fmt(s.countHint, { max: Math.min(room, MAX_SPLIT) })}
          value={count}
          onChange={setCount}
          error={errorFor('COUNT')}
        />
        <AmountFields
          idPrefix="split"
          currency={currency}
          mode={mode}
          onMode={setMode}
          initial={item.amount ?? ''}
          onAmount={setAmount}
          error={errorFor('AMOUNT', 'TOO_SMALL')}
        />
        <Field label={s.firstLabel} id="split-first" required error={errorFor('DATE')}>
          {(control) => (
            <input
              {...control}
              type="date"
              value={first}
              onChange={(event) => setFirst(event.target.value)}
            />
          )}
        </Field>
        <Field label={s.everyLegend} hint={everyKind === 'MONTH' ? s.monthHint : undefined}>
          {(control) => (
            <select
              {...control}
              value={everyKind}
              onChange={(event) => setEveryKind(event.target.value as Every['kind'])}
            >
              <option value="WEEK">{s.everyWeek}</option>
              <option value="TWO_WEEKS">{s.everyTwoWeeks}</option>
              <option value="MONTH">{s.everyMonth}</option>
              <option value="DAYS">{s.everyDays}</option>
            </select>
          )}
        </Field>
        {everyKind === 'DAYS' && (
          <CountField
            id="split-days"
            label={s.daysLabel}
            hint=""
            value={days}
            onChange={setDays}
            error={errorFor('DAYS')}
          />
        )}

        {plan.ok && (
          <section aria-labelledby="split-preview">
            <h3 id="split-preview">{s.previewHeading}</h3>
            <ol className="plain">
              {plan.rows.map((row, index) => (
                <li key={row.date + index}>
                  {fmt(s.previewLine, {
                    description: describe(index + 1, plan.rows.length),
                    amount: money(row.amountMinor, currency),
                    date: day(row.date),
                  })}
                </li>
              ))}
            </ol>
          </section>
        )}
        <p className="hint">{s.plainHint}</p>
        <div className="actions">
          <button type="button" className="primary" onClick={add}>
            {s.done}
          </button>
          <button type="button" onClick={onCancel}>
            {wording.common.cancel}
          </button>
        </div>
      </div>
    </Panel>
  )
}

// ---- Stages ------------------------------------------------------------------------

export function StagesSheet({ item, existing, digits, currency, onAdd, onCancel }: SheetProps) {
  const { wording, fmt, day, money } = useI18n()
  const w = wording.composer.split
  const s = w.stages
  const [count, setCount] = useState('3')
  const [names, setNames] = useState<string[]>(() => Array.from({ length: MAX_SPLIT }, () => ''))
  const [dates, setDates] = useState<string[]>(() => {
    const filled = Array.from({ length: MAX_SPLIT }, () => '')
    // The first stage starts from the item's own date, if it had one.
    if (item.due.kind === 'DATE') filled[0] = item.due.date
    return filled
  })
  const [chain, setChain] = useState(false)
  const [pay, setPay] = useState(false)
  const [mode, setMode] = useState<AmountChoice['mode']>('SHARE')
  const [amount, setAmount] = useState<string | null>('')
  const [checked, setChecked] = useState(false)

  const total = whole(count)
  // With a payment for each stage, each stage takes two items.
  const room = splitRoom(existing, pay ? 2 : 1)
  const plan = planStages(
    { count: total, names, dates, chain, pay: pay ? { mode, amount } : null },
    digits,
    room,
  )
  const problems: readonly StagesProblem[] = checked && !plan.ok ? plan.problems : []
  const errorFor = (...codes: StagesProblem[]) => {
    const found = problems.find((problem) => codes.includes(problem))
    return found
      ? fmt(s.problems[found], { max: Math.min(room, MAX_SPLIT), example: '100.00' })
      : null
  }
  const shown = Number.isInteger(total) ? Math.max(0, Math.min(total, MAX_SPLIT)) : 0
  const example = (index: number) =>
    [s.exampleOne, s.exampleTwo, s.exampleThree][index] ?? undefined

  const payer = item.from === 'A' ? 'B' : 'A'

  function add() {
    if (!plan.ok) {
      setChecked(true)
      const first = plan.problems[0]
      focusSoon(
        first === 'COUNT'
          ? 'split-count'
          : first === 'NAME'
            ? `stage-name-${names.findIndex((name, index) => index < shown && name.trim() === '')}`
            : first === 'DATE'
              ? 'stage-date-0'
              : 'split-amount',
      )
      return
    }
    const made = splitIntoStages(
      item,
      plan.plan,
      { chain, payer },
      (stage) => fmt(s.paymentDescription, { stage }),
      () => crypto.randomUUID(),
      digits,
    )
    onAdd(made.items, made.group)
  }

  const set = (list: string[], index: number, value: string) =>
    list.map((existingValue, at) => (at === index ? value : existingValue))

  let preview: ReactNode = null
  if (plan.ok) {
    preview = (
      <section aria-labelledby="split-preview">
        <h3 id="split-preview">{s.previewHeading}</h3>
        <ol className="plain">
          {plan.plan.stages.flatMap((stage, index) => {
            const number = index + 1
            const line =
              chain && index > 0
                ? fmt(s.previewStageChained, { number, name: stage.name })
                : stage.date === ''
                  ? fmt(s.previewStageOnSigning, { number, name: stage.name })
                  : fmt(s.previewStage, { number, name: stage.name, date: day(stage.date) })
            const rows = [<li key={`stage-${index}`}>{line}</li>]
            const minor = plan.plan.payments?.[index]
            if (minor !== undefined) {
              rows.push(
                <li key={`pay-${index}`}>
                  {fmt(s.previewPayment, {
                    description: fmt(s.paymentDescription, { stage: stage.name }),
                    amount: money(minor, currency),
                    number,
                  })}
                </li>,
              )
            }
            return rows
          })}
        </ol>
      </section>
    )
  }

  return (
    <Panel title={s.title}>
      <div role="presentation" onKeyDown={addOnEnter(add)}>
        <p>{s.intro}</p>
        <CountField
          id="split-count"
          label={s.countLabel}
          hint={fmt(s.countHint, { max: Math.min(room, MAX_SPLIT) })}
          value={count}
          onChange={setCount}
          error={errorFor('COUNT')}
        />
        {Array.from({ length: shown }, (_, index) => (
          <div className="pair" key={index}>
            <Field
              label={fmt(s.nameLabel, { number: index + 1 })}
              hint={example(index)}
              id={`stage-name-${index}`}
              required
              error={names[index].trim() === '' ? errorFor('NAME') : null}
            >
              {(control) => (
                <input
                  {...control}
                  type="text"
                  maxLength={200}
                  value={names[index]}
                  onChange={(event) => setNames(set(names, index, event.target.value))}
                />
              )}
            </Field>
            <Field
              label={fmt(s.dueLabel, { number: index + 1 })}
              hint={index === 0 ? s.dueHint : undefined}
              id={`stage-date-${index}`}
              error={index === 0 || !chain ? errorFor('DATE') : null}
            >
              {(control) => (
                <input
                  {...control}
                  type="date"
                  disabled={chain && index > 0}
                  value={chain && index > 0 ? '' : dates[index]}
                  onChange={(event) => setDates(set(dates, index, event.target.value))}
                />
              )}
            </Field>
          </div>
        ))}
        <label className="check">
          <input
            type="checkbox"
            checked={chain}
            aria-describedby="split-chain-hint"
            onChange={(event) => setChain(event.target.checked)}
          />
          <span>{s.chainLabel}</span>
        </label>
        <p className="hint" id="split-chain-hint">
          {s.chainHint}
        </p>
        <label className="check">
          <input
            type="checkbox"
            checked={pay}
            aria-describedby="split-pay-hint"
            onChange={(event) => setPay(event.target.checked)}
          />
          <span>{s.payLabel}</span>
        </label>
        <p className="hint" id="split-pay-hint">
          {s.payHint}
        </p>
        {pay && (
          <AmountFields
            idPrefix="split"
            currency={currency}
            mode={mode}
            onMode={setMode}
            initial=""
            onAmount={setAmount}
            error={errorFor('AMOUNT', 'TOO_SMALL')}
          />
        )}
        {preview}
        <div className="actions">
          <button type="button" className="primary" onClick={add}>
            {s.done}
          </button>
          <button type="button" onClick={onCancel}>
            {wording.common.cancel}
          </button>
        </div>
      </div>
    </Panel>
  )
}
