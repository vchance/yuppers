import {
  addMonths,
  decimalForInput,
  fractionDigitsOf,
  otherSlot,
  parseDecimal,
  planInstalments,
  planStages,
  splitIntoInstalments,
  splitIntoStages,
  splitRoom,
  type AmountChoice,
  type DraftContribution,
  type Every,
  type SplitGroup,
} from '@yuppers/shared';
import { Fragment, useState } from 'react';

import { announce } from '../lib/accessibility';
import { useI18n } from '../lib/context';
import { DateField } from './DateField';
import { Actions, Button, Check, Choice, Hint, P, Panel, TextField } from './ui';

/*
 * The two split sheets of the composer: a money item into instalments
 * (DESIGN.md §7.1) and a service or task item into stages (DESIGN.md §7.2).
 * Each shows every amount and date it will add before anything is added.
 */

export interface SplitResult {
  items: DraftContribution[];
  group: SplitGroup;
}

interface SheetProps {
  item: DraftContribution;
  /** How many items the working copy holds now. */
  existing: number;
  currency: string;
  /** Today in the exchange's time zone. */
  today: string;
  /** Makes the id of a new item. */
  newId(): string;
  onDone(result: SplitResult): void;
  onCancel(): void;
}

type EveryKind = Every['kind'];

/** The typed number as a plain decimal, `''` for nothing and `null` when it cannot be read. */
function plain(typed: string, language: string): string | null {
  return typed.trim() === '' ? '' : parseDecimal(typed, language);
}

/** A whole number as typed, or NaN. */
function whole(typed: string): number {
  return typed.trim() === '' ? NaN : Number(typed);
}

/** The amount choice shared by both sheets: how it is worked out, and the amount. */
function Amounts({
  mode,
  onMode,
  text,
  onText,
  currency,
  error,
}: {
  mode: AmountChoice['mode'];
  onMode(mode: AmountChoice['mode']): void;
  text: string;
  onText(text: string): void;
  currency: string;
  error: string | null;
}) {
  const { wording, fmt } = useI18n();
  const w = wording.composer.split.amounts;
  return (
    <>
      <Choice<AmountChoice['mode']>
        label={w.howLegend}
        value={mode}
        options={[
          { value: 'SHARE', label: w.howShare },
          { value: 'EACH', label: w.howEach },
        ]}
        onChange={onMode}
      />
      <TextField
        label={fmt(mode === 'SHARE' ? w.shareLabel : w.eachLabel, { currency })}
        hint={mode === 'SHARE' ? w.remainderHint : undefined}
        error={error}
        inputMode="decimal"
        autoComplete="off"
        value={text}
        onChangeText={onText}
      />
    </>
  );
}

export function InstalmentsSheet({
  item,
  existing,
  currency,
  today,
  newId,
  onDone,
  onCancel,
}: SheetProps) {
  const { wording, fmt, language, money, day } = useI18n();
  const w = wording.composer.split.instalments;
  const digits = fractionDigitsOf(currency);
  const room = splitRoom(existing);
  const [count, setCount] = useState(String(Math.min(3, room)));
  const [mode, setMode] = useState<AmountChoice['mode']>('SHARE');
  const [amount, setAmount] = useState(() =>
    item.amount ? decimalForInput(item.amount, language) : '',
  );
  const [first, setFirst] = useState(() => addMonths(today, 1));
  const [every, setEvery] = useState<EveryKind>('MONTH');
  const [days, setDays] = useState('10');
  const [tried, setTried] = useState(false);

  const describe = (number: number, total: number) =>
    item.description.trim()
      ? fmt(wording.composer.split.ordinal, {
          description: item.description.trim(),
          number,
          count: total,
        })
      : fmt(wording.composer.split.ordinalBlank, { number, count: total });

  const planned = planInstalments(
    {
      count: whole(count),
      amounts: { mode, amount: plain(amount, language) },
      first,
      every: every === 'DAYS' ? { kind: 'DAYS', days: whole(days) } : { kind: every },
    },
    digits,
    room,
  );
  const problems: readonly string[] = planned.ok ? [] : planned.problems;
  const message = (kinds: readonly string[]) => {
    const found = tried ? problems.find((problem) => kinds.includes(problem)) : undefined;
    return found
      ? fmt(w.problems[found as keyof typeof w.problems], {
          max: room,
          example: decimalForInput('25.50', language),
        })
      : null;
  };

  function add() {
    setTried(true);
    if (!planned.ok) return;
    const result = splitIntoInstalments(item, planned.rows, describe, newId, digits);
    announce(fmt(w.added, { count: result.items.length }));
    onDone(result);
  }

  const everyOptions: { value: EveryKind; label: string }[] = [
    { value: 'WEEK', label: w.everyWeek },
    { value: 'TWO_WEEKS', label: w.everyTwoWeeks },
    { value: 'MONTH', label: w.everyMonth },
    { value: 'DAYS', label: w.everyDays },
  ];

  return (
    <Panel title={w.title}>
      <P>{w.intro}</P>
      <TextField
        label={w.countLabel}
        hint={fmt(w.countHint, { max: room })}
        error={message(['COUNT'])}
        inputMode="numeric"
        autoComplete="off"
        value={count}
        onChangeText={setCount}
      />
      <Amounts
        mode={mode}
        onMode={setMode}
        text={amount}
        onText={setAmount}
        currency={currency}
        error={message(['AMOUNT', 'TOO_SMALL'])}
      />
      <DateField
        label={w.firstLabel}
        error={message(['DATE'])}
        value={first}
        today={today}
        onChange={setFirst}
      />
      <Choice<EveryKind>
        label={w.everyLegend}
        hint={every === 'MONTH' ? w.monthHint : undefined}
        value={every}
        options={everyOptions}
        onChange={setEvery}
      />
      {every === 'DAYS' && (
        <TextField
          label={w.daysLabel}
          inputMode="numeric"
          autoComplete="off"
          error={message(['DAYS'])}
          value={days}
          onChangeText={setDays}
        />
      )}
      {planned.ok && (
        <>
          <P>{w.previewHeading}</P>
          {planned.rows.map((row, index) => (
            <P key={index}>
              {fmt(w.previewLine, {
                description: describe(index + 1, planned.rows.length),
                amount: money(row.amountMinor, currency),
                date: day(row.date),
              })}
            </P>
          ))}
        </>
      )}
      <Hint>{w.plainHint}</Hint>
      <Actions>
        <Button variant="primary" label={w.done} onPress={add} />
        <Button label={wording.common.cancel} onPress={onCancel} />
      </Actions>
    </Panel>
  );
}

export function StagesSheet({
  item,
  existing,
  currency,
  today,
  newId,
  onDone,
  onCancel,
}: SheetProps) {
  const { wording, fmt, language, money, day } = useI18n();
  const w = wording.composer.split.stages;
  const c = wording.composer;
  const digits = fractionDigitsOf(currency);
  // With a payment for each stage a stage takes two items.
  const [pay, setPay] = useState(false);
  const room = splitRoom(existing, pay ? 2 : 1);
  const [count, setCount] = useState(String(Math.min(3, room)));
  const [names, setNames] = useState<string[]>([]);
  const [dates, setDates] = useState<string[]>([]);
  const [chain, setChain] = useState(false);
  const [mode, setMode] = useState<AmountChoice['mode']>('SHARE');
  const [amount, setAmount] = useState('');
  const [tried, setTried] = useState(false);

  const total = whole(count);
  const shownCount = Number.isInteger(total) && total >= 2 && total <= room ? total : 0;
  const set = (list: string[], at: number, value: string) => {
    const next = Array.from({ length: Math.max(list.length, at + 1) }, (_, i) => list[i] ?? '');
    next[at] = value;
    return next;
  };
  const planned = planStages(
    {
      count: total,
      names,
      dates,
      chain,
      pay: pay ? { mode, amount: plain(amount, language) } : null,
    },
    digits,
    room,
  );
  const problems: readonly string[] = planned.ok ? [] : planned.problems;
  const message = (kinds: readonly string[]) => {
    const found = tried ? problems.find((problem) => kinds.includes(problem)) : undefined;
    return found
      ? fmt(w.problems[found as keyof typeof w.problems], {
          max: room,
          example: decimalForInput('25.50', language),
        })
      : null;
  };
  const examples = [w.exampleOne, w.exampleTwo, w.exampleThree];

  function add() {
    setTried(true);
    if (!planned.ok) return;
    const result = splitIntoStages(
      item,
      planned.plan,
      { chain, payer: otherSlot(item.from) },
      (stage) => fmt(w.paymentDescription, { stage }),
      newId,
      digits,
    );
    announce(fmt(w.added, { count: result.items.length }));
    onDone(result);
  }

  return (
    <Panel title={w.title}>
      <P>{w.intro}</P>
      <TextField
        label={w.countLabel}
        hint={fmt(w.countHint, { max: room })}
        error={message(['COUNT'])}
        inputMode="numeric"
        autoComplete="off"
        value={count}
        onChangeText={setCount}
      />
      {Array.from({ length: shownCount }, (_, index) => {
        const date = dates[index] ?? '';
        return (
          <Fragment key={index}>
            <TextField
              label={fmt(w.nameLabel, { number: index + 1 })}
              hint={examples[index % examples.length]}
              error={
                problems.includes('NAME') && tried && (names[index] ?? '').trim() === ''
                  ? message(['NAME'])
                  : null
              }
              maxLength={200}
              value={names[index] ?? ''}
              onChangeText={(name) => setNames((list) => set(list, index, name))}
            />
            {!(chain && index > 0) && (
              <>
                <Choice<'ON_AGREEMENT' | 'DATE'>
                  label={fmt(w.dueLabel, { number: index + 1 })}
                  hint={index === 0 ? w.dueHint : undefined}
                  value={date !== '' ? 'DATE' : 'ON_AGREEMENT'}
                  options={[
                    { value: 'ON_AGREEMENT', label: c.dueOnAgreement },
                    { value: 'DATE', label: c.dueOnDate },
                  ]}
                  onChange={(due) =>
                    setDates((list) => set(list, index, due === 'DATE' ? addMonths(today, 1) : ''))
                  }
                />
                {date !== '' && (
                  <DateField
                    label={c.dateLabel}
                    error={message(['DATE'])}
                    value={date}
                    today={today}
                    onChange={(next) => setDates((list) => set(list, index, next))}
                  />
                )}
              </>
            )}
          </Fragment>
        );
      })}
      <Check label={w.chainLabel} hint={w.chainHint} value={chain} onChange={setChain} />
      <Check label={w.payLabel} hint={w.payHint} value={pay} onChange={setPay} />
      {pay && (
        <Amounts
          mode={mode}
          onMode={setMode}
          text={amount}
          onText={setAmount}
          currency={currency}
          error={message(['AMOUNT', 'TOO_SMALL'])}
        />
      )}
      {planned.ok && (
        <>
          <P>{w.previewHeading}</P>
          {planned.plan.stages.flatMap((stage, index) => {
            const lines = [
              fmt(
                chain && index > 0
                  ? w.previewStageChained
                  : stage.date === ''
                    ? w.previewStageOnSigning
                    : w.previewStage,
                { number: index + 1, name: stage.name, date: stage.date ? day(stage.date) : '' },
              ),
            ];
            if (planned.plan.payments) {
              lines.push(
                fmt(w.previewPayment, {
                  description: fmt(w.paymentDescription, { stage: stage.name }),
                  amount: money(planned.plan.payments[index], currency),
                  number: index + 1,
                }),
              );
            }
            return lines.map((line, at) => <P key={`${index}-${at}`}>{line}</P>);
          })}
        </>
      )}
      <Actions>
        <Button variant="primary" label={w.done} onPress={add} />
        <Button label={wording.common.cancel} onPress={onCancel} />
      </Actions>
    </Panel>
  );
}
