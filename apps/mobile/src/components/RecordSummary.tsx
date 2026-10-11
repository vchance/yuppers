import { recordDays, summarizeRecord, summaryText, type RecordDocument } from '@yuppers/shared';
import { useMemo } from 'react';
import { StyleSheet, View } from 'react-native';

import { useI18n } from '../lib/context';
import { fonts, space, useColors } from '../lib/theme';
import { Card, Heading, Hint, Lines, P, Written } from './ui';

/**
 * The plain summary at the top of a record (DESIGN.md §14.1): who, what each
 * gives, who signed and when, how it stands or ended, and what became of each
 * item. Worked out in the shared package, so the web says the same. Everyone
 * is named: the copy may be handed to someone who is neither party.
 */
export function RecordSummary({ record }: { record: RecordDocument }) {
  const i18n = useI18n();
  const colors = useColors();
  const { language } = i18n;
  const { timezone, currency } = record.exchange;
  const text = useMemo(
    () => summaryText(summarizeRecord(record), i18n, currency, recordDays(language, timezone)),
    [record, i18n, currency, language, timezone],
  );

  return (
    <Card>
      <Heading level={2}>{text.heading}</Heading>
      <Hint>{text.intro}</Hint>
      <P>{text.between}</P>
      <P>{text.basis}</P>
      {text.sides.map((side) => (
        <View key={side.slot} style={styles.side}>
          <Heading level={3}>{side.heading}</Heading>
          {side.nothing ? <P>{side.nothing}</P> : null}
          {side.counts.map((line) => (
            <P key={line}>{line}</P>
          ))}
          {side.items.length > 0 && (
            <View role="list" style={styles.list}>
              {side.items.map((item) => (
                // One stop for a screen reader: what it is, then what became of it.
                <View
                  key={item.id}
                  role="listitem"
                  accessible
                  style={[styles.item, { borderTopColor: colors.divider }]}>
                  <Written>{item.description}</Written>
                  {item.details.map((detail) => (
                    <P key={detail}>{detail}</P>
                  ))}
                  {item.outcome ? <P style={styles.outcome}>{item.outcome}</P> : null}
                </View>
              ))}
            </View>
          )}
        </View>
      ))}
      {text.signed.length > 0 && (
        <Lines>
          {text.signed.map((line) => (
            <P key={line}>{line}</P>
          ))}
        </Lines>
      )}
      {text.standing.map((line) => (
        <P key={line}>{line}</P>
      ))}
    </Card>
  );
}

const styles = StyleSheet.create({
  side: { gap: space.s },
  list: { gap: space.m },
  item: { borderTopWidth: StyleSheet.hairlineWidth, paddingTop: space.m, gap: space.xs },
  outcome: { fontFamily: fonts.textBold },
});
