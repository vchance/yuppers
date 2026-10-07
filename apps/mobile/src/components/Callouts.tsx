import { labelText } from '@yuppers/shared';
import { useEffect, useState } from 'react';
import { AccessibilityInfo, Animated, Easing, StyleSheet, Text, View } from 'react-native';

import { useI18n } from '../lib/context';
import { fonts, radius, space, tilt, type, useColors, useScheme } from '../lib/theme';
import { Mark } from './Mark';

/*
 * The yup card, the one thing on a screen that may be tilted, and only by a
 * degree or two: never the terms, a control, a status or the history.
 */

/** The callout's shadow and edge, so it lifts off the screen in light and in dark alike. */
function useElevation() {
  const colors = useColors();
  const dark = useScheme() === 'dark';
  return {
    borderWidth: 1,
    borderColor: colors.divider,
    shadowColor: dark ? '#000000' : '#1A1B2E',
    shadowOpacity: dark ? 0.75 : 0.45,
    shadowRadius: 15,
    shadowOffset: { width: 0, height: 12 },
    elevation: 8,
  };
}

/**
 * Signed by both: the mark with a check on the overlap, and the words. It
 * comes in tilted and settles straight; for someone who has asked for less
 * motion it is shown straight from the start.
 */
export function AgreedHero({ children }: { children: string }) {
  const colors = useColors();
  const { language } = useI18n();
  const elevation = useElevation();
  const [settle] = useState(() => new Animated.Value(0));
  const [still, setStill] = useState<boolean | null>(null);

  useEffect(() => {
    let current = true;
    AccessibilityInfo.isReduceMotionEnabled().then(
      (reduce) => {
        if (current) setStill(reduce);
      },
      () => {
        if (current) setStill(true);
      },
    );
    return () => {
      current = false;
    };
  }, []);

  useEffect(() => {
    if (still === null) return;
    if (still) {
      settle.setValue(1);
      return;
    }
    const animation = Animated.timing(settle, {
      toValue: 1,
      duration: 900,
      delay: 200,
      easing: Easing.out(Easing.back(1.4)),
      useNativeDriver: true,
    });
    animation.start();
    return () => animation.stop();
  }, [still, settle]);

  const rotate = settle.interpolate({ inputRange: [0, 1], outputRange: [`${tilt.calloutAlt}deg`, '0deg'] });
  const translateY = settle.interpolate({ inputRange: [0, 1], outputRange: [-8, 0] });
  const scale = settle.interpolate({ inputRange: [0, 1], outputRange: [0.96, 1] });

  return (
    <Animated.View
      style={[
        styles.agreed,
        elevation,
        { backgroundColor: colors.surfaceRaised },
        // Until the person's setting is known, it waits, straight.
        still === null ? null : { transform: [{ translateY }, { rotate }, { scale }] },
      ]}>
      <Mark width={128} check />
      <Text
        accessibilityLanguage={language}
        style={[type.title, styles.agreedText, { color: colors.text }]}>
        {children}
      </Text>
    </Animated.View>
  );
}

/**
 * The invitation at a glance: who it is from in their colour, who it is for
 * in the reader's, the mark on the seam. It repeats the names the terms give
 * below, so it is hidden from screen readers.
 */
export function YupCard({ from, to }: { from: string; to: string }) {
  const colors = useColors();
  const elevation = useElevation();
  return (
    <View
      aria-hidden
      accessibilityElementsHidden
      importantForAccessibility="no-hide-descendants"
      style={[
        styles.card,
        elevation,
        { backgroundColor: colors.surface, transform: [{ rotate: `${tilt.callout}deg` }] },
      ]}>
      {/* Clipped inside, so the shadow outside is not. */}
      <View style={styles.sides}>
        <View style={[styles.side, { backgroundColor: colors.partyThem }]}>
          <Text style={[styles.name, { color: colors.onPartyThem }]}>{labelText(from)}</Text>
        </View>
        <View style={[styles.side, styles.end, { backgroundColor: colors.partyYou }]}>
          <Text style={[styles.name, styles.endText, { color: colors.onPartyYou }]}>
            {labelText(to)}
          </Text>
        </View>
      </View>
      <View style={[styles.seam, { backgroundColor: colors.surface }]}>
        <Mark width={40} />
      </View>
    </View>
  );
}

const styles = StyleSheet.create({
  agreed: {
    alignSelf: 'center',
    alignItems: 'center',
    gap: space.s,
    width: '100%',
    maxWidth: 340,
    borderRadius: radius.l,
    paddingVertical: space.l,
    paddingHorizontal: space.l,
  },
  agreedText: { textAlign: 'center' },
  card: {
    alignSelf: 'center',
    width: '100%',
    maxWidth: 360,
    marginVertical: space.s,
    borderRadius: radius.l,
  },
  sides: { flexDirection: 'row', minHeight: 112, borderRadius: radius.l, overflow: 'hidden' },
  side: { flex: 1, justifyContent: 'flex-end', padding: space.l },
  end: { alignItems: 'flex-end' },
  name: { fontFamily: fonts.display, fontSize: 24, lineHeight: 26 },
  endText: { textAlign: 'right' },
  seam: {
    position: 'absolute',
    top: space.m,
    alignSelf: 'center',
    left: '50%',
    marginLeft: -28,
    padding: space.s,
    borderRadius: 28,
  },
});
