import { Pressable, StyleSheet, Text, View } from 'react-native';

import { useI18n } from '../lib/context';
import {
  chooseTheme,
  fonts,
  radius,
  space,
  THEME_CHOICES,
  TOUCH_TARGET,
  type,
  useColors,
  useThemeChoice,
} from '../lib/theme';
import { Heading, Hint } from './ui';

/**
 * System, Light or Dark, for this device (`lib/theme.ts`): three radio
 * buttons drawn as one segmented control. The choice applies at once, so
 * there is nothing to save.
 */
export function Appearance() {
  const { wording, language } = useI18n();
  const w = wording.appearance;
  const colors = useColors();
  const choice = useThemeChoice();
  return (
    <View style={styles.appearance}>
      <Heading level={2}>{w.heading}</Heading>
      <Hint>{w.hint}</Hint>
      <View
        accessibilityRole="radiogroup"
        accessibilityLabel={w.heading}
        style={[styles.segmented, { borderColor: colors.border, backgroundColor: colors.surface }]}>
        {THEME_CHOICES.map((option) => {
          const selected = option === choice;
          return (
            <Pressable
              key={option}
              accessibilityRole="radio"
              accessibilityLabel={w[option]}
              accessibilityLanguage={language}
              accessibilityState={{ checked: selected }}
              aria-checked={selected}
              onPress={() => chooseTheme(option)}
              style={[styles.option, selected && { backgroundColor: colors.primary }]}>
              <Text
                style={[
                  type.body,
                  styles.optionText,
                  { color: selected ? colors.onPrimary : colors.text },
                ]}>
                {w[option]}
              </Text>
            </Pressable>
          );
        })}
      </View>
    </View>
  );
}

const styles = StyleSheet.create({
  appearance: { gap: space.s },
  segmented: {
    flexDirection: 'row',
    gap: space.xs,
    padding: space.xs,
    borderWidth: 2,
    borderRadius: radius.pill,
  },
  option: {
    flex: 1,
    minHeight: TOUCH_TARGET,
    borderRadius: radius.pill,
    alignItems: 'center',
    justifyContent: 'center',
    paddingHorizontal: space.s,
  },
  optionText: { fontFamily: fonts.textBold, textAlign: 'center' },
});
