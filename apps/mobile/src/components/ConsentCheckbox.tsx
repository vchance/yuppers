import { consentPieces } from '@yuppers/shared';
import { Linking, Pressable, StyleSheet, Text, View } from 'react-native';

import { useI18n } from '../lib/context';
import { space, TOUCH_TARGET, type, useColors } from '../lib/theme';

/** Opens an address in the system's browser; a device without one has nothing better. */
function openInBrowser(url: string): void {
  Linking.openURL(url).catch(() => {});
}

interface Props {
  /** The consent wording, word for word; its addresses become links. */
  wording: string;
  checked: boolean;
  onChange(checked: boolean): void;
  disabled?: boolean;
}

/**
 * A box beside consent wording to texts, as on the web
 * (`apps/web/src/components/ConsentCheckbox.tsx`): the box beside a number a
 * code is to be texted to (`smsCode`), and the box that turns on an
 * agreement's text updates (`smsUpdates.consent`). It is never ticked to
 * begin with. The box's name is the wording, and its two addresses are
 * links that open in the browser.
 */
export function ConsentCheckbox({ wording, checked, onChange, disabled = false }: Props) {
  const { wording: words, language } = useI18n();
  const colors = useColors();
  return (
    <View style={styles.consent}>
      <Pressable
        accessibilityRole="checkbox"
        accessibilityLabel={wording}
        accessibilityLanguage={language}
        accessibilityState={{ checked, disabled }}
        aria-checked={checked}
        disabled={disabled}
        onPress={() => onChange(!checked)}
        style={styles.box}>
        <View
          style={[
            styles.square,
            {
              borderColor: checked ? colors.primary : colors.border,
              backgroundColor: checked ? colors.primary : colors.background,
            },
          ]}>
          {checked ? (
            <Text
              aria-hidden
              importantForAccessibility="no"
              style={[type.body, { color: colors.onPrimary }]}>
              ✓
            </Text>
          ) : null}
        </View>
      </Pressable>
      <Text
        accessibilityLanguage={language}
        style={[type.body, styles.consentText, { color: colors.text }]}>
        {consentPieces(wording).map((piece, index) =>
          'url' in piece ? (
            <Text
              key={index}
              accessibilityRole="link"
              accessibilityHint={words.help.inBrowser}
              onPress={() => openInBrowser(piece.url)}
              style={[styles.link, { color: colors.primary }]}>
              {piece.url}
            </Text>
          ) : (
            piece.text
          ),
        )}
      </Text>
    </View>
  );
}

const styles = StyleSheet.create({
  consent: { flexDirection: 'row', alignItems: 'flex-start', gap: space.m },
  box: {
    minHeight: TOUCH_TARGET,
    minWidth: TOUCH_TARGET,
    alignItems: 'center',
    justifyContent: 'center',
  },
  square: {
    width: 28,
    height: 28,
    borderWidth: 2,
    borderRadius: 4,
    alignItems: 'center',
    justifyContent: 'center',
  },
  consentText: { flex: 1, paddingTop: space.s },
  link: { textDecorationLine: 'underline' },
});
