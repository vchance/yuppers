import type { components, ErrorCode, ExchangeView as Exchange } from '@yuppers/api-client'
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
  toMinorUnits,
  type Draft,
  type DraftContribution,
  type DraftDue,
  type InvitationChoice,
  type ItemEffect,
  type Problem,
  type ProblemField,
  type SaveState,
  dueDateZone,
  timeZoneCity,
  useSignInChannels,
} from '@yuppers/shared'
import { useEffect, useMemo, useRef, useState } from 'react'

import { useI18n, useSession } from '../app/context'
import { Link } from '../app/Link'
import { navigate } from '../app/router'
import { paths } from '../app/routes'
import { Consent } from '../components/Consent'
import { HelpLink } from '../components/HelpLink'
import { InvitationFor } from '../components/InvitationLink'
import { Panel } from '../components/Panel'
import { ShowWhenSigning } from '../components/ShowWhenSigning'
import { TermsView } from '../components/TermsView'
import {
  ErrorNote,
  Failure,
  Field,
  PageHeading,
  Written,
  type ControlProps,
} from '../components/ui'
import { useAnnouncement } from '../lib/announce'
import { showAfterSigning } from '../lib/payments'
import { api, failureCode, type RevisionSent, type Slot } from '../lib/api'

type ContributionType = components['schemas']['ContributionType']

/** The field for who a first proposal's invitation is for. */
const BOUND_TO = 'bound-to'

interface Props {
  exchange: Exchange
  reload(): Promise<Exchange | null>
  /** `boundTo` is who a first proposal's invitation was made for, as typed. */
  onSent(sent: RevisionSent, boundTo: string | null): void
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
export default function Composer(props: Props) {
  const { wording } = useI18n()
  const { exchange } = props

  if (!canCompose(exchange)) {
    return (
      <>
        <PageHeading>{wording.composer.titleCounter}</PageHeading>
        <p>{wording.composer.notAvailable}</p>
        <p>
          <Link to={paths.exchange(exchange.id)}>{wording.exchange.titleNoName}</Link>
        </p>
      </>
    )
  }
  return <Editor {...props} />
}

function Editor({ exchange, reload, onSent }: Props) {
  const { wording, fmt, language, money } = useI18n()
  const { account } = useSession()
  const w = wording.composer

  const you = exchange.you
  const other = otherSlot(you)
  const kind = composerKind(exchange)
  // What a counteroffer or an amendment starts from: the terms on the table.
  const base = baseRevision(exchange)
  const digits = fractionDigitsOf(exchange.currency)

  const [draft, setDraft] = useState<Draft>(() =>
    startingDraft(exchange, account?.display_name ?? '', digits),
  )
  // Bumped to rebuild the inputs when the whole working copy is replaced.
  const [generation, setGeneration] = useState(0)
  const [step, setStep] = useState<'edit' | 'sign'>('edit')
  const [returned, setReturned] = useState(false)
  const [checked, setChecked] = useState(false)
  const [conflict, setConflict] = useState(false)
  const [invitee, setInvitee] = useState<InvitationChoice>(NAMED_INVITATION)
  const [busy, setBusy] = useState(false)
  const [failure, setFailure] = useState<ErrorCode | null>(null)
  const [saveState, setSaveState] = useState<SaveState>('idle')
  const [added, setAdded] = useState<string | null>(null)
  const [discarding, setDiscarding] = useState(false)
  const [discardFailure, setDiscardFailure] = useState<ErrorCode | null>(null)
  // Showing payment options on this yup too, once the terms are sent.
  const [alsoShow, setAlsoShow] = useState(false)
  // Who a first proposal's invitation is for is asked for beside their
  // name, and checked with the rest before the signing step.
  const channels = useSignInChannels(api)
  const boundProblem = kind === 'first' && checked ? invitationForProblem(invitee, channels) : null

  // The working copy was started from terms that have since been replaced.
  const stale = base !== null && draft.base !== base.id

  // An amendment's effect on each item of the agreement, predicted from the
  // rule the service applies (DESIGN.md §7), so nothing about it is a
  // surprise after signing.
  const inForce = kind === 'amend' ? (exchange.in_force_revision ?? null) : null
  const effects = useMemo(
    () => (inForce ? draftEffects(draft, inForce.terms, statusesOf(exchange), digits) : null),
    [draft, inForce, exchange, digits],
  )
  const effectOf = (id: string) => effects?.find((item) => item.id === id) ?? null
  // Items of the agreement the working copy no longer has.
  const dropped = (effects ?? []).filter(
    (item) => !draft.contributions.some((contribution) => contribution.id === item.id),
  )

  // ---- Saving the working copy ----------------------------------------------

  const latest = useRef(draft)
  const exchangeId = exchange.id
  // One save at a time, a moment after the typing pauses.
  const [saver] = useState(() =>
    createDraftSaver({
      save: (copy) => api.saveDraft(exchangeId, copy),
      onState: setSaveState,
    }),
  )

  function edit(next: Draft) {
    latest.current = next
    setDraft(next)
    saver.changed(next)
  }

  // Leaving the page keeps what was typed in the last moment.
  useEffect(() => () => saver.leave(), [saver])

  // ---- Editing -----------------------------------------------------------------

  const change = (patch: Partial<Draft>) => edit({ ...latest.current, ...patch })
  const changeItem = (id: string, patch: Partial<DraftContribution>) =>
    change({
      contributions: latest.current.contributions.map((item) =>
        item.id === id ? { ...item, ...patch } : item,
      ),
    })

  function addItem(from: Slot) {
    const id = crypto.randomUUID()
    change({ contributions: [...latest.current.contributions, newContribution(id, from)] })
    setAdded(id)
  }

  // A new item starts with the keyboard in it.
  useEffect(() => {
    if (added) document.getElementById(`${added}-description`)?.focus()
  }, [added])

  const built = useMemo(() => buildTerms(draft, digits, base?.terms), [draft, digits, base])
  const problems = checked && !built.ok ? built.problems : []
  const problemCount = problems.length + (boundProblem ? 1 : 0)
  const problemText = (problem: Problem) => problemMessage(problem, wording, language)

  function errorFor(field: ProblemField, contribution?: string): string | null {
    const found = problems.find(
      (problem) => problem.field === field && problem.contribution === contribution,
    )
    return found ? problemText(found) : null
  }

  function fieldId(problem: Problem): string {
    if (problem.contribution) return `${problem.contribution}-${problem.field}`
    if (problem.field === 'partyA') return 'party-A'
    if (problem.field === 'partyB') return 'party-B'
    if (problem.field === 'note') return 'note'
    return 'add-yours'
  }

  function review() {
    setChecked(true)
    setConflict(false)
    setFailure(null)
    const bound = kind === 'first' ? invitationForProblem(invitee, channels) : null
    if (built.ok && !bound) {
      setStep('sign')
      setReturned(true)
      return
    }
    // The keyboard goes to the first thing to fix, once it has been marked:
    // the names come before who the invitation is for, and the rest after.
    const firstProblem = built.ok ? null : built.problems[0]
    const nameFirst = firstProblem?.field === 'partyA' || firstProblem?.field === 'partyB'
    const first = firstProblem && (nameFirst || !bound) ? fieldId(firstProblem) : BOUND_TO
    window.setTimeout(() => document.getElementById(first)?.focus())
  }

  // ---- Sending -------------------------------------------------------------------

  async function send() {
    if (!built.ok) return
    setBusy(true)
    setFailure(null)
    // The service drops the working copy when the revision is sent. A save
    // still on its way must not put it back afterwards.
    await saver.settle()
    try {
      const result = await api.sendRevision(
        exchange.id,
        revisionToSend(exchange, built, language, invitationBoundTo(invitee) ?? ''),
      )
      saver.sent()
      await showAfterSigning(exchange.id, alsoShow)
      onSent(result, kind === 'first' ? invitationBoundTo(invitee) : null)
    } catch (error) {
      const code = failureCode(error)
      saver.resume()
      if (code === 'VERSION_CONFLICT') {
        // The exchange moved on. Nothing was sent; show what it is now and
        // keep what was written.
        await reload()
        setConflict(true)
        setStep('edit')
      } else setFailure(code)
      setBusy(false)
    }
  }

  // ---- Discarding a draft never sent ----------------------------------------------

  async function discard() {
    setBusy(true)
    setDiscardFailure(null)
    // The working copy goes with the draft; a save still on its way must not
    // be refused noisily, or put anything back.
    await saver.settle()
    try {
      await api.runCommand(exchange.id, exchange.version, { type: 'DISCARD' })
      saver.sent()
      navigate(paths.home, { replace: true })
    } catch (error) {
      saver.resume()
      setDiscardFailure(failureCode(error))
      setBusy(false)
    }
  }

  const title = kind === 'first' ? w.titleFirst : kind === 'amend' ? w.titleAmend : w.titleCounter

  if (step === 'sign' && built.ok) {
    // What signing this amendment does, from the terms exactly as they go.
    const predicted = inForce
      ? amendmentEffects(inForce.terms, statusesOf(exchange), built.terms.contributions)
      : null
    return (
      <>
        <PageHeading key="sign" step>
          {w.signTitle}
        </PageHeading>
        <p>{w.signIntro}</p>
        {built.note && (
          <section>
            <h2>{w.yourNote}</h2>
            <Written>{built.note}</Written>
            <p className="hint">{w.noteHint}</p>
          </section>
        )}
        <section className="card">
          <TermsView
            terms={built.terms}
            currency={exchange.currency}
            timezone={exchange.timezone}
            you={you}
            level={2}
          />
        </section>
        {predicted && <Effects effects={predicted} />}
        {kind === 'first' && (
          <p>
            {invitee.anyone
              ? wording.invitationLink.forAnyoneSummary
              : fmt(wording.invitationLink.boundSummary, { identifier: invitee.to.trim() })}
          </p>
        )}
        <ShowWhenSigning
          terms={built.terms}
          you={you}
          shown={Boolean(exchange.payment_options?.shown)}
          checked={alsoShow}
          onChange={setAlsoShow}
        />
        <Consent
          signLabel={w.signAndSend}
          busy={busy}
          failure={failure}
          onSign={() => void send()}
          onCancel={() => setStep('edit')}
          cancelLabel={w.backToEdit}
          level={2}
        />
      </>
    )
  }

  // An accepted contribution is locked: an amendment may not touch it.
  const locked = lockedContributions(exchange)
  // Due dates are read in the exchange's zone; named when this device keeps another.
  const zone = dueDateZone(exchange.timezone)
  const nameOf = (slot: Slot) => (slot === 'A' ? draft.partyA : draft.partyB)
  const setName = (slot: Slot, name: string) =>
    change(slot === 'A' ? { partyA: name } : { partyB: name })
  const general = problems.filter((problem) => problem.field === 'contributions')

  return (
    <>
      <EditAnnouncements
        stale={stale ? w.staleDraft : null}
        dropped={dropped.length > 0 ? fmt(w.removedCount, { count: dropped.length }) : null}
        saveFailed={saveState === 'failed' ? w.saveFailed : null}
      />
      {/* Coming back from the signing step, the keyboard starts from the top again. */}
      <PageHeading key="edit" step={returned}>
        {title}
      </PageHeading>
      <p>{kind === 'first' ? w.introFirst : kind === 'amend' ? w.introAmend : w.introCounter}</p>
      {kind === 'amend' && (
        <>
          <p>{w.effectsSteer}</p>
          <HelpLink place="amendment" />
        </>
      )}

      {conflict && <ErrorNote>{w.conflict}</ErrorNote>}
      {stale && base && (
        <div className="notice">
          <p>{w.staleDraft}</p>
          <div className="actions">
            <button type="button" onClick={() => change({ base: base.id })}>
              {w.staleDraftKeep}
            </button>
            <button
              type="button"
              onClick={() => {
                edit(draftFromTerms(base.terms, base.id, digits))
                setGeneration((count) => count + 1)
                setChecked(false)
              }}
            >
              {w.staleDraftDiscard}
            </button>
          </div>
        </div>
      )}
      {problemCount > 0 && <ErrorNote>{fmt(w.problemsSummary, { count: problemCount })}</ErrorNote>}

      <form
        noValidate
        key={generation}
        onSubmit={(event) => {
          event.preventDefault()
          review()
        }}
      >
        <fieldset>
          <legend>{w.partiesLegend}</legend>
          <Field
            label={w.yourName}
            id={`party-${you}`}
            required
            error={errorFor(you === 'A' ? 'partyA' : 'partyB')}
          >
            {(control) => (
              <input
                {...control}
                type="text"
                maxLength={100}
                value={nameOf(you)}
                onChange={(event) => setName(you, event.target.value)}
              />
            )}
          </Field>
          <Field
            label={w.otherName}
            id={`party-${other}`}
            required
            error={errorFor(other === 'A' ? 'partyA' : 'partyB')}
          >
            {(control) => (
              <input
                {...control}
                type="text"
                maxLength={100}
                value={nameOf(other)}
                onChange={(event) => setName(other, event.target.value)}
              />
            )}
          </Field>
          {kind === 'first' && (
            <InvitationFor
              choice={invitee}
              onChange={setInvitee}
              channels={channels}
              id={BOUND_TO}
              error={
                boundProblem
                  ? boundToProblemText(boundProblem, wording.invitationLink, channels, fmt)
                  : null
              }
            />
          )}
        </fieldset>

        <Field label={w.termsLabel} hint={w.termsHint}>
          {(control) => (
            <textarea
              {...control}
              rows={5}
              value={draft.terms}
              onChange={(event) => change({ terms: event.target.value })}
            />
          )}
        </Field>

        <h2>{w.itemsHeading}</h2>
        {draft.contributions.map((item, index) => {
          const number = index + 1
          const minor =
            item.type === 'MONEY' && item.amount ? toMinorUnits(item.amount, digits) : null
          const effect = effectOf(item.id)
          return (
            <fieldset key={item.id} disabled={locked.has(item.id)}>
              <legend>{fmt(w.itemLegend, { number })}</legend>
              {locked.has(item.id) && <p className="notice">{w.locked}</p>}
              {/* What the amendment does to this item, as it is being written. */}
              {effect && !locked.has(item.id) && <EffectNote effect={effect.effect} />}

              <div className="pair">
                <Field label={w.fromLabel}>
                  {(control) => (
                    <select
                      {...control}
                      value={item.from}
                      onChange={(event) =>
                        changeItem(item.id, { from: event.target.value as Slot })
                      }
                    >
                      <option value={you}>{wording.party.you}</option>
                      <option value={other}>{wording.party.other}</option>
                    </select>
                  )}
                </Field>
                <Field label={w.typeLabel}>
                  {(control) => (
                    <select
                      {...control}
                      value={item.type}
                      onChange={(event) =>
                        changeItem(item.id, { type: event.target.value as ContributionType })
                      }
                    >
                      {CONTRIBUTION_TYPES.map((type) => (
                        <option key={type} value={type}>
                          {wording.contributionTypes[type]}
                        </option>
                      ))}
                    </select>
                  )}
                </Field>
              </div>

              <Field
                label={w.descriptionLabel}
                id={`${item.id}-description`}
                required
                error={errorFor('description', item.id)}
              >
                {(control) => (
                  <textarea
                    {...control}
                    rows={2}
                    value={item.description}
                    onChange={(event) => changeItem(item.id, { description: event.target.value })}
                  />
                )}
              </Field>

              {item.type === 'MONEY' ? (
                <Field
                  label={fmt(w.amountLabel, { currency: exchange.currency })}
                  id={`${item.id}-amount`}
                  required
                  error={errorFor('amount', item.id)}
                >
                  {(control) => (
                    <>
                      <DecimalInput
                        {...control}
                        // Read with the field, rather than announced at every keystroke.
                        aria-describedby={[
                          control['aria-describedby'],
                          minor !== null ? `${item.id}-amount-preview` : null,
                          `${item.id}-amount-outside`,
                        ]
                          .filter(Boolean)
                          .join(' ')}
                        value={item.amount}
                        onChange={(amount) => changeItem(item.id, { amount })}
                      />
                      {/* What the typed number will be signed as. */}
                      {minor !== null && (
                        <p className="hint" id={`${item.id}-amount-preview`}>
                          {fmt(w.amountPreview, { amount: money(minor, exchange.currency) })}
                        </p>
                      )}
                      {/* Money is paid outside the product and only recorded here (DESIGN.md §11). */}
                      <p className="hint" id={`${item.id}-amount-outside`}>
                        {w.moneyOutside}
                      </p>
                      <HelpLink place="moneyOutside" />
                    </>
                  )}
                </Field>
              ) : (
                <div className="pair">
                  <Field
                    label={w.quantityLabel}
                    id={`${item.id}-quantity`}
                    error={errorFor('quantity', item.id)}
                  >
                    {(control) => (
                      <DecimalInput
                        {...control}
                        value={item.quantity}
                        onChange={(quantity) => changeItem(item.id, { quantity })}
                      />
                    )}
                  </Field>
                  <Field label={w.unitLabel}>
                    {(control) => (
                      <input
                        {...control}
                        type="text"
                        maxLength={40}
                        value={item.unit}
                        onChange={(event) => changeItem(item.id, { unit: event.target.value })}
                      />
                    )}
                  </Field>
                </div>
              )}

              <Field label={w.dueLabel}>
                {(control) => (
                  <select
                    {...control}
                    value={item.due.kind}
                    onChange={(event) =>
                      changeItem(item.id, { due: dueOf(event.target.value as DraftDue['kind']) })
                    }
                  >
                    <option value="ON_AGREEMENT">{w.dueOnAgreement}</option>
                    <option value="DATE">{w.dueOnDate}</option>
                    <option value="AFTER_CONTRIBUTION">{w.dueAfter}</option>
                  </select>
                )}
              </Field>
              {item.due.kind === 'DATE' && (
                <Field
                  label={w.dateLabel}
                  hint={zone ? fmt(w.dateInZone, { zone: timeZoneCity(zone) }) : undefined}
                  id={`${item.id}-date`}
                  required
                  error={errorFor('date', item.id)}
                >
                  {(control) => (
                    <input
                      {...control}
                      type="date"
                      value={item.due.kind === 'DATE' ? item.due.date : ''}
                      onChange={(event) =>
                        changeItem(item.id, { due: { kind: 'DATE', date: event.target.value } })
                      }
                    />
                  )}
                </Field>
              )}
              {item.due.kind === 'AFTER_CONTRIBUTION' && (
                <Field
                  label={w.afterLabel}
                  id={`${item.id}-after`}
                  required
                  error={errorFor('after', item.id)}
                >
                  {(control) => (
                    <select
                      {...control}
                      value={item.due.kind === 'AFTER_CONTRIBUTION' ? item.due.contribution : ''}
                      onChange={(event) =>
                        changeItem(item.id, {
                          due: { kind: 'AFTER_CONTRIBUTION', contribution: event.target.value },
                        })
                      }
                    >
                      <option value="">{w.afterChoose}</option>
                      {draft.contributions.map((candidate, position) =>
                        candidate.id === item.id ? null : (
                          <option key={candidate.id} value={candidate.id}>
                            {candidate.description.trim()
                              ? fmt(w.itemOption, {
                                  number: position + 1,
                                  description: candidate.description.trim(),
                                })
                              : fmt(w.itemOptionBlank, { number: position + 1 })}
                          </option>
                        ),
                      )}
                    </select>
                  )}
                </Field>
              )}

              <Field label={w.criteriaLabel}>
                {(control) => (
                  <textarea
                    {...control}
                    rows={2}
                    value={item.criteria}
                    onChange={(event) => changeItem(item.id, { criteria: event.target.value })}
                  />
                )}
              </Field>

              <label className="check">
                <input
                  type="checkbox"
                  checked={item.required}
                  onChange={(event) => changeItem(item.id, { required: event.target.checked })}
                />
                <span>{w.requiredLabel}</span>
              </label>

              <div className="actions">
                <button
                  type="button"
                  onClick={() =>
                    change({
                      contributions: latest.current.contributions.filter(
                        (candidate) => candidate.id !== item.id,
                      ),
                    })
                  }
                >
                  {fmt(w.remove, { number })}
                </button>
              </div>
            </fieldset>
          )
        })}

        {/* Items of the agreement this change removes, named as the agreement wrote them. */}
        {dropped.length > 0 && (
          <div className="notice">
            <p>{fmt(w.removedCount, { count: dropped.length })}</p>
            <ul className="plain">
              {dropped.map((item) => (
                <li key={item.id}>
                  <Written inline>{item.description}</Written>
                  {' — '}
                  {w.effects[item.effect]}
                </li>
              ))}
            </ul>
          </div>
        )}

        {general.map((problem) => (
          <p key={problem.code} className="field-error" id="items-error">
            {problemText(problem)}
          </p>
        ))}
        <div className="actions">
          <button
            type="button"
            id="add-yours"
            aria-describedby={general.length > 0 ? 'items-error' : undefined}
            onClick={() => addItem(you)}
          >
            {w.addYours}
          </button>
          <button type="button" onClick={() => addItem(other)}>
            {w.addTheirs}
          </button>
        </div>

        <Field label={w.noteLabel} hint={w.noteHint} id="note" error={errorFor('note')}>
          {(control) => (
            <textarea
              {...control}
              rows={3}
              value={draft.note}
              onChange={(event) => change({ note: event.target.value })}
            />
          )}
        </Field>

        <p className="hint">
          {saveState === 'saving' && w.saving}
          {saveState === 'saved' && w.saved}
          {saveState === 'failed' && w.saveFailed}
        </p>

        <div className="actions">
          <button type="submit" className="primary" disabled={stale || busy}>
            {w.review}
          </button>
          <Link className="button" to={kind === 'first' ? paths.home : paths.exchange(exchange.id)}>
            {kind === 'first' ? wording.nav.exchanges : wording.common.cancel}
          </Link>
          {/* A draft never sent can be thrown away; afterwards it is closed and out of the list. */}
          {kind === 'first' && (
            <button
              type="button"
              aria-expanded={discarding}
              disabled={busy}
              onClick={() => setDiscarding(true)}
            >
              {w.discard}
            </button>
          )}
        </div>
        {discarding && (
          <Panel title={w.discard}>
            <p>{w.discardText}</p>
            <Failure code={discardFailure} />
            <div className="actions">
              <button
                type="button"
                className="primary"
                disabled={busy}
                onClick={() => void discard()}
              >
                {w.confirmDiscard}
              </button>
              <button type="button" disabled={busy} onClick={() => setDiscarding(false)}>
                {wording.common.cancel}
              </button>
            </div>
          </Panel>
        )}
      </form>
    </>
  )
}

/**
 * Changes on the editing page that are seen rather than reached: the working
 * copy turning out to be older than the terms, items of the agreement
 * dropping out of it, and a save failing. Saving and saved are shown but not
 * said, or they would be said every few seconds while someone types.
 */
function EditAnnouncements(props: {
  stale: string | null
  dropped: string | null
  saveFailed: string | null
}) {
  useAnnouncement(props.stale)
  useAnnouncement(props.dropped)
  useAnnouncement(props.saveFailed)
  return null
}

/** What the amendment does to one item, said as it changes while the item is edited. */
function EffectNote({ effect }: { effect: ItemEffect['effect'] }) {
  const { wording } = useI18n()
  const text = wording.composer.effects[effect]
  const refused = effect === 'LOCKED' || effect === 'REUSED'
  // Polite even when refused: it is said while the person is typing, and
  // nothing is lost by finishing the word first.
  useAnnouncement(text)
  return <p className={refused ? 'notice notice-error' : 'hint'}>{text}</p>
}

/**
 * What an amendment does to each item of the agreement once both have signed
 * it, from the same rule the service applies (DESIGN.md §7): untouched items
 * keep their status, a changed one goes back to the start, a removed one
 * leaves, a new one starts, and a confirmed one cannot be touched.
 */
function Effects({ effects }: { effects: readonly ItemEffect[] }) {
  const { wording } = useI18n()
  const w = wording.composer
  return (
    <section className="card" aria-labelledby="effects-heading">
      <h2 id="effects-heading">{w.effectsHeading}</h2>
      <p>{w.effectsIntro}</p>
      <ul className="plain">
        {effects.map((item) => (
          <li key={item.id} className="contribution">
            <Written>{item.description}</Written>
            <p
              className={
                item.effect === 'LOCKED' || item.effect === 'REUSED' ? 'notice notice-error' : ''
              }
            >
              {w.effects[item.effect]}
            </p>
          </li>
        ))}
      </ul>
      <p className="hint">{w.effectsSteer}</p>
    </section>
  )
}

interface DecimalInputProps extends ControlProps {
  /** A plain decimal, empty for none, `null` when what is typed is not a number. */
  value: string | null
  onChange(value: string | null): void
}

/**
 * A number typed the way the reader's language writes numbers. What is typed
 * stays on screen as typed; the working copy gets the plain form, or `null`
 * while it cannot be read as a number.
 */
function DecimalInput({ value, onChange, ...control }: DecimalInputProps) {
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
