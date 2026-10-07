import type { ErrorCode, ExchangeView as Exchange } from '@yuppers/api-client';
import {
  boundToProblem,
  boundToProblemText,
  consentShown,
  failureCode,
  isUnconfirmedClaimant,
  labelText,
  moneyIds,
  otherPartyName,
  remainingRequired,
  statusesOf,
  troublePanel,
  troubleSituationOf,
  useActions,
  useHistory,
  useSignInChannels,
  type Actions as ExchangeActions,
  type ClosedReason,
  type RevisionView,
} from '@yuppers/shared';
import { useIsFocused, useRouter } from 'expo-router';
import { useEffect, useMemo, useRef, useState } from 'react';
import { AppState, StyleSheet, Text, type ScrollView } from 'react-native';

import { AgreedHero } from '../components/Callouts';
import { Consent } from '../components/Consent';
import { InvitationFor, InvitationLink } from '../components/InvitationLink';
import { ContentHidden, OtherPartyLeft } from '../components/OtherPartyLeft';
import { TermsView } from '../components/TermsView';
import { SmsUpdates } from '../components/SmsUpdates';
import { WalletButton } from '../components/WalletButton';
import {
  Actions,
  Button,
  Card,
  Failure,
  Heading,
  Hint,
  Lines,
  Notice,
  P,
  Panel,
  Screen,
  Tag,
  Tags,
  Written,
} from '../components/ui';
import { focusKeeper, useReduceMotion } from '../lib/accessibility';
import { useI18n } from '../lib/context';
import { api } from '../lib/session';
import { type, useColors } from '../lib/theme';
import { ClaimantWaiting, ConfirmClaimant, NobodyYet } from './Claimant';
import { Ending } from './Ending';
import { ExchangeSafety } from './ExchangeSafety';
import { Fulfillment } from './Fulfillment';
import { History } from './History';
import { ProposalChanges } from './ProposalChanges';
import { Trouble } from './Trouble';

/** How often an open exchange is checked for what the other party has done. */
const CHECK_EVERY_MS = 20_000;

interface Props {
  exchange: Exchange;
  /** A just-issued invitation token, to show once. */
  issued: string | null;
  onIssued(token: string | null): void;
  onChange(exchange: Exchange): void;
  reload(): Promise<Exchange | null>;
}

/**
 * An exchange as one of its two parties sees it: its state, the revision
 * waiting to be signed, the agreement in force and where each contribution
 * stands, and every action open to this party right now. The service decides
 * what is allowed; this offers what should be, and shows the refusal if it
 * was wrong.
 */
export function ExchangeView({ exchange, issued, onIssued, onChange, reload }: Props) {
  const { wording, fmt } = useI18n();
  const router = useRouter();
  const w = wording.exchange;
  // A panel cancelled gives the screen reader's focus back to what opened it.
  const actions = useActions(api, exchange, onChange, reload, focusKeeper);
  const reduceMotion = useReduceMotion();

  const you = exchange.you;
  const open = exchange.open_revision ?? null;
  const inForce = exchange.in_force_revision ?? null;
  // Read here rather than in the history section: an exchange closed without
  // agreement has no revision to read names from, and the history names the
  // parties as the last terms did.
  const history = useHistory(api, exchange);
  const writtenName = otherPartyName(exchange, history.page?.parties);
  // In a sentence, someone with no name yet is "the other party".
  const otherName = writtenName || wording.party.other;
  const active = exchange.state === 'ACTIVE';
  // Money is spoken of in words for paying and receiving (DESIGN.md §11).
  const money = useMemo(() => moneyIds([open?.terms, inForce?.terms]), [open, inForce]);
  const revise = () => router.push(`/exchanges/${exchange.id}/revise`);

  // A newer version found while the person is in the middle of something is
  // held back and offered, not swapped in under them.
  const [newer, setNewer] = useState<Exchange | null>(null);
  const [refreshed, setRefreshed] = useState(false);
  const [refreshing, setRefreshing] = useState(false);
  const current = useRef({ exchange, engaged: false });
  useEffect(() => {
    current.current = { exchange, engaged: actions.panel !== null || actions.busy };
  });

  // While this screen is the one in front and the app is open, look now and
  // then for what the other party has done. Every change needs the service,
  // so a stale screen is only ever a refusal away from being corrected.
  const closed = exchange.state === 'CLOSED';
  const exchangeId = exchange.id;
  const focused = useIsFocused();
  useEffect(() => {
    if (closed || !focused) return;
    let cancelled = false;
    async function check() {
      if (AppState.currentState !== 'active' || current.current.engaged) return;
      let found: Exchange;
      try {
        found = await api.getExchange(exchangeId);
      } catch {
        return;
      }
      if (cancelled || found.version === current.current.exchange.version) return;
      if (current.current.engaged) setNewer(found);
      else {
        onChange(found);
        setRefreshed(true);
      }
    }
    const timer = setInterval(() => void check(), CHECK_EVERY_MS);
    const subscription = AppState.addEventListener('change', (state) => {
      if (state === 'active') void check();
    });
    return () => {
      cancelled = true;
      clearInterval(timer);
      subscription.remove();
    };
  }, [exchangeId, closed, focused, onChange]);

  // A refusal with no panel left to show it in, such as one that reloaded the
  // exchange, is shown at the top, and the top is brought into view: the
  // person may be far down the screen, looking at what they just pressed.
  const refused = actions.panel === null ? actions.failure : null;
  const scroll = useRef<ScrollView>(null);
  useEffect(() => {
    if (refused) scroll.current?.scrollTo({ y: 0, animated: !reduceMotion });
  }, [refused, reduceMotion]);

  const statuses = statusesOf(exchange);
  const since = new Map(exchange.contributions.map((item) => [item.id, item.since ?? null]));
  const remaining = remainingRequired(exchange);

  return (
    <Screen
      scroll={scroll}
      refreshing={refreshing}
      onRefresh={() => {
        setRefreshing(true);
        setRefreshed(false);
        void reload().finally(() => setRefreshing(false));
      }}>
      {/* A screen reader says the heading first. Nothing in the name may
          turn the words around it or break the line. */}
      <Heading>
        {writtenName ? fmt(w.title, { name: labelText(writtenName) }) : w.titleNoName}
      </Heading>
      <Tags>
        <Tag>
          {exchange.closed_outcome
            ? wording.outcomes[exchange.closed_outcome]
            : wording.states[exchange.state]}
        </Tag>
        <Tag>{fmt(wording.home.reference, { code: exchange.display_code })}</Tag>
      </Tags>
      {exchange.closed_reason && Object.hasOwn(wording.closedReasons, exchange.closed_reason) ? (
        <P>{wording.closedReasons[exchange.closed_reason as ClosedReason]}</P>
      ) : null}

      <Failure code={refused} />
      {(actions.done || refreshed) && !newer && <Notice>{w.updated}</Notice>}
      {newer && (
        <Notice>
          <P>{w.newer}</P>
          <Actions>
            <Button
              label={w.showLatest}
              onPress={() => {
                actions.close();
                onChange(newer);
                setNewer(null);
              }}
            />
          </Actions>
        </Notice>
      )}

      <OtherPartyLeft exchange={exchange} otherName={otherName} />
      <ContentHidden exchange={exchange} />
      <Counterparty
        exchange={exchange}
        otherName={otherName}
        actions={actions}
        issued={issued}
        onIssued={onIssued}
        reload={reload}
      />

      {open && (
        <OpenRevision
          exchange={exchange}
          revision={open}
          otherName={otherName}
          actions={actions}
          onRevise={revise}
        />
      )}

      {inForce && (
        <Card>
          <Heading level={2}>{w.agreementHeading}</Heading>
          <AgreedHero>{w.agreementSigned}</AgreedHero>
          {active && remaining > 0 && <P>{fmt(w.remaining, { count: remaining })}</P>}
          {active && <WalletButton exchange={exchange} />}
          <TermsView
            terms={inForce.terms}
            currency={exchange.currency}
            timezone={exchange.timezone}
            you={you}
            statuses={statuses}
            footer={(contribution) => (
              <Fulfillment
                contribution={contribution}
                status={statuses.get(contribution.id) ?? 'PENDING'}
                since={since.get(contribution.id) ?? null}
                you={you}
                otherName={otherName}
                active={active}
                actions={actions}
              />
            )}
          />
          <Fingerprint hash={inForce.content_hash} />
          {active && (
            <Actions>
              {!open && <Button label={w.amend} onPress={revise} />}
              {/* One way in to the ways out (DESIGN.md §5.3). */}
              <Button
                label={wording.trouble.open}
                expanded={troubleSituationOf(actions.panel) !== undefined}
                disabled={actions.busy}
                onPress={() => actions.open(troublePanel())}
              />
            </Actions>
          )}
          {active && troubleSituationOf(actions.panel) !== undefined && (
            <Trouble
              key={actions.panel}
              exchange={exchange}
              otherName={otherName}
              actions={actions}
              onRevise={revise}
            />
          )}
        </Card>
      )}

      {active && <Ending exchange={exchange} otherName={otherName} actions={actions} />}

      <SmsUpdates exchange={exchange} />

      <History exchange={exchange} reading={history} money={money} />

      <ExchangeSafety exchange={exchange} otherName={otherName} actions={actions} reload={reload} />

      <Actions>
        {!closed && (
          <Button
            variant="link"
            label={w.refresh}
            onPress={() => {
              setRefreshed(false);
              void reload();
            }}
          />
        )}
        {/* Opened from a link, there is no screen underneath to go back to. */}
        {!router.canGoBack() && (
          <Button
            variant="link"
            label={wording.common.goHome}
            onPress={() => router.replace('/')}
          />
        )}
      </Actions>
    </Screen>
  );
}

/** The hash of the signed terms, which a signature is bound to (DESIGN.md §6). */
function Fingerprint({ hash }: { hash: string }) {
  const { wording, fmt } = useI18n();
  const colors = useColors();
  return (
    <Text selectable style={[type.hint, styles.fingerprint, { color: colors.muted }]}>
      {fmt(wording.terms.fingerprint, { hash })}
    </Text>
  );
}

interface CounterpartyProps {
  exchange: Exchange;
  otherName: string;
  actions: ExchangeActions;
  issued: string | null;
  onIssued(token: string | null): void;
  reload(): Promise<Exchange | null>;
}

/**
 * Who is on the other side (DESIGN.md §8). Until someone opens the link the
 * initiator can replace it; once someone has, the initiator confirms it is
 * who they meant before any signature takes effect.
 */
function Counterparty({
  exchange,
  otherName,
  actions,
  issued,
  onIssued,
  reload,
}: CounterpartyProps) {
  const { wording } = useI18n();
  const link = wording.invitationLink;
  const initiator = exchange.you === 'A';
  const claimant = exchange.claimant ?? null;

  if (exchange.state !== 'NEGOTIATING') return null;

  if (initiator && exchange.counterparty === 'UNCLAIMED') {
    return (
      <Card>
        <Heading level={2}>{link.heading}</Heading>
        {issued ? <InvitationLink key={issued} token={issued} /> : <NobodyYet exchange={exchange} />}
        <P>{link.reissueIntro}</P>
        <Actions>
          <Button
            label={link.reissue}
            expanded={actions.panel === 'reissue'}
            onPress={() => actions.open('reissue')}
          />
        </Actions>
        {actions.panel === 'reissue' && (
          <Reissue exchange={exchange.id} actions={actions} onIssued={onIssued} reload={reload} />
        )}
      </Card>
    );
  }

  if (initiator && exchange.counterparty === 'CLAIMED' && claimant) {
    return (
      <ConfirmClaimant
        exchange={exchange}
        claimant={claimant}
        actions={actions}
        // Straight on to making a link for the person who was meant.
        onRejected={() => actions.open('reissue')}
      />
    );
  }

  if (isUnconfirmedClaimant(exchange)) {
    return <ClaimantWaiting exchange={exchange} otherName={otherName} actions={actions} />;
  }
  return null;
}

interface ReissueProps {
  exchange: string;
  actions: ExchangeActions;
  onIssued(token: string | null): void;
  reload(): Promise<Exchange | null>;
}

function Reissue({ exchange, actions, onIssued, reload }: ReissueProps) {
  const { wording, fmt } = useI18n();
  const link = wording.invitationLink;
  const [boundTo, setBoundTo] = useState('');
  const [busy, setBusy] = useState(false);
  const [failure, setFailure] = useState<ErrorCode | null>(null);
  const [checked, setChecked] = useState(false);
  const channels = useSignInChannels(api);
  const problem = checked ? boundToProblem(boundTo, channels) : null;

  async function submit() {
    setChecked(true);
    if (boundToProblem(boundTo, channels)) return;
    setBusy(true);
    setFailure(null);
    try {
      onIssued(await api.reissueInvitation(exchange, boundTo.trim() || null));
      actions.close();
    } catch (error) {
      const code = failureCode(error);
      setFailure(code);
      // Refused because someone has opened the link in the meantime.
      if (code === 'ACTION_NOT_ALLOWED') await reload();
    } finally {
      setBusy(false);
    }
  }

  return (
    <Panel title={link.reissue}>
      <InvitationFor
        value={boundTo}
        onChange={setBoundTo}
        channels={channels}
        error={problem ? boundToProblemText(problem, link, channels, fmt) : null}
      />
      <Failure code={failure} />
      <Actions>
        <Button
          variant="primary"
          label={link.reissue}
          disabled={busy}
          onPress={() => void submit()}
        />
        <Button label={wording.common.cancel} disabled={busy} onPress={actions.close} />
      </Actions>
    </Panel>
  );
}

interface OpenRevisionProps {
  exchange: Exchange;
  revision: RevisionView;
  otherName: string;
  actions: ExchangeActions;
  onRevise(): void;
}

/**
 * The one revision waiting to be signed: a first proposal, a counteroffer,
 * or an amendment to the agreement in force. Its author signed it by sending
 * it; the other party can sign it, decline it, or answer with their own.
 */
function OpenRevision({ exchange, revision, otherName, actions, onRevise }: OpenRevisionProps) {
  const { wording, fmt, moment, language } = useI18n();
  const w = wording.exchange;
  const you = exchange.you;
  const yours = revision.author === you;
  const youSigned = revision.accepted_by.includes(you);
  const otherSigned = revision.accepted_by.some((slot) => slot !== you);
  const amendment = exchange.state === 'ACTIVE';
  // The initiator is never bound to someone they have not confirmed.
  const blocked = you === 'A' && exchange.counterparty !== 'CONFIRMED';

  return (
    <Card>
      <Heading level={2}>{amendment ? w.amendmentHeading : w.proposalHeading}</Heading>
      <Lines>
        <Hint>{fmt(w.version, { number: revision.sequence })}</Hint>
        <Hint>{yours ? w.sentByYou : fmt(w.sentByOther, { name: otherName })}</Hint>
        <Hint>{fmt(w.expires, { date: moment(revision.expires_at) })}</Hint>
      </Lines>

      {revision.note ? (
        <>
          <Heading level={3}>
            {yours ? w.noteFromYou : fmt(w.noteFromOther, { name: otherName })}
          </Heading>
          <Written>{revision.note}</Written>
        </>
      ) : null}

      {/* The complete terms come before any way to sign them (DESIGN.md §14.1). */}
      <TermsView
        terms={revision.terms}
        currency={exchange.currency}
        timezone={exchange.timezone}
        you={you}
      />
      <Fingerprint hash={revision.content_hash} />

      {/* What it changes, for the person asked to sign it: the author saw this while writing it. */}
      {!yours && <ProposalChanges exchange={exchange} revision={revision} />}

      <Lines>
        <P>{youSigned ? w.signedByYou : w.unsignedByYou}</P>
        <P>
          {otherSigned
            ? fmt(w.signedByOther, { name: otherName })
            : fmt(w.unsignedByOther, { name: otherName })}
        </P>
      </Lines>

      {yours && (
        <Actions>
          <Button label={w.change} onPress={onRevise} />
          <Button
            label={w.withdraw}
            expanded={actions.panel === 'withdraw'}
            disabled={actions.busy}
            onPress={() => actions.open('withdraw')}
          />
        </Actions>
      )}

      {!yours && !youSigned && (
        <>
          {blocked && <Notice quiet>{w.acceptBlocked}</Notice>}
          <Actions>
            {!blocked && (
              <Button
                variant="primary"
                label={w.accept}
                expanded={actions.panel === 'accept'}
                disabled={actions.busy}
                onPress={() => actions.open('accept')}
              />
            )}
            {/* Someone the initiator has not confirmed can sign or leave (DESIGN.md §8). */}
            {!isUnconfirmedClaimant(exchange) && (
              <>
                <Button label={w.counter} onPress={onRevise} />
                <Button
                  label={w.decline}
                  expanded={actions.panel === 'decline'}
                  disabled={actions.busy}
                  onPress={() => actions.open('decline')}
                />
              </>
            )}
          </Actions>
        </>
      )}

      {actions.panel === 'accept' && (
        <Panel title={w.signHeading}>
          <P>{w.signIntro}</P>
          <Consent
            headingLevel={4}
            signLabel={w.accept}
            busy={actions.busy}
            failure={actions.failure}
            onCancel={actions.close}
            onSign={() =>
              // Acceptance names the revision: if the terms have changed since
              // this screen showed them, the service refuses (DESIGN.md §6).
              void actions.run({
                type: 'ACCEPT',
                revision: revision.id,
                consent: consentShown(language),
              })
            }
          />
        </Panel>
      )}

      {actions.panel === 'decline' && (
        <Panel title={w.decline}>
          <P>{amendment ? w.declineKeeps : w.declineEnds}</P>
          <Failure code={actions.failure} />
          <Actions>
            <Button
              variant="primary"
              label={w.confirmDecline}
              disabled={actions.busy}
              onPress={() => void actions.run({ type: 'DECLINE', revision: revision.id })}
            />
            <Button label={wording.common.cancel} disabled={actions.busy} onPress={actions.close} />
          </Actions>
        </Panel>
      )}

      {actions.panel === 'withdraw' && (
        <Panel title={w.withdraw}>
          <P>{amendment ? w.withdrawKeeps : w.withdrawEnds}</P>
          <Failure code={actions.failure} />
          <Actions>
            <Button
              variant="primary"
              label={w.confirmWithdraw}
              disabled={actions.busy}
              onPress={() => void actions.run({ type: 'WITHDRAW', revision: revision.id })}
            />
            <Button label={wording.common.cancel} disabled={actions.busy} onPress={actions.close} />
          </Actions>
        </Panel>
      )}
    </Card>
  );
}

const styles = StyleSheet.create({
  fingerprint: { fontVariant: ['tabular-nums'] },
});
