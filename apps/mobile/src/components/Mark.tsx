import { StyleSheet, View } from 'react-native';

import { useColors } from '../lib/theme';

/*
 * The Yuppers mark: two circles, the reader's in yellow and the other
 * party's in blue, and where they overlap, agree green. Drawn with plain
 * views (the app has no SVG renderer): the overlap is a green circle placed
 * where the blue one is, clipped by the yellow one. Decorative: whatever it
 * stands beside says the same in words, so it is hidden from screen readers.
 */

/** The mark, `width` points wide; with `check`, a check on the overlap for "signed by both". */
export function Mark({ width, check = false }: { width: number; check?: boolean }) {
  const colors = useColors();
  // On the 112 by 80 grid of the web mark: radius 34, centres 32 apart.
  const unit = width / 112;
  const size = 68 * unit;
  const offset = 32 * unit;
  const circle = { width: size, height: size, borderRadius: size / 2 };
  return (
    <View
      testID="mark"
      aria-hidden
      accessibilityElementsHidden
      importantForAccessibility="no-hide-descendants"
      style={{ width: (68 + 32) * unit, height: size, direction: 'ltr' }}>
      <View style={[styles.at, circle, { left: offset, backgroundColor: colors.partyThem }]} />
      <View style={[styles.at, circle, styles.clip, { left: 0, backgroundColor: colors.partyYou }]}>
        <View style={[styles.at, circle, { left: offset, backgroundColor: colors.agree }]} />
      </View>
      {check ? (
        <View
          style={[
            styles.check,
            {
              left: offset + (size - offset) / 2 - 7 * unit,
              top: size / 2 - 13 * unit,
              width: 12 * unit,
              height: 22 * unit,
              borderColor: colors.onAgree,
              borderRightWidth: 4.5 * unit,
              borderBottomWidth: 4.5 * unit,
            },
          ]}
        />
      ) : null}
    </View>
  );
}

const styles = StyleSheet.create({
  at: { position: 'absolute', top: 0 },
  clip: { overflow: 'hidden' },
  check: { position: 'absolute', transform: [{ rotate: '45deg' }] },
});
