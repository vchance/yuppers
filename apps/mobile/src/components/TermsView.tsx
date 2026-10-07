import type { components, RevisionTerms } from '@yuppers/api-client';
import { dueDateZone, dueOnDateText, isOverdue, todayIn, type Slot } from '@yuppers/shared';
import { useMemo, type ReactNode } from 'react';
import { StyleSheet, View } from 'react-native';

import { useI18n } from '../lib/context';
import { deviceTimezone } from '../lib/time-zone';
import { radius, space, useColors } from '../lib/theme';
import { HelpLink } from './HelpLink';
import { Heading, Hint, Label, P, Tag, Tags, Written } from './ui';

type Contribution = components['schemas']['ContributionDto'];
type Status = components['schemas']['Status'];

interface Props {
  terms: RevisionTerms;
  currency: string;
  /** The exchange's timezone, which its due dates are read in. Unknown before joining. */
  timezone?: string;
  /** Which side the reader is; `null` for someone who has not joined yet. */
  you: Slot | null;
  /** Where each contribution stands, once the terms are in force. */
  statuses?: ReadonlyMap<string, Status>;
  /** Shown under a contribution: its status and what can be done about it. */
  footer?: (contribution: Contribution) => ReactNode;
}

/**
 * The complete terms of a revision, with nothing collapsed or left for later:
 * this is what a signature covers (DESIGN.md §14.1). Used wherever terms are
 * read, so a proposal looks the same before signing as the agreement does
 * after.
 */
export function TermsView({ terms, currency, timezone, you, statuses, footer }: Props) {
  const i18n = useI18n();
  const { wording, fmt, money } = i18n;
  const colors = useColors();
  const w = wording.terms;
  const today = timezone ? todayIn(timezone) : null;
  // Named beside each due date when the reader's device keeps another zone.
  const zone = useMemo(() => dueDateZone(timezone, deviceTimezone()), [timezone]);
  const nameOf = (slot: Slot) => (slot === 'A' ? terms.party_a_name : terms.party_b_name);
  // Whose side is drawn in the reader's colour. Someone who has not joined
  // yet is reading an invitation to take the invited party's place.
  const coloured = you ?? 'B';

  function due(contribution: Contribution): string {
    const condition = contribution.due;
    if (condition.kind === 'DATE') return dueOnDateText(i18n, condition.date, zone);
    if (condition.kind === 'ON_AGREEMENT') return w.dueOnAgreement;
    const awaited = terms.contributions.find((other) => other.id === condition.contribution);
    return fmt(w.dueAfter, { description: awaited?.description ?? '' });
  }

  return (
    <View style={styles.terms}>
      <Hint>{w.ownWords}</Hint>

      <Heading level={3}>{w.partiesHeading}</Heading>
      {(['A', 'B'] as const).map((slot) => (
        <Written key={slot}>
          {slot === you ? fmt(wording.party.nameYou, { name: nameOf(slot) }) : nameOf(slot)}
        </Written>
      ))}

      {terms.terms.trim() !== '' && (
        <>
          <Heading level={3}>{w.termsHeading}</Heading>
          <Written>{terms.terms}</Written>
        </>
      )}

      {(['A', 'B'] as const).map((slot) => {
        const provided = terms.contributions.filter((contribution) => contribution.from === slot);
        // Each side in its party's colour, the reader's in yellow and the
        // other party's in blue, headed by whose it is.
        const ink = slot === coloured ? colors.onPartyYou : colors.onPartyThem;
        return (
          <View
            key={slot}
            style={[
              styles.party,
              { backgroundColor: slot === coloured ? colors.partyYou : colors.partyThem },
            ]}>
            <Heading level={3} color={ink}>
              {slot === you ? w.youProvide : fmt(w.otherProvides, { name: nameOf(slot) })}
            </Heading>
            {provided.length === 0 && <P style={{ color: ink }}>{w.nothing}</P>}
            {provided.map((contribution) => {
              const status = statuses?.get(contribution.id);
              const overdue =
                status !== undefined &&
                today !== null &&
                isOverdue(status, contribution.due, today);
              return (
                <View
                  key={contribution.id}
                  style={[styles.contribution, { backgroundColor: colors.surface }]}>
                  <Tags>
                    <Tag>{wording.contributionTypes[contribution.type]}</Tag>
                    <Tag>{contribution.required ? w.required : w.optional}</Tag>
                    {overdue && <Tag alert>{w.overdue}</Tag>}
                  </Tags>
                  <Written>{contribution.description}</Written>
                  {contribution.amount_minor != null && (
                    <P>{fmt(w.amount, { amount: money(contribution.amount_minor, currency) })}</P>
                  )}
                  {/* Money is paid outside the product and only recorded here (DESIGN.md §11). */}
                  {contribution.type === 'MONEY' && (
                    <>
                      <Hint>{w.moneyOutside}</Hint>
                      <HelpLink place="moneyOutside" />
                    </>
                  )}
                  {contribution.quantity && (
                    <P>
                      {contribution.quantity.unit
                        ? fmt(w.quantityWithUnit, {
                            amount: contribution.quantity.amount,
                            unit: contribution.quantity.unit,
                          })
                        : fmt(w.quantity, { amount: contribution.quantity.amount })}
                    </P>
                  )}
                  <P>{due(contribution)}</P>
                  {contribution.completion_criteria ? (
                    <View style={[styles.criteria, { backgroundColor: colors.noticeSurface }]}>
                      <Label>{w.criteriaLabel}</Label>
                      <Written>{contribution.completion_criteria}</Written>
                    </View>
                  ) : null}
                  {footer?.(contribution)}
                </View>
              );
            })}
          </View>
        );
      })}

      {timezone && terms.contributions.some((contribution) => contribution.due.kind === 'DATE') && (
        <Hint>{fmt(w.timezone, { timezone })}</Hint>
      )}
    </View>
  );
}

const styles = StyleSheet.create({
  terms: { gap: space.m },
  party: { borderRadius: radius.l, padding: space.m, gap: space.s },
  contribution: { borderRadius: radius.m, padding: space.m, gap: space.s },
  criteria: { borderRadius: radius.m, padding: space.m, gap: space.xs },
});
