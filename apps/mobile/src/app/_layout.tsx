import { directionOf } from '@yuppers/shared';
import { useFonts } from 'expo-font';
import { DarkTheme, DefaultTheme, LocaleProvider, Stack, ThemeProvider } from 'expo-router';
import { StatusBar } from 'expo-status-bar';
import { useEffect } from 'react';
import { Platform, StyleSheet, Text, View } from 'react-native';
import { SafeAreaProvider } from 'react-native-safe-area-context';

import { Actions, Button, Heading, Screen } from '../components/ui';
import { AppProviders, useI18n, useSession } from '../lib/context';
import { useReduceMotion } from '../lib/accessibility';
import { installPluralRules } from '../lib/plural-rules';
import { installNotificationHandling, syncDevice, useNotificationTaps } from '../lib/push';
import { holdSplash, useSplashUntil, useWindowBackground } from '../lib/splash';
import { colorsFor, FONT_FILES, fonts, useScheme } from '../lib/theme';

// Before any wording is formatted: the engine lacks the plural rules it needs.
installPluralRules();
// The launch screen stays until the app knows who is signed in (`lib/splash.ts`).
holdSplash();
// A notification arriving while the app is open is not shown (`lib/push.ts`).
installNotificationHandling();

export default function RootLayout() {
  // The bundled fonts, loaded while the launch screen is still up. Nothing is
  // drawn before they are, or before they fail to, in which case the system
  // font stands in.
  const [fontsLoaded, fontsFailed] = useFonts(FONT_FILES);
  if (!fontsLoaded && !fontsFailed) return null;
  return (
    <SafeAreaProvider>
      <AppProviders>
        <Navigation />
      </AppProviders>
    </SafeAreaProvider>
  );
}

/**
 * One stack of screens. Titles come from the wording; colors follow the
 * system's light or dark setting, or the one chosen on this device; the layout runs in the direction of the
 * language being shown (DESIGN.md §4.2).
 */
function Navigation() {
  const { wording, language } = useI18n();
  const { outdated, ready, account } = useSession();
  const scheme = useScheme();
  const colors = colorsFor(scheme);
  useWindowBackground(colors);
  useSplashUntil(ready);
  // A tapped notification opens its exchange; signed in, the service's
  // record of this device is kept current (`lib/push.ts`).
  useNotificationTaps();
  const accountId = account?.id;
  const accountLanguage = account?.language;
  const channel = wording.mobile.notifications.channel;
  useEffect(() => {
    if (accountId && accountLanguage) void syncDevice(accountLanguage, channel);
  }, [accountId, accountLanguage, channel]);
  const direction = directionOf(language);
  const base = scheme === 'dark' ? DarkTheme : DefaultTheme;
  // Screens slide in, unless the person has asked for less motion.
  const reduceMotion = useReduceMotion();

  return (
    <ThemeProvider
      value={{
        ...base,
        colors: {
          ...base.colors,
          background: colors.background,
          card: colors.background,
          text: colors.text,
          border: colors.border,
          primary: colors.link,
        },
      }}>
      <LocaleProvider direction={direction}>
        <View style={[styles.fill, { direction, backgroundColor: colors.background }]}>
          {/* A build too old to act shows that it must be updated, and nothing else. */}
          {outdated && <Outdated />}
          <Stack
            screenOptions={{
              title: wording.productName,
              headerBackTitle: wording.mobile.back,
              headerTintColor: colors.link,
              headerTitleStyle: { color: colors.text, fontFamily: fonts.display },
              // In the browser harness the bar's own title would be a second
              // `h1` above the screen's (React Navigation marks it as one);
              // there it is plain text. A device keeps the system's own bar,
              // whose title VoiceOver and TalkBack already treat as a heading.
              // An element, not the component: the bar calls `headerTitle` as
              // a plain function, so a component's hooks would run as the
              // bar's own, and a screen that sets a title of its own
              // (`index.tsx`) would then change how many hooks the bar has.
              headerTitle:
                Platform.OS === 'web'
                  ? ({ children }) => <NavigationTitle>{children}</NavigationTitle>
                  : undefined,
              headerStyle: { backgroundColor: colors.background },
              contentStyle: { backgroundColor: colors.background },
              animation: reduceMotion ? 'none' : 'default',
            }}>
            <Stack.Screen name="index" />
            <Stack.Screen name="account" options={{ title: wording.nav.account }} />
            <Stack.Screen name="account/payments" options={{ title: wording.payments.heading }} />
            <Stack.Screen name="invitation" />
            <Stack.Screen name="new" options={{ title: wording.templates.chooserTitle }} />
            <Stack.Screen name="new/[from]" options={{ title: wording.composer.titleFirst }} />
            <Stack.Screen name="example" options={{ title: wording.sample.title }} />
            <Stack.Screen
              name="exchanges/[id]/index"
              options={{ title: wording.exchange.titleNoName }}
            />
            <Stack.Screen
              name="exchanges/[id]/revise"
              options={{ title: wording.exchange.titleNoName }}
            />
            <Stack.Screen
              name="exchanges/[id]/record"
              options={{ title: wording.exchange.titleNoName }}
            />
            <Stack.Screen name="[language]/i" options={{ headerShown: false }} />
            <Stack.Screen name="+not-found" options={{ title: wording.common.notFoundTitle }} />
          </Stack>
        </View>
        <StatusBar style={scheme === 'dark' ? 'light' : 'dark'} />
      </LocaleProvider>
    </ThemeProvider>
  );
}

/**
 * The navigation bar's title in the browser harness: the same words, not a
 * heading. Each screen names itself with its own level 1 heading.
 */
function NavigationTitle({ children }: { children: string }) {
  const colors = colorsFor(useScheme());
  return (
    <Text numberOfLines={1} style={[styles.navigationTitle, { color: colors.text }]}>
      {children}
    </Text>
  );
}

/**
 * This build is older than the service accepts changes from. It covers the
 * screens, since nothing in them may offer a change it cannot make, and says
 * the one thing to do. Trying again asks the service once more, for a build
 * updated while the app was open.
 */
function Outdated() {
  const { wording } = useI18n();
  const { retry } = useSession();
  const colors = colorsFor(useScheme());
  return (
    <View style={[styles.cover, { backgroundColor: colors.background }]}>
      <Screen>
        <Heading>{wording.errors.CLIENT_TOO_OLD}</Heading>
        <Actions>
          <Button label={wording.common.tryAgain} onPress={retry} />
        </Actions>
      </Screen>
    </View>
  );
}

const styles = StyleSheet.create({
  fill: { flex: 1 },
  navigationTitle: { fontSize: 18, fontFamily: fonts.display },
  cover: { position: 'absolute', top: 0, bottom: 0, left: 0, right: 0, zIndex: 1 },
});
