import type { components, ErrorCode, ExchangeView as Exchange } from '@yuppers/api-client';
import {
  amendmentEffects,
  baseRevision,
  boundToProblemText,
  buildTerms,
  canCompose,
  composerKind,
  CONTRIBUTION_TYPES,
  createDraftSaver,
  decimalForInput,
  draftEffects,
  draftFromTerms,
  dueOf,
  failureCode,
  fractionDigitsOf,
  invitationBoundTo,
  invitationForProblem,
  lockedContributions,
  NAMED_INVITATION,
  newContribution,
  otherSlot,
  parseDecimal,
  problemText as problemMessage,
  revisionToSend,
  startingDraft,
  statusesOf,
  todayIn,
  dueDateZone,
  timeZoneCity,
  toMinorUnits,
  useSignInChannels,
  type Draft,
  type DraftContribution,
  type DraftDue,
  type InvitationChoice,
  type ItemEffect,
  type Problem,
  type ProblemField,
  type RevisionSent,
  type SaveState,
  type Slot,
} from '@yuppers/shared';
import * as Crypto from 'expo-crypto';
import { useEffect, useMemo, useRef, useState } from 'react';
import { View, type ScrollView } from 'react-native';

import { Consent } from '../components/Consent';
import { DateField } from '../components/DateField';
import { HelpLink } from '../components/HelpLink';
import { InvitationFor } from '../components/InvitationLink';
import { ShowWhenSigning } from '../components/ShowPaymentOptions';
import { TermsView } from '../components/TermsView';
import {
  Actions,
  Button,
  Card,
  Check,
  Choice,
  ErrorNote,
  Failure,
  FieldErrorsAnnounced,
  Heading,
  Hint,
  Notice,
  P,
  Panel,
  Screen,
  TextField,
  Written,
} from '../components/ui';
import { useReduceMotion } from '../lib/accessibility';
import { useI18n, useSession } from '../lib/context';
import { deviceTimezone } from '../lib/time-zone';
import { showAfterSigning } from '../lib/payments';
import { api } from '../lib/session';

type ContributionType = components['schemas']['ContributionType'];

interface Props {
  exchange: Exchange;
  reload(): Promise<Exchange | null>;
  /** `boundTo` is who a first proposal's invitation was made for, as typed. */
  onSent(sent: RevisionSent, boundTo: string | null): void;
  /** Leaving without sending. The working copy stays saved as a draft. */
  onLeave(): void;
}

/**
 * Writing terms and sending them: a first proposal on a draft, a counteroffer
 * during negotiation, or an amendment to an agreement in force. The three
 * differ only in what they start from.
 *
 * The working copy saves itself as a draft, which is private and binds
 * nobody. Sending is a separate, deliberate step, because sending signs
 * (DESIGN.md §6, §14.1): it shows the complete terms as they will be sent,
 * then the consent wording, and does nothing until the person agrees.
 */
export function Composer(props: Props) {
  const { wording } = useI18n();

  if (!canCompose(props.exchange)) {
    return (
      <Screen>
        <Heading>{wording.composer.titleCounter}</Heading>
        <P>{wording.composer.notAvailable}</P>
        <Actions>
          <Button label={wording.exchange.titleNoName} onPress={props.onLeave} />
        </Actions>
      </Screen>
    );
  }
  return <Editor {...props} />;
}

function Editor({ exchange, reload, onSent, onLeave }: Props) {
  const { wording, fmt, language, money } = useI18n();
  const { account } = useSession();
  const w = wording.composer;

  const you = exchange.you;
  const other = otherSlot(you);
  const kind = composerKind(exchange);
  // What a counteroffer or an amendment starts from: the terms on the table.
  const base = baseRevision(exchange);
  const digits = fractionDigitsOf(exchange.currency);

  const [draft, setDraft] = useState<Draft>(() =>
    startingDraft(exchange, account?.display_name ?? '', digits),
  );
  // Bumped to rebuild the inputs when the whole working copy is replaced.
  const [generation, setGeneration] = useState(0);
  const [step, setStep] = useState<'edit' | 'sign'>('edit');
  const [checked, setChecked] = useState(false);
  const [conflict, setConflict] = useState(false);
  const [invitee, setInvitee] = useState<InvitationChoice>(NAMED_INVITATION);
  const [busy, setBusy] = useState(false);
  const [failure, setFailure] = useState<ErrorCode | null>(null);
  const [saveState, setSaveState] = useState<SaveState>('idle');
  const [discarding, setDiscarding] = useState(false);
  const [discardFailure, setDiscardFailure] = useState<ErrorCode | null>(null);
  // Showing payment options on this yup too, once the terms are sent.
  const [alsoShow, setAlsoShow] = useState(false);
  const scroll = useRef<ScrollView>(null);
  const reduceMotion = useReduceMotion();
  // Who a first proposal's invitation is for is asked for beside their
  // name, and checked with the rest before the signing step.
  const channels = useSignInChannels(api);
  const boundProblem = kind === 'first' && checked ? invitationForProblem(invitee, channels) : null;

  // The working copy was started from terms that have since been replaced.
  const stale = base !== null && draft.base !== base.id;

  // An amendment's effect on each item of the agreement, predicted from the
  // rule the service applies (DESIGN.md §7), so nothing about it is a
  // surprise after signing.
  const inForce = kind === 'amend' ? (exchange.in_force_revision ?? null) : null;
  const effects = useMemo(
    () => (inForce ? draftEffects(draft, inForce.terms, statusesOf(exchange), digits) : null),
    [draft, inForce, exchange, digits],
  );
  const effectOf = (id: string) => effects?.find((item) => item.id === id) ?? null;
  // Items of the agreement the working copy no longer has.
  const dropped = (effects ?? []).filter(
    (item) => !draft.contributions.some((contribution) => contribution.id === item.id),
  );

  // ---- Saving the working copy ----------------------------------------------

  const latest = useRef(draft);
  const exchangeId = exchange.id;
  // One save at a time, a moment after the typing pauses.
  const [saver] = useState(() =>
    createDraftSaver({
      save: (copy) => api.saveDraft(exchangeId, copy),
      onState: setSaveState,
    }),
  );

  function edit(next: Draft) {
    latest.current = next;
    setDraft(next);
    saver.changed(next);
  }

  // Leaving the screen keeps what was typed in the last moment.
  useEffect(() => () => saver.leave(), [saver]);

  // ---- Editing -----------------------------------------------------------------

  const change = (patch: Partial<Draft>) => edit({ ...latest.current, ...patch });
  const changeItem = (id: string, patch: Partial<DraftContribution>) =>
    change({
      contributions: latest.current.contributions.map((item) =>
        item.id === id ? { ...item, ...patch } : item,
      ),
    });

  function addItem(from: Slot) {
    // The contribution's id is chosen here and stays with it for the life of
    // the exchange (DESIGN.md §7).
    const id = Crypto.randomUUID();
    change({ contributions: [...latest.current.contributions, newContribution(id, from)] });
  }

  const built = useMemo(() => buildTerms(draft, digits, base?.terms), [draft, digits, base]);
  const problems = checked && !built.ok ? built.problems : [];
  const problemCount = problems.length + (boundProblem ? 1 : 0);
  const problemText = (problem: Problem) => problemMessage(problem, wording, language);

  function errorFor(field: ProblemField, contribution?: string): string | null {
    const found = problems.find(
      (problem) => problem.field === field && problem.contribution === contribution,
    );
    return found ? problemText(found) : null;
  }

  function review() {
    setChecked(true);
    setConflict(false);
    setFailure(null);
    if (built.ok && !(kind === 'first' && invitationForProblem(invitee, channels))) {
      setStep('sign');
      return;
    }
    // What needs fixing is summed up at the top and marked on each field.
    scroll.current?.scrollTo({ y: 0, animated: !reduceMotion });
  }

  // ---- Sending -------------------------------------------------------------------

  async function send() {
    if (!built.ok) return;
    setBusy(true);
    setFailure(null);
    // The service drops the working copy when the revision is sent. A save
    // still on its way must not put it back afterwards.
    await saver.settle();
    try {
      const result = await api.sendRevision(
        exchange.id,
        revisionToSend(exchange, built, language, invitationBoundTo(invitee) ?? ''),
      );
      saver.sent();
      await showAfterSigning(exchange.id, alsoShow);
      onSent(result, kind === 'first' ? invitationBoundTo(invitee) : null);
    } catch (error) {
      const code = failureCode(error);
      saver.resume();
      if (code === 'VERSION_CONFLICT') {
        // The exchange moved on. Nothing was sent; show what it is now and
        // keep what was written.
        await reload();
        setConflict(true);
        setStep('edit');
      } else setFailure(code);
      setBusy(false);
    }
  }

  // ---- Discarding a draft never sent ----------------------------------------------

  async function discard() {
    setBusy(true);
    setDiscardFailure(null);
    // The working copy goes with the draft; a save still on its way must not
    // be refused noisily, or put anything back.
    await saver.settle();
    try {
      await api.runCommand(exchange.id, exchange.version, { type: 'DISCARD' });
      saver.sent();
      onLeave();
    } catch (error) {
      saver.resume();
      setDiscardFailure(failureCode(error));
      setBusy(false);
    }
  }

  const title = kind === 'first' ? w.titleFirst : kind === 'amend' ? w.titleAmend : w.titleCounter;

  if (step === 'sign' && built.ok) {
    // What signing this amendment does, from the terms exactly as they go.
    const predicted = inForce
      ? amendmentEffects(inForce.terms, statusesOf(exchange), built.terms.contributions)
      : null;
    return (
      // A screen of its own, so it opens at the top: the terms are read before
      // the way to sign them is reached.
      <Screen key="sign">
        <Heading>{w.signTitle}</Heading>
        <P>{w.signIntro}</P>
        {built.note ? (
          <>
            <Heading level={2}>{w.yourNote}</Heading>
            <Written>{built.note}</Written>
            <Hint>{w.noteHint}</Hint>
          </>
        ) : null}
        <Card>
          <TermsView
            terms={built.terms}
            currency={exchange.currency}
            timezone={exchange.timezone}
            you={you}
          />
        </Card>
        {predicted && <Effects effects={predicted} />}
        {kind === 'first' ? (
          <P>
            {invitee.anyone
              ? wording.invitationLink.forAnyoneSummary
              : fmt(wording.invitationLink.boundSummary, { identifier: invitee.to.trim() })}
          </P>
        ) : null}
        <ShowWhenSigning
          terms={built.terms}
          you={you}
          shown={Boolean(exchange.payment_options?.shown)}
          value={alsoShow}
          onChange={setAlsoShow}
        />
        <Consent
          signLabel={w.signAndSend}
          busy={busy}
          failure={failure}
          onSign={() => void send()}
          onCancel={() => setStep('edit')}
          cancelLabel={w.backToEdit}
        />
      </Screen>
    );
  }

  // An accepted contribution is locked: an amendment may not touch it.
  const locked = lockedContributions(exchange);
  const nameOf = (slot: Slot) => (slot === 'A' ? draft.partyA : draft.partyB);
  const setName = (slot: Slot, name: string) =>
    change(slot === 'A' ? { partyA: name } : { partyB: name });
  const general = problems.filter((problem) => problem.field === 'contributions');
  const today = todayIn(exchange.timezone);
  // Due dates are read in the exchange's zone; named when this device keeps another.
  const zone = dueDateZone(exchange.timezone, deviceTimezone());

  const fromOptions = [
    { value: you, label: wording.party.you },
    { value: other, label: wording.party.other },
  ];
  const typeOptions = CONTRIBUTION_TYPES.map((type) => ({
    value: type,
    label: wording.contributionTypes[type],
  }));
  const dueOptions: { value: DraftDue['kind']; label: string }[] = [
    { value: 'ON_AGREEMENT', label: w.dueOnAgreement },
    { value: 'DATE', label: w.dueOnDate },
    { value: 'AFTER_CONTRIBUTION', label: w.dueAfter },
  ];

  return (
    <Screen key={`edit-${generation}`} scroll={scroll}>
      {/* What needs fixing is announced once, as a count at the top; each
          field's own error is read when the person gets to it. */}
      <FieldErrorsAnnounced value={false}>
        <Heading>{title}</Heading>
        <P>{kind === 'first' ? w.introFirst : kind === 'amend' ? w.introAmend : w.introCounter}</P>
        {kind === 'amend' && (
          <>
            <P>{w.effectsSteer}</P>
            <HelpLink place="amendment" />
          </>
        )}

        {conflict && <ErrorNote>{w.conflict}</ErrorNote>}
        {stale && base && (
          <Notice>
            <P>{w.staleDraft}</P>
            <Actions>
              <Button label={w.staleDraftKeep} onPress={() => change({ base: base.id })} />
              <Button
                label={w.staleDraftDiscard}
                onPress={() => {
                  edit(draftFromTerms(base.terms, base.id, digits));
                  setGeneration((count) => count + 1);
                  setChecked(false);
                }}
              />
            </Actions>
          </Notice>
        )}
        {problemCount > 0 && (
          <ErrorNote>{fmt(w.problemsSummary, { count: problemCount })}</ErrorNote>
        )}

        <Heading level={2}>{w.partiesLegend}</Heading>
        <TextField
          label={w.yourName}
          required
          error={errorFor(you === 'A' ? 'partyA' : 'partyB')}
          maxLength={100}
          defaultValue={nameOf(you)}
          onChangeText={(name) => setName(you, name)}
        />
        <TextField
          label={w.otherName}
          required
          error={errorFor(other === 'A' ? 'partyA' : 'partyB')}
          maxLength={100}
          defaultValue={nameOf(other)}
          onChangeText={(name) => setName(other, name)}
        />
        {kind === 'first' && (
          <InvitationFor
            choice={invitee}
            onChange={setInvitee}
            channels={channels}
            error={
              boundProblem
                ? boundToProblemText(boundProblem, wording.invitationLink, channels, fmt)
                : null
            }
          />
        )}

        <TextField
          label={w.termsLabel}
          hint={w.termsHint}
          multiline
          defaultValue={draft.terms}
          onChangeText={(terms) => change({ terms })}
        />

        <Heading level={2}>{w.itemsHeading}</Heading>
        {draft.contributions.map((item, index) => {
          const number = index + 1;
          const fixed = locked.has(item.id);
          const minor =
            item.type === 'MONEY' && item.amount ? toMinorUnits(item.amount, digits) : null;
          const candidates = draft.contributions
            .map((candidate, position) => ({ candidate, position }))
            .filter(({ candidate }) => candidate.id !== item.id)
            .map(({ candidate, position }) => ({
              value: candidate.id,
              label: candidate.description.trim()
                ? fmt(w.itemOption, {
                    number: position + 1,
                    description: candidate.description.trim(),
                  })
                : fmt(w.itemOptionBlank, { number: position + 1 }),
            }));
          const effect = effectOf(item.id);
          const refused = effect?.effect === 'LOCKED' || effect?.effect === 'REUSED';
          return (
            <Card key={item.id}>
              <Heading level={3}>{fmt(w.itemLegend, { number })}</Heading>
              {fixed && <Notice quiet>{w.locked}</Notice>}
              {/* What the amendment does to this item, as it is being written. */}
              {effect && !fixed ? (
                refused ? (
                  <ErrorNote>{w.effects[effect.effect]}</ErrorNote>
                ) : (
                  <Hint>{w.effects[effect.effect]}</Hint>
                )
              ) : null}

              <Choice<Slot>
                label={w.fromLabel}
                value={item.from}
                options={fromOptions}
                disabled={fixed}
                onChange={(from) => changeItem(item.id, { from })}
              />
              <Choice<ContributionType>
                label={w.typeLabel}
                value={item.type}
                options={typeOptions}
                disabled={fixed}
                onChange={(type) => changeItem(item.id, { type })}
              />

              <TextField
                label={w.descriptionLabel}
                required
                error={errorFor('description', item.id)}
                multiline
                disabled={fixed}
                defaultValue={item.description}
                onChangeText={(description) => changeItem(item.id, { description })}
              />

              {item.type === 'MONEY' ? (
                <>
                  <DecimalField
                    key="amount"
                    label={fmt(w.amountLabel, { currency: exchange.currency })}
                    required
                    error={errorFor('amount', item.id)}
                    disabled={fixed}
                    value={item.amount}
                    onChange={(amount) => changeItem(item.id, { amount })}
                  />
                  {/* What the typed number will be signed as. */}
                  {minor !== null && (
                    <Hint>{fmt(w.amountPreview, { amount: money(minor, exchange.currency) })}</Hint>
                  )}
                  {/* Money is paid outside the product and only recorded here (DESIGN.md §11). */}
                  <Hint>{w.moneyOutside}</Hint>
                  <HelpLink place="moneyOutside" />
                </>
              ) : (
                <>
                  <DecimalField
                    key="quantity"
                    label={w.quantityLabel}
                    error={errorFor('quantity', item.id)}
                    disabled={fixed}
                    value={item.quantity}
                    onChange={(quantity) => changeItem(item.id, { quantity })}
                  />
                  <TextField
                    label={w.unitLabel}
                    maxLength={40}
                    disabled={fixed}
                    autoCapitalize="none"
                    defaultValue={item.unit}
                    onChangeText={(unit) => changeItem(item.id, { unit })}
                  />
                </>
              )}

              <Choice<DraftDue['kind']>
                label={w.dueLabel}
                value={item.due.kind}
                options={dueOptions}
                disabled={fixed}
                onChange={(due) => {
                  if (due !== item.due.kind) changeItem(item.id, { due: dueOf(due) });
                }}
              />
              {item.due.kind === 'DATE' && (
                <DateField
                  label={w.dateLabel}
                  hint={zone ? fmt(w.dateInZone, { zone: timeZoneCity(zone) }) : undefined}
                  error={errorFor('date', item.id)}
                  value={item.due.date}
                  today={today}
                  disabled={fixed}
                  onChange={(date) => changeItem(item.id, { due: { kind: 'DATE', date } })}
                />
              )}
              {item.due.kind === 'AFTER_CONTRIBUTION' && (
                <Choice
                  label={w.afterLabel}
                  hint={item.due.contribution === '' ? w.afterChoose : undefined}
                  error={errorFor('after', item.id)}
                  value={item.due.contribution || null}
                  options={candidates}
                  disabled={fixed}
                  onChange={(contribution) =>
                    changeItem(item.id, { due: { kind: 'AFTER_CONTRIBUTION', contribution } })
                  }
                />
              )}

              <TextField
                label={w.criteriaLabel}
                multiline
                disabled={fixed}
                defaultValue={item.criteria}
                onChangeText={(criteria) => changeItem(item.id, { criteria })}
              />

              <Check
                label={w.requiredLabel}
                value={item.required}
                disabled={fixed}
                onChange={(required) => changeItem(item.id, { required })}
              />

              <Actions>
                <Button
                  label={fmt(w.remove, { number })}
                  disabled={fixed}
                  onPress={() =>
                    change({
                      contributions: latest.current.contributions.filter(
                        (candidate) => candidate.id !== item.id,
                      ),
                    })
                  }
                />
              </Actions>
            </Card>
          );
        })}

        {/* Items of the agreement this change removes, named as the agreement wrote them. */}
        {dropped.length > 0 && (
          <Notice quiet>
            <P>{fmt(w.removedCount, { count: dropped.length })}</P>
            {dropped.map((item) => (
              <View key={item.id}>
                <Written>{item.description}</Written>
                <Hint>{w.effects[item.effect]}</Hint>
              </View>
            ))}
          </Notice>
        )}

        {general.map((problem) => (
          <ErrorNote key={problem.code}>{problemText(problem)}</ErrorNote>
        ))}
        <Actions>
          <Button label={w.addYours} onPress={() => addItem(you)} />
          <Button label={w.addTheirs} onPress={() => addItem(other)} />
        </Actions>

        <TextField
          label={w.noteLabel}
          hint={w.noteHint}
          error={errorFor('note')}
          multiline
          defaultValue={draft.note}
          onChangeText={(note) => change({ note })}
        />

        {saveState === 'saving' && <Hint>{w.saving}</Hint>}
        {saveState === 'saved' && <Hint>{w.saved}</Hint>}
        {saveState === 'failed' && <ErrorNote>{w.saveFailed}</ErrorNote>}

        <Actions>
          <Button variant="primary" label={w.review} disabled={stale || busy} onPress={review} />
          <Button
            label={kind === 'first' ? wording.nav.exchanges : wording.common.cancel}
            onPress={onLeave}
          />
          {/* A draft never sent can be thrown away; afterwards it is closed and out of
              the list. */}
          {kind === 'first' && (
            <Button
              label={w.discard}
              expanded={discarding}
              disabled={busy}
              onPress={() => setDiscarding(true)}
            />
          )}
        </Actions>
        {discarding && (
          <Panel title={w.discard}>
            <P>{w.discardText}</P>
            <Failure code={discardFailure} />
            <Actions>
              <Button
                variant="primary"
                label={w.confirmDiscard}
                disabled={busy}
                onPress={() => void discard()}
              />
              <Button
                label={wording.common.cancel}
                disabled={busy}
                onPress={() => setDiscarding(false)}
              />
            </Actions>
          </Panel>
        )}
      </FieldErrorsAnnounced>
    </Screen>
  );
}

/**
 * What an amendment does to each item of the agreement once both have signed
 * it, from the same rule the service applies (DESIGN.md §7): untouched items
 * keep their status, a changed one goes back to the start, a removed one
 * leaves, a new one starts, and a confirmed one cannot be touched.
 */
function Effects({ effects }: { effects: readonly ItemEffect[] }) {
  const { wording } = useI18n();
  const w = wording.composer;
  return (
    <Card>
      <Heading level={2}>{w.effectsHeading}</Heading>
      <P>{w.effectsIntro}</P>
      {effects.map((item) => (
        <View key={item.id}>
          <Written>{item.description}</Written>
          {item.effect === 'LOCKED' || item.effect === 'REUSED' ? (
            <ErrorNote>{w.effects[item.effect]}</ErrorNote>
          ) : (
            <P>{w.effects[item.effect]}</P>
          )}
        </View>
      ))}
      <Hint>{w.effectsSteer}</Hint>
    </Card>
  );
}

interface DecimalFieldProps {
  label: string;
  error: string | null;
  disabled: boolean;
  /** A plain decimal, empty for none, `null` when what is typed is not a number. */
  value: string | null;
  onChange(value: string | null): void;
  required?: boolean;
}

/**
 * A number typed the way the reader's language writes numbers. What is typed
 * stays on screen as typed; the working copy gets the plain form, or `null`
 * while it cannot be read as a number.
 */
function DecimalField({ label, error, disabled, required, value, onChange }: DecimalFieldProps) {
  const { language } = useI18n();
  const [text, setText] = useState(() => (value ? decimalForInput(value, language) : ''));
  return (
    <TextField
      label={label}
      error={error}
      required={required}
      disabled={disabled}
      inputMode="decimal"
      autoComplete="off"
      value={text}
      onChangeText={(typed) => {
        setText(typed);
        onChange(typed.trim() === '' ? '' : parseDecimal(typed, language));
      }}
    />
  );
}
