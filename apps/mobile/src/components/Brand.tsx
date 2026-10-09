import { Platform, StyleSheet, Text, View } from 'react-native';
import { useSafeAreaInsets } from 'react-native-safe-area-context';

import { useI18n } from '../lib/context';
import { fonts, radius, space, tilt, type, useColors } from '../lib/theme';
import { Mark } from './Mark';

/*
 * The product's name as it is drawn: the mark beside the whole word, lower
 * case in the display face, with the trademark set up small beside it. The
 * word is always whole, never "yup" on its own. To a screen reader the mark
 * and the word are one image, named for the product; the mark says nothing
 * on its own and is hidden (`Mark`).
 */

/** The whole word and its trademark, for the eye: whatever holds it carries the name. */
function Wordmark({ size }: { size: number }) {
  const { wording } = useI18n();
  const colors = useColors();
  return (
    <View
      aria-hidden
      accessibilityElementsHidden
      importantForAccessibility="no-hide-descendants"
      style={styles.word}>
      <Text
        // A logotype, not reading text: it grows with the device's text
        // size, but not without limit, or it would run off the screen.
        maxFontSizeMultiplier={1.5}
        style={{
          fontFamily: fonts.display,
          fontSize: size,
          lineHeight: size * 1.05,
          letterSpacing: -0.04 * size,
          textTransform: 'lowercase',
          color: colors.text,
        }}>
        {wording.productName}
      </Text>
      <Text
        maxFontSizeMultiplier={1.5}
        style={{
          fontFamily: fonts.textBold,
          fontSize: size * 0.24,
          lineHeight: size * 0.3,
          marginTop: size * 0.14,
          color: colors.muted,
        }}>
        {'™'}
      </Text>
    </View>
  );
}

/**
 * The brand at the top of the screens that stand for the product itself,
 * before anyone is signed in: the mark at a generous size over the wordmark,
 * the two party colours as a band under it, and one line saying what Yuppers
 * is. The same picture as the launch screen (`app.json`, the splash image),
 * so the app does not drop from its mark into a plain form. It is not a
 * heading: the screen's own heading follows it.
 *
 * The screen it opens has no navigation bar, so on Android, where the status
 * bar is drawn over the app, it keeps clear of it itself; iOS adjusts the
 * scrolling content for the status bar on its own, as does a browser.
 */
export function BrandHero() {
  const { wording, language } = useI18n();
  const colors = useColors();
  const insets = useSafeAreaInsets();
  return (
    <View style={[styles.hero, Platform.OS === 'android' && { paddingTop: insets.top }]}>
      <View
        accessible
        accessibilityRole="image"
        accessibilityLabel={wording.productName}
        style={styles.lockup}>
        {/* The one tilted thing on the screen, and only by a couple of degrees. */}
        <View style={styles.tilted}>
          <Mark width={144} />
        </View>
        <Wordmark size={44} />
      </View>
      <TandemBand />
      <Text
        accessibilityLanguage={language}
        style={[type.body, styles.tagline, { color: colors.muted }]}>
        {wording.tagline}
      </Text>
    </View>
  );
}

/**
 * The brand small, as the title of the navigation bar on the first screen
 * once signed in: the mark and the wordmark, in place of the plain word. To a
 * screen reader it is one image, named for the product, not a heading: the
 * screen names itself with its own level 1 heading, as every screen does.
 */
export function BrandTitle() {
  const { wording } = useI18n();
  return (
    <View
      accessible
      accessibilityRole="image"
      accessibilityLabel={wording.productName}
      style={styles.title}>
      <Mark width={36} />
      <Wordmark size={22} />
    </View>
  );
}

/** You, where you meet, them: the mark's colours as a short rule, in the order the mark draws them. */
function TandemBand() {
  const colors = useColors();
  return (
    <View
      aria-hidden
      accessibilityElementsHidden
      importantForAccessibility="no-hide-descendants"
      style={styles.band}>
      <View style={[styles.stripe, { backgroundColor: colors.partyYou }]} />
      <View style={[styles.seam, { backgroundColor: colors.agree }]} />
      <View style={[styles.stripe, { backgroundColor: colors.partyThem }]} />
    </View>
  );
}

const styles = StyleSheet.create({
  hero: { alignItems: 'center', gap: space.m, paddingTop: space.s, paddingBottom: space.s },
  lockup: { alignItems: 'center', gap: space.l },
  tilted: { transform: [{ rotate: `${tilt.calloutAlt}deg` }] },
  word: { flexDirection: 'row', alignItems: 'flex-start', direction: 'ltr' },
  tagline: { textAlign: 'center', maxWidth: 400 },
  title: { flexDirection: 'row', alignItems: 'center', gap: space.s },
  band: {
    flexDirection: 'row',
    width: 72,
    height: 6,
    borderRadius: radius.pill,
    overflow: 'hidden',
    direction: 'ltr',
  },
  stripe: { flex: 1 },
  seam: { width: 12 },
});
