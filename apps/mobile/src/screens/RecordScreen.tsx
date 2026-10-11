import type { ErrorCode } from '@yuppers/api-client';
import {
  documentOf,
  moneyIds,
  recordFile,
  recordMoments,
  statusWording,
  termsOfRevision,
  useRecord,
  verificationText,
  type ClosedReason,
  type RecordDocument,
  type RecordRevision,
  type Slot,
} from '@yuppers/shared';
import { useRouter } from 'expo-router';
import { useMemo, useState, type ReactNode } from 'react';
import { StyleSheet, Text, View } from 'react-native';

import { EventList } from '../components/EventList';
import { HelpLink } from '../components/HelpLink';
import { RecordSummary } from '../components/RecordSummary';
import { StatusChip } from '../components/StatusChip';
import { TermsView } from '../components/TermsView';
import {
  Actions,
  Button,
  Card,
  ErrorNote,
  Failure,
  Heading,
  Hint,
  Lines,
  P,
  Screen,
  Tag,
  Tags,
  Written,
} from '../components/ui';
import { useI18n } from '../lib/context';
import { recordHtml } from '../lib/record-html';
import { recordSharer } from '../lib/record-sharer';
import { api } from '../lib/session';
import { fonts, space, type, useColors } from '../lib/theme';
import { VoidSignature } from './Claimant';

/**
 * The record of an exchange: how it stands, every version sent and who signed
 * it, and everything that happened, laid out to be read from top to bottom,
 * under a plain summary of it. The copy a party takes away is a PDF of the
 * same, or the record as a file, handed to the system's share sheet
 * (DESIGN.md §14.1).
 */
export function RecordScreen({ id }: { id: string }) {
  const { wording } = useI18n();
  const router = useRouter();
  // A long record comes in parts; the screen shows it whole.
  const { record, failure, reload } = useRecord(api, id);

  if (record) return <Record record={record} failure={failure} reload={reload} />;
  if (!failure) {
    return (
      <Screen>
        <P>{wording.common.loading}</P>
      </Screen>
    );
  }
  return (
    <Screen>
      <Heading>{wording.exchange.titleNoName}</Heading>
      <Failure code={failure} />
      <Actions>
        <Button label={wording.common.goHome} onPress={() => router.dismissTo('/')} />
      </Actions>
    </Screen>
  );
}

interface RecordProps {
  record: RecordDocument;
  /** Why reading it again failed; what was read before stays on screen. */
  failure: ErrorCode | null;
  reload(): Promise<void>;
}

function Record({ record, failure, reload }: RecordProps) {
  const i18n = useI18n();
  const { wording, fmt, language } = i18n;
  const router = useRouter();
  const w = wording.record;
  const mobile = wording.mobile.record;
  const { exchange, parties } = record;
  const when = useMemo(() => recordMoments(language, exchange.timezone), [language, exchange.timezone]);
  const code = exchange.display_code;
  const reason = exchange.closed_reason;
  // Money is spoken of in words for paying and receiving (DESIGN.md §11).
  const money = useMemo(() => moneyIds(record.revisions.map(termsOfRevision)), [record]);

  const [refreshing, setRefreshing] = useState(false);
  const [sharing, setSharing] = useState(false);
  const [problem, setProblem] = useState<'unavailable' | 'failed' | 'pdfFailed' | null>(null);

  // The record laid out as a page, summary first, printed by the device to a
  // PDF and handed to the share sheet, where it can be saved or sent.
  async function savePdf() {
    setSharing(true);
    setProblem(null);
    try {
      const outcome = await recordSharer.sharePdf(
        recordHtml(record, i18n),
        fmt(w.fileName, { code }),
        fmt(w.title, { code }),
      );
      if (outcome === 'unavailable') setProblem('unavailable');
    } catch {
      setProblem('pdfFailed');
    } finally {
      setSharing(false);
    }
  }

  async function share() {
    setSharing(true);
    setProblem(null);
    try {
      const outcome = await recordSharer.share(
        recordFile(record, fmt(w.fileName, { code })),
        fmt(w.title, { code }),
      );
      if (outcome === 'unavailable') setProblem('unavailable');
    } catch {
      setProblem('failed');
    } finally {
      setSharing(false);
    }
  }

  // Back to the exchange: the screen underneath, when this was opened from it.
  const back = () => {
    if (router.canGoBack()) router.back();
    else router.replace(`/exchanges/${exchange.id}`);
  };

  return (
    // Everyone is named here, the reader included: the copy may be handed to
    // someone who is neither party.
    <Screen
      refreshing={refreshing}
      onRefresh={() => {
        setRefreshing(true);
        void reload().finally(() => setRefreshing(false));
      }}>
      <Heading>{fmt(w.title, { code })}</Heading>
      <Failure code={failure} />
      <Lines>
        <Hint>
          {fmt(w.madeFor, { name: parties[record.prepared_for], date: when(record.generated_at) })}
        </Hint>
        <Hint>{fmt(w.timesIn, { timezone: exchange.timezone })}</Hint>
      </Lines>
      <Actions>
        <Button
          variant="primary"
          label={mobile.savePdf}
          disabled={sharing}
          onPress={() => void savePdf()}
        />
        <Button label={mobile.share} disabled={sharing} onPress={() => void share()} />
      </Actions>
      <Hint>{mobile.savePdfHint}</Hint>
      <Hint>{mobile.shareHint}</Hint>
      <HelpLink place="record" />
      {problem === 'unavailable' && <ErrorNote>{mobile.shareUnavailable}</ErrorNote>}
      {problem === 'failed' && <ErrorNote>{mobile.shareFailed}</ErrorNote>}
      {problem === 'pdfFailed' && <ErrorNote>{mobile.pdfFailed}</ErrorNote>}

      <RecordSummary record={record} />

      <Heading level={2}>{w.summaryHeading}</Heading>
      <Tags>
        <Tag>
          {exchange.closed_outcome
            ? wording.outcomes[exchange.closed_outcome]
            : wording.states[exchange.state]}
        </Tag>
        <Tag>{fmt(wording.home.reference, { code })}</Tag>
      </Tags>
      <Heading level={3}>{wording.terms.partiesHeading}</Heading>
      {(['A', 'B'] as const).map((slot) =>
        parties[slot] ? <Written key={slot}>{parties[slot]}</Written> : null,
      )}
      {reason && Object.hasOwn(w.closedReasons, reason) ? (
        <P>{w.closedReasons[reason as ClosedReason]}</P>
      ) : null}
      <Lines>
        <P>{fmt(w.started, { date: when(exchange.created_at) })}</P>
        {exchange.closed_at ? <P>{fmt(w.closedOn, { date: when(exchange.closed_at) })}</P> : null}
      </Lines>
      <P>
        {exchange.in_force_revision
          ? fmt(w.agreementIs, { number: exchange.in_force_revision.sequence })
          : w.agreementNone}
      </P>
      {exchange.open_revision ? (
        <P>{fmt(w.waiting, { number: exchange.open_revision.sequence })}</P>
      ) : null}
      {exchange.end_proposed_by ? (
        <P>{fmt(w.endProposed, { name: parties[exchange.end_proposed_by] })}</P>
      ) : null}
      {exchange.close_requested_by && exchange.close_requested_at ? (
        <P>
          {fmt(w.closeRequested, {
            name: parties[exchange.close_requested_by],
            date: when(exchange.close_requested_at),
          })}
        </P>
      ) : null}

      <Heading level={2}>{w.aboutHeading}</Heading>
      <P>{w.export.about}</P>
      <P>{w.export.signatures}</P>
      <P>{w.export.statements}</P>
      <P>{w.export.contentHash}</P>

      {record.contributions.length > 0 && (
        <>
          <Heading level={2}>{w.itemsHeading}</Heading>
          <View role="list" style={styles.list}>
            {record.contributions.map((contribution) => (
              // One stop for a screen reader: what it is, whose, and where it stands.
              <Item key={contribution.id}>
                <Written>{contribution.description}</Written>
                <P>{fmt(w.itemFrom, { name: parties[contribution.from] })}</P>
                <StatusChip status={contribution.status}>
                  {statusWording(wording, contribution.status, money.has(contribution.id))}
                </StatusChip>
                {contribution.since ? (
                  <Hint>{fmt(w.since, { date: when(contribution.since) })}</Hint>
                ) : null}
              </Item>
            ))}
          </View>
        </>
      )}

      {record.revisions.length === 0 && <P>{w.versionsNone}</P>}
      {record.revisions.map((revision) => (
        <Version
          key={revision.id}
          revision={revision}
          name={(slot) => documentOf(revision).parties[slot]}
          when={when}
        />
      ))}

      <Heading level={2}>{w.eventsHeading}</Heading>
      {record.events.length === 0 ? (
        <P>{w.historyEmpty}</P>
      ) : (
        <EventList
          events={record.events}
          parties={parties}
          reader={null}
          when={when}
          money={money}
        />
      )}

      {record.history_chain ? <Hint>{record.history_chain}</Hint> : null}

      <Actions>
        <Button variant="link" label={w.back} onPress={back} />
      </Actions>
    </Screen>
  );
}

/** One entry of a list, read out whole. */
function Item({ children }: { children: ReactNode }) {
  const colors = useColors();
  return (
    <View role="listitem" accessible style={[styles.item, { borderTopColor: colors.divider }]}>
      {children}
    </View>
  );
}

interface VersionProps {
  revision: RecordRevision;
  /** A party's name as this version writes it. */
  name(slot: Slot): string;
  when(instant: string): string;
}

/**
 * One version that was sent: who sent it and what became of it, its complete
 * terms, the fingerprint a signature covers, and each signature with what it
 * rests on.
 */
function Version({ revision, name, when }: VersionProps) {
  const { wording, fmt } = useI18n();
  const colors = useColors();
  const w = wording.record;
  const { standing } = revision;
  const signed = documentOf(revision);

  return (
    <Card>
      <Heading level={2}>{fmt(w.versionHeading, { number: revision.sequence })}</Heading>
      <Lines>
        <P>
          {fmt(w.versionSent, {
            name: name(revision.author),
            date: when(revision.sent_at),
            expires: when(revision.expires_at),
          })}
        </P>
        {revision.answers ? (
          <P>{fmt(w.versionAnswers, { number: revision.answers.sequence })}</P>
        ) : null}
      </Lines>
      <P style={styles.status}>{w.versionStatus[standing.status]}</P>
      <Lines>
        <Hint>{fmt(w.since, { date: when(standing.since) })}</Hint>
        {standing.replaced_by ? (
          <Hint>{fmt(w.versionReplacedBy, { number: standing.replaced_by.sequence })}</Hint>
        ) : null}
      </Lines>

      {revision.note ? (
        <>
          <Heading level={3}>{w.noteLabels.message}</Heading>
          <Written>{revision.note}</Written>
        </>
      ) : null}

      <TermsView
        terms={termsOfRevision(revision)}
        currency={signed.currency}
        timezone={signed.timezone}
        you={null}
      />
      {/* Selectable, so it can be copied and compared with one computed elsewhere. */}
      <Text selectable style={[type.hint, styles.fingerprint, { color: colors.muted }]}>
        {fmt(wording.terms.fingerprint, { hash: revision.content_hash })}
      </Text>

      <Heading level={3}>{w.signaturesHeading}</Heading>
      <View role="list" style={styles.list}>
        {revision.signatures.map((signature) => (
          // One stop for a screen reader: who signed, then what the signature rests on.
          <Item key={signature.party}>
            <P>
              {fmt(w.versionSignedBy, { name: signature.name, date: when(signature.signed_at) })}
            </P>
            <Hint>
              {fmt(w.verifiedBy, {
                method: verificationText(signature.verification, w.export.verification),
              })}
            </Hint>
            <Hint>{fmt(w.verifiedAt, { date: when(signature.verification.verified_at) })}</Hint>
            {signature.verification.attribution ? <Hint>{signature.verification.attribution}</Hint> : null}
            <Hint>
              {fmt(w.consentShown, {
                version: signature.consent.version,
                language: signature.consent.language,
              })}
            </Hint>
          </Item>
        ))}
        {/* Left by someone removed before being confirmed: kept, and counting for nothing. */}
        {revision.void_signatures?.map((signature) => (
          <Item key={`void-${signature.signed_at}`}>
            <VoidSignature signature={signature} when={when} />
          </Item>
        ))}
      </View>
    </Card>
  );
}

const styles = StyleSheet.create({
  list: { gap: space.m },
  item: { borderTopWidth: StyleSheet.hairlineWidth, paddingTop: space.m, gap: space.xs },
  status: { fontFamily: fonts.textBold },
  fingerprint: { fontVariant: ['tabular-nums'] },
});
