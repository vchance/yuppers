import type { ExchangeView as Exchange, components } from '@yuppers/api-client';
import {
  fractionDigitsOf,
  fromMinorUnits,
  paymentNote,
  payOptions,
  type ExchangeApi,
  type PaymentHandleChanges,
  type PaymentHandles,
  type PayOption,
} from '@yuppers/shared';
import * as Clipboard from 'expo-clipboard';
import { useEffect, useRef, useState } from 'react';
import { Linking, Modal, ScrollView, StyleSheet, Text, View } from 'react-native';
import { useSafeAreaInsets } from 'react-native-safe-area-context';

import { focusOn } from '../lib/accessibility';
import { useI18n } from '../lib/context';
import { api } from '../lib/session';
import { radius, space, type, useColors } from '../lib/theme';
import { Actions, Button, ErrorNote, Heading, Hint, Label, Notice, P } from './ui';

type Contribution = components['schemas']['ContributionDto'];

/** Opens a payment link: the app where it is installed, its website otherwise. */
export function openPaymentLink(url: string): void {
  // A web address, so iOS hands it to the app that claims it (a universal
  // link) and Android likewise (an app link), with no URL scheme to declare
  // (`LSApplicationQueriesSchemes` is only for asking about schemes). On the
  // web it opens a new tab.
  Linking.openURL(url).catch(() => {});
}

interface Props {
  exchange: Exchange;
  contribution: Contribution;
  otherName: string;
  onClose(found: Exchange | null): void;
  /** "I've paid": opens the existing claim, which the payer still sends themselves. */
  onPaid(found: Exchange | null): void;
  /** For tests: the service's calls, and what opens a link. */
  client?: Pick<ExchangeApi, 'getExchange'>;
  open?: (url: string) => void;
}

/**
 * "Pay {name} {amount}", as on the web (`apps/web/src/components/PaySheet.tsx`):
 * a modal sheet of the payee's payment options as plain-text buttons that
 * open each app, prefilled where the app takes it, and Zelle's address or
 * number to copy. Yuppers moves no money and records nothing when a link is
 * opened; "I've paid" opens the usual claim, which the payer sends. The
 * exchange is read again as the sheet opens, so options the payee stopped
 * showing are never offered.
 */
export function PaySheet({
  exchange,
  contribution,
  otherName,
  onClose,
  onPaid,
  client = api,
  open = openPaymentLink,
}: Props) {
  const { wording, fmt, money, language } = useI18n();
  const colors = useColors();
  const insets = useSafeAreaInsets();
  const w = wording.payments;
  const heading = useRef<Text>(null);
  const [found, setFound] = useState<Exchange | null>(null);
  const [handles, setHandles] = useState<PaymentHandles | null | undefined>(
    exchange.payment_options?.theirs,
  );
  const [changes, setChanges] = useState<PaymentHandleChanges | null | undefined>(
    exchange.payment_options?.theirs_changed,
  );

  const amountMinor = contribution.amount_minor ?? 0;
  const amount = money(amountMinor, exchange.currency);
  const plainAmount = fromMinorUnits(amountMinor, fractionDigitsOf(exchange.currency));
  const note = paymentNote(w.note, contribution.description, exchange.display_code);
  const options = payOptions(handles, amountMinor, exchange.currency, note, changes);
  const title = fmt(w.sheetTitle, { name: otherName, amount });

  useEffect(() => {
    let cancelled = false;
    client.getExchange(exchange.id).then(
      (fresh) => {
        if (cancelled) return;
        setFound(fresh);
        setHandles(fresh.payment_options?.theirs);
        setChanges(fresh.payment_options?.theirs_changed);
      },
      () => {},
    );
    return () => {
      cancelled = true;
    };
  }, [client, exchange.id]);

  return (
    <Modal
      visible
      transparent
      animationType="slide"
      onRequestClose={() => onClose(found)}
      onShow={() => focusOn(heading.current as unknown as View | null)}>
      <View style={[styles.backdrop, { backgroundColor: 'rgba(17, 19, 39, 0.6)' }]}>
        <View
          accessibilityViewIsModal
          aria-modal
          role="dialog"
          aria-label={title}
          testID="pay-sheet"
          style={[
            styles.sheet,
            {
              backgroundColor: colors.surface,
              borderColor: colors.divider,
              paddingBottom: Math.max(insets.bottom, space.l),
            },
          ]}>
          <ScrollView contentContainerStyle={styles.content}>
            <Text
              ref={heading}
              accessibilityRole="header"
              accessibilityLanguage={language}
              aria-level={2}
              style={[type.heading, { color: colors.text }]}>
              {title}
            </Text>
            {options.length === 0 ? (
              <Notice>{fmt(w.gone, { name: otherName })}</Notice>
            ) : (
              <>
                <Notice tone="warning">{fmt(w.addedBy, { name: otherName })}</Notice>
                <Hint>{`${wording.terms.moneyOutside} ${w.appTerms}`}</Hint>
                <Fact label={w.amountLabel} value={amount} copy={plainAmount} />
                <Fact label={w.noteLabel} value={note} copy={note} />
                {options.map((option) => (
                  <Option
                    key={option.app}
                    option={option}
                    amount={amount}
                    otherName={otherName}
                    open={open}
                  />
                ))}
              </>
            )}
            <Heading level={3}>{w.afterHeading}</Heading>
            <P>{fmt(w.afterText, { name: otherName })}</P>
            <Actions>
              <Button
                testID="pay-sheet-paid"
                variant="primary"
                label={wording.exchange.moneyMoves.CLAIM}
                onPress={() => onPaid(found)}
              />
              <Button testID="pay-sheet-close" label={w.close} onPress={() => onClose(found)} />
            </Actions>
          </ScrollView>
        </View>
      </View>
    </Modal>
  );
}

function Fact({ label, value, copy }: { label: string; value: string; copy: string }) {
  const colors = useColors();
  return (
    <View style={styles.fact}>
      <Label>{label}</Label>
      <View style={styles.row}>
        <Text selectable style={[type.body, styles.grow, { color: colors.text }]}>
          {value}
        </Text>
        <Copy text={copy} what={label} />
      </View>
    </View>
  );
}

function Option({
  option,
  amount,
  otherName,
  open,
}: {
  option: PayOption;
  amount: string;
  otherName: string;
  open(url: string): void;
}) {
  const { wording, fmt, moment } = useI18n();
  const colors = useColors();
  const w = wording.payments;
  // Changed while money is owed: the way a payment is stolen after an
  // account is taken over. Said before the button, never the old value.
  const warning = option.changedAt
    ? fmt(w.changed[option.app], { name: otherName, date: moment(option.changedAt) })
    : null;
  const changed = warning ? <ErrorNote>{warning}</ErrorNote> : null;
  const boxStyle = [styles.option, { borderColor: colors.divider, backgroundColor: colors.surfaceRaised }];

  if (option.app === 'zelle') {
    return (
      <View style={boxStyle} testID="pay-option-zelle">
        <Heading level={3}>{w.zelleHeading}</Heading>
        {changed}
        <P>{w.zelleNoLinks}</P>
        <View style={styles.row}>
          <Text
            selectable
            style={[type.subheading, styles.grow, styles.ltr, { color: colors.text }]}>
            {option.shown}
          </Text>
          <Copy text={option.shown} what={w.zelleHeading} />
        </View>
        <Hint>{fmt(w.enterAmount, { amount })}</Hint>
      </View>
    );
  }

  const handle = fmt(w.handle[option.app], { handle: option.handle });
  const told = !option.amountFilled
    ? fmt(w.enterAmount, { amount })
    : option.noteFilled
      ? fmt(w.prefillsAmountAndNote, { amount })
      : fmt(w.prefillsAmount, { amount });
  return (
    <View style={boxStyle} testID={`pay-option-${option.app}`}>
      {changed}
      <Actions>
        <Button
          testID={`pay-open-${option.app}`}
          label={w.open[option.app]}
          hint={[warning, `${handle}.`, told].filter(Boolean).join(' ')}
          onPress={() => open(option.url)}
        />
      </Actions>
      <Text style={[type.subheading, styles.ltr, { color: colors.text }]}>{handle}</Text>
      <Hint>{told}</Hint>
    </View>
  );
}

/** A button that copies one thing, and says when it has. */
function Copy({ text, what }: { text: string; what: string }) {
  const { wording, fmt } = useI18n();
  const w = wording.payments;
  const [state, setState] = useState<'idle' | 'copied' | 'failed'>('idle');

  async function copy() {
    try {
      setState((await Clipboard.setStringAsync(text)) ? 'copied' : 'failed');
    } catch {
      setState('failed');
    }
  }

  return (
    <View style={styles.copy}>
      <Button label={w.copy} accessibilityLabel={fmt(w.copyWhat, { what })} onPress={() => void copy()} />
      {state !== 'idle' ? (
        <Hint>{state === 'copied' ? w.copied : w.copyFailed}</Hint>
      ) : null}
    </View>
  );
}

const styles = StyleSheet.create({
  backdrop: { flex: 1, justifyContent: 'flex-end' },
  sheet: {
    maxHeight: '92%',
    width: '100%',
    maxWidth: 640,
    alignSelf: 'center',
    borderTopLeftRadius: radius.l,
    borderTopRightRadius: radius.l,
    borderWidth: 1,
  },
  content: { padding: space.l, gap: space.m },
  fact: { gap: space.xs },
  row: { flexDirection: 'row', flexWrap: 'wrap', alignItems: 'center', gap: space.m },
  grow: { flexGrow: 1, flexShrink: 1 },
  option: { borderWidth: 2, borderRadius: radius.m, padding: space.m, gap: space.s },
  copy: { gap: space.xs },
  ltr: { writingDirection: 'ltr' },
});
