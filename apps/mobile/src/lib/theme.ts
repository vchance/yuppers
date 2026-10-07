import { useSyncExternalStore } from 'react';
import { useColorScheme } from 'react-native';

import { themeStore } from './theme-store';

/*
 * Tandem: two people, two colours, and where they meet it turns green. A
 * light, cool background with ink actions; the parties' colours are kept to
 * the agreement itself. Gabarito for headings, Instrument Sans for text, at
 * the size the person has chosen for their device. Every text and background
 * pair here meets WCAG AA contrast for body text (`theme.test.ts`).
 *
 * The palettes follow the system's light or dark setting unless this device
 * chose Light or Dark (the account screen); the choice is kept on the device
 * only, never with the account.
 */

export interface Colors {
  /** The screen. */
  background: string;
  /** Cards and fields. */
  surface: string;
  /** Panels and notices, a step above a card in dark. */
  surfaceRaised: string;
  surfaceStrong: string;
  onStrong: string;
  onStrongMuted: string;
  /** Decorative rules and edges only. */
  divider: string;
  text: string;
  muted: string;
  /** The edges of controls. */
  border: string;
  link: string;
  focus: string;
  /** Primary buttons: ink in light, chalk in dark. */
  primary: string;
  onPrimary: string;
  /** You, as a fill, with the text drawn on it. */
  partyYou: string;
  onPartyYou: string;
  /** Your entries in the history, one-time notices. */
  warningSurface: string;
  /** The other party, as a fill, with the text drawn on it. */
  partyThem: string;
  onPartyThem: string;
  /** Their entries in the history, "Done when", notices. */
  noticeSurface: string;
  /** Their name as text. */
  partyThemText: string;
  /** Where the two meet: the overlap, signed by both. */
  agree: string;
  onAgree: string;
  statusWaiting: string;
  onStatusWaiting: string;
  statusDisputed: string;
  onStatusDisputed: string;
  /** Dispute text, errors. */
  danger: string;
  dangerSurface: string;
  statusConfirmed: string;
  onStatusConfirmed: string;
  statusConfirmedIcon: string;
}

const light: Colors = {
  background: '#F4F4EF',
  surface: '#FFFFFF',
  surfaceRaised: '#FFFFFF',
  surfaceStrong: '#1A1B2E',
  onStrong: '#FFFFFF',
  onStrongMuted: '#D9D8E4',
  divider: '#DCDCE3',
  text: '#1A1B2E',
  muted: '#4F5066',
  border: '#7A7B90',
  link: '#2F4BE0',
  focus: '#2F4BE0',
  primary: '#1A1B2E',
  onPrimary: '#FFFFFF',
  partyYou: '#FFD23F',
  onPartyYou: '#1A1B2E',
  warningSurface: '#FFF1C2',
  partyThem: '#2F4BE0',
  onPartyThem: '#FFFFFF',
  noticeSurface: '#E3E9FF',
  partyThemText: '#2F4BE0',
  agree: '#1B9A5C',
  onAgree: '#1A1B2E',
  statusWaiting: '#D9D8E4',
  onStatusWaiting: '#1A1B2E',
  statusDisputed: '#C2410C',
  onStatusDisputed: '#FFFFFF',
  danger: '#C2410C',
  dangerSurface: '#FDE8DF',
  statusConfirmed: '#1A1B2E',
  onStatusConfirmed: '#FFFFFF',
  statusConfirmedIcon: '#4BD18C',
};

const dark: Colors = {
  background: '#111327',
  surface: '#1B1E36',
  surfaceRaised: '#252946',
  surfaceStrong: '#2B3056',
  onStrong: '#F1F0EA',
  onStrongMuted: '#B4B5C8',
  divider: '#2F3352',
  text: '#F1F0EA',
  muted: '#B4B5C8',
  border: '#7D80A0',
  link: '#9DB0FF',
  focus: '#9DB0FF',
  primary: '#F1F0EA',
  onPrimary: '#111327',
  partyYou: '#F2C94C',
  onPartyYou: '#111327',
  warningSurface: '#3B3420',
  partyThem: '#7B93FF',
  onPartyThem: '#111327',
  noticeSurface: '#1F2A5C',
  partyThemText: '#9DB0FF',
  agree: '#3FCB85',
  onAgree: '#111327',
  statusWaiting: '#3A3D5C',
  onStatusWaiting: '#F1F0EA',
  statusDisputed: '#FF8A5C',
  onStatusDisputed: '#111327',
  danger: '#FF8A5C',
  dangerSurface: '#3A1D14',
  statusConfirmed: '#F1F0EA',
  onStatusConfirmed: '#111327',
  statusConfirmedIcon: '#15803D',
};

export type Scheme = 'light' | 'dark';

/** What this device chose: the system's setting, or Light or Dark whatever it says. */
export type ThemeChoice = 'system' | Scheme;

export const THEME_CHOICES: readonly ThemeChoice[] = ['system', 'light', 'dark'];

export function colorsFor(scheme: Scheme): Colors {
  return scheme === 'dark' ? dark : light;
}

// Read once, synchronously, the first time anything asks: while the launch
// screen is still up, so the first screen is drawn in the right theme.
let choice: ThemeChoice | null = null;
const listeners = new Set<() => void>();

function currentChoice(): ThemeChoice {
  if (choice === null) {
    const stored = themeStore.read();
    choice = stored === 'light' || stored === 'dark' ? stored : 'system';
  }
  return choice;
}

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

export function useThemeChoice(): ThemeChoice {
  return useSyncExternalStore(subscribe, currentChoice, currentChoice);
}

/** Applies a choice everywhere at once and keeps it on this device. */
export function chooseTheme(next: ThemeChoice): void {
  choice = next;
  themeStore.write(next === 'system' ? null : next);
  for (const listener of listeners) listener();
}

export function useScheme(): Scheme {
  const chosen = useThemeChoice();
  const system = useColorScheme();
  if (chosen !== 'system') return chosen;
  return system === 'dark' ? 'dark' : 'light';
}

export function useColors(): Colors {
  return colorsFor(useScheme());
}

/**
 * The bundled fonts, each weight a family of its own, as Android needs: a
 * style names the family and never a `fontWeight` beside it. Loaded before
 * the launch screen goes (`app/_layout.tsx`).
 */
export const fonts = {
  text: 'InstrumentSans-Regular',
  textMedium: 'InstrumentSans-Medium',
  textBold: 'InstrumentSans-Bold',
  textItalic: 'InstrumentSans-Italic',
  display: 'Gabarito-Black',
  displayBold: 'Gabarito-Bold',
} as const;

/** The files behind `fonts`, from the designers' releases, each with its OFL.txt beside it. */
export const FONT_FILES = {
  [fonts.text]: require('../../assets/fonts/instrument-sans/InstrumentSans-Regular.ttf'),
  [fonts.textMedium]: require('../../assets/fonts/instrument-sans/InstrumentSans-Medium.ttf'),
  [fonts.textBold]: require('../../assets/fonts/instrument-sans/InstrumentSans-Bold.ttf'),
  [fonts.textItalic]: require('../../assets/fonts/instrument-sans/InstrumentSans-Italic.ttf'),
  [fonts.display]: require('../../assets/fonts/gabarito/Gabarito-Black.ttf'),
  [fonts.displayBold]: require('../../assets/fonts/gabarito/Gabarito-Bold.ttf'),
};

/** Nothing interactive is smaller than this in either direction. */
export const TOUCH_TARGET = 48;

export const space = { xs: 4, s: 8, m: 12, l: 16, xl: 24, xxl: 32, xxxl: 48 } as const;

export const radius = { s: 10, m: 16, l: 26, xl: 40, pill: 999 } as const;

/**
 * The tilt of the one callout a screen may have, and only the yup card
 * itself: never terms, controls, status or history. Degrees.
 */
export const tilt = { callout: -1.5, calloutAlt: 2 } as const;

export const type = {
  body: { fontFamily: fonts.text, fontSize: 17, lineHeight: 24 },
  hint: { fontFamily: fonts.text, fontSize: 15, lineHeight: 21 },
  title: { fontFamily: fonts.display, fontSize: 26, lineHeight: 32 },
  heading: { fontFamily: fonts.display, fontSize: 21, lineHeight: 27 },
  subheading: { fontFamily: fonts.textBold, fontSize: 18, lineHeight: 24 },
} as const;
