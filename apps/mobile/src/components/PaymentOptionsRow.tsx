import {
  directionOf,
  paymentOptionsSummary,
  useSavedPaymentHandles,
  type ExchangeApi,
} from '@yuppers/shared';
import { useFocusEffect, useRouter } from 'expo-router';
import { useCallback, useRef, useState } from 'react';
import { Pressable, StyleSheet, Text, View } from 'react-native';

import { useI18n } from '../lib/context';
import { api } from '../lib/session';
import { fonts, radius, space, TOUCH_TARGET, type, useColors } from '../lib/theme';

interface Props {
  /** For tests: the service's calls. */
  client?: Pick<ExchangeApi, 'paymentHandles'>;
}

/**
 * Payment options on the account screen, as on the web
 * (`apps/web/src/components/PaymentOptionsRow.tsx`): one row, named for
 * them, with the apps added ("Venmo, Zelle") or "None added", that opens
 * their own screen. Read as one control: its name, then the summary.
 */
export function PaymentOptionsRow({ client = api }: Props) {
  const { wording, language } = useI18n();
  const colors = useColors();
  const router = useRouter();
  const w = wording.payments;
  const saved = useSavedPaymentHandles(client, useShownAgain());
  // Unknown while loading, and if it cannot be loaded: the row still opens the screen.
  const summary = saved ? paymentOptionsSummary(saved, w.apps, w.noneAdded) : null;

  return (
    <Pressable
      testID="payment-options-row"
      accessibilityRole="button"
      accessibilityLabel={summary ? `${w.heading}, ${summary}` : w.heading}
      accessibilityLanguage={language}
      onPress={() => router.push('/account/payments')}
      style={({ pressed }) => [
        styles.row,
        { borderColor: colors.divider, backgroundColor: colors.surface },
        pressed && styles.pressed,
      ]}>
      <View style={styles.text}>
        <Text style={[type.heading, styles.title, { color: colors.text }]}>{w.heading}</Text>
        {summary ? (
          <Text style={[type.hint, { color: colors.muted }]}>{summary}</Text>
        ) : null}
      </View>
      {/* Points on in the direction the screen reads. */}
      <Text
        aria-hidden
        importantForAccessibility="no"
        style={[styles.chevron, { color: colors.muted }, directionOf(language) === 'rtl' && styles.flipped]}>
        ›
      </Text>
    </Pressable>
  );
}

const styles = StyleSheet.create({
  row: {
    minHeight: TOUCH_TARGET,
    flexDirection: 'row',
    alignItems: 'center',
    gap: space.m,
    borderWidth: 1,
    borderRadius: radius.l,
    padding: space.l,
  },
  pressed: { opacity: 0.6 },
  text: { flex: 1, gap: space.xs },
  title: { fontFamily: fonts.display },
  chevron: { fontSize: 32, lineHeight: 36 },
  flipped: { transform: [{ scaleX: -1 }] },
});

/**
 * A count of the times the screen has come back to the front after it was
 * first shown, to read again what may have changed on another screen.
 */
export function useShownAgain(): number {
  const [times, setTimes] = useState(0);
  const first = useRef(true);
  useFocusEffect(
    useCallback(() => {
      if (first.current) first.current = false;
      else setTimes((count) => count + 1);
    }, []),
  );
  return times;
}
