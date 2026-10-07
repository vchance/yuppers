import type { ExchangeView } from '@yuppers/api-client';
import {
  dueDateZone,
  fieldChangeText,
  proposalChanges,
  statusWording,
  useProposalBase,
  type RevisionView,
} from '@yuppers/shared';
import { useMemo } from 'react';
import { StyleSheet, View } from 'react-native';

import { Heading, Hint, Label, P, Tag, Tags, Written } from '../components/ui';
import { useI18n } from '../lib/context';
import { deviceTimezone } from '../lib/time-zone';
import { api } from '../lib/session';
import { fonts, space, useColors } from '../lib/theme';

interface Props {
  exchange: ExchangeView;
  /** The proposal waiting for the reader's signature. */
  revision: RevisionView;
}

/**
 * What a proposal changes, shown to the person asked to sign it, above the
 * way to sign: each item added, removed or changed field by field, against
 * the agreement in force for an amendment (with where each item would then
 * stand), or against the version a counteroffer answers. The complete terms
 * above remain what is signed.
 */
export function ProposalChanges({ exchange, revision }: Props) {
  const i18n = useI18n();
  const { wording, fmt } = i18n;
  const colors = useColors();
  const w = wording.proposalChanges;
  const base = useProposalBase(api, exchange);
  const changes = useMemo(
    () => (base ? proposalChanges(base.terms, revision.terms, base.statuses) : null),
    [base, revision.terms],
  );
  if (!base || !changes) return null;

  const amendment = base.against === 'IN_FORCE';
  const terms = { before: base.terms, after: revision.terms };
  // Named beside a due date when the reader's device keeps another zone.
  const zone = dueDateZone(exchange.timezone, deviceTimezone());
  // For an amendment every item is shown, with where it would stand; for a
  // counteroffer only what differs, and how many items did not.
  const shown = amendment ? changes.items : changes.items.filter((item) => item.kind !== 'UNCHANGED');
  const unchanged = changes.items.length - shown.length;
  const anyItemChanged = changes.items.some((item) => item.kind !== 'UNCHANGED');

  return (
    <View style={[styles.section, { borderTopColor: colors.divider }]}>
      <Heading level={3}>{w.heading}</Heading>
      <Hint>{fmt(amendment ? w.againstInForce : w.againstPrevious, { number: base.sequence })}</Hint>
      {changes.names.map((name) => (
        <Change key={name.slot} label={w.fields.name} before={name.before} after={name.after} />
      ))}
      {changes.termsChanged ? <P>{w.termsChanged}</P> : null}
      {!anyItemChanged ? <P>{w.nothing}</P> : null}
      {shown.length > 0 && (
        <View role="list" style={styles.list}>
          {shown.map((item) => (
            <View
              key={item.id}
              role="listitem"
              style={[styles.item, { borderTopColor: colors.divider }]}>
              <Tags>
                <Tag>{w.kinds[item.kind]}</Tag>
              </Tags>
              <Written>{item.description}</Written>
              {item.fields.map((change) => {
                const text = fieldChangeText(change, i18n, exchange.currency, terms, zone);
                return text.sentence ? (
                  <P key={change.field}>{text.sentence}</P>
                ) : (
                  <Change
                    key={change.field}
                    label={text.label}
                    before={text.before}
                    after={text.after}
                  />
                );
              })}
              {/* The one consequence nobody would guess (DESIGN.md §7): the tag says the rest. */}
              {item.effect === 'CHANGED' ? <P>{wording.composer.effects.CHANGED}</P> : null}
              {item.status && item.status !== 'REMOVED' ? (
                <P style={styles.status}>
                  {fmt(w.statusAfter, { status: statusWording(wording, item.status, item.money) })}
                </P>
              ) : null}
            </View>
          ))}
        </View>
      )}
      {unchanged > 0 && anyItemChanged ? <P>{fmt(w.unchangedCount, { count: unchanged })}</P> : null}
    </View>
  );
}

/** A field written in the parties' own words, before and after. */
function Change({ label, before, after }: { label: string; before: string; after: string }) {
  const { wording } = useI18n();
  const w = wording.proposalChanges;
  return (
    <View style={styles.change}>
      <Label>{label}</Label>
      <Hint>{w.was}</Hint>
      <Written>{before}</Written>
      <Hint>{w.now}</Hint>
      <Written>{after}</Written>
    </View>
  );
}

const styles = StyleSheet.create({
  section: { borderTopWidth: StyleSheet.hairlineWidth, paddingTop: space.m, gap: space.s },
  list: { gap: space.m },
  item: { borderTopWidth: StyleSheet.hairlineWidth, paddingTop: space.m, gap: space.xs },
  change: { gap: space.xs },
  status: { fontFamily: fonts.textBold },
});
