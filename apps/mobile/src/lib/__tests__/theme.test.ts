import { colorsFor, TOUCH_TARGET, type Colors, type Scheme } from '../theme';

/** Relative luminance and contrast ratio, as WCAG 2 defines them. */
function luminance(hex: string): number {
  const channel = (index: number) => {
    const value = parseInt(hex.slice(1 + index * 2, 3 + index * 2), 16) / 255;
    return value <= 0.03928 ? value / 12.92 : ((value + 0.055) / 1.055) ** 2.4;
  };
  return 0.2126 * channel(0) + 0.7152 * channel(1) + 0.0722 * channel(2);
}

function contrast(a: string, b: string): number {
  const [light, dark] = [luminance(a), luminance(b)].sort((x, y) => y - x);
  return (light + 0.05) / (dark + 0.05);
}

// Every pair of text and the surface it is drawn on.
const textOn: [keyof Colors, keyof Colors][] = [
  ['text', 'background'],
  ['text', 'surface'],
  ['text', 'surfaceRaised'],
  ['text', 'noticeSurface'],
  ['text', 'warningSurface'],
  ['text', 'dangerSurface'],
  ['muted', 'background'],
  ['muted', 'surface'],
  ['muted', 'surfaceRaised'],
  ['muted', 'warningSurface'],
  ['muted', 'noticeSurface'],
  ['link', 'background'],
  ['link', 'surface'],
  ['link', 'surfaceRaised'],
  ['partyThemText', 'surface'],
  ['partyThemText', 'noticeSurface'],
  ['onPrimary', 'primary'],
  ['onPartyYou', 'partyYou'],
  ['onPartyThem', 'partyThem'],
  ['onStrong', 'surfaceStrong'],
  ['onStrongMuted', 'surfaceStrong'],
  ['onStatusWaiting', 'statusWaiting'],
  ['onStatusDisputed', 'statusDisputed'],
  ['onStatusConfirmed', 'statusConfirmed'],
  ['danger', 'background'],
  ['danger', 'surface'],
  ['danger', 'surfaceRaised'],
];

// Shapes that carry meaning without text, against what is around them
// (WCAG 1.4.11): the overlap of the mark, the checks.
const shapesOn: [keyof Colors, keyof Colors][] = [
  ['agree', 'background'],
  ['onAgree', 'agree'],
  ['statusConfirmedIcon', 'statusConfirmed'],
  ['text', 'surface'],
];

test.each<Scheme>(['light', 'dark'])('text is readable in the %s palette', (scheme) => {
  const colors = colorsFor(scheme);
  for (const [text, surface] of textOn) {
    const ratio = contrast(colors[text], colors[surface]);
    // WCAG AA for body text.
    expect(`${text} on ${surface}: ${ratio >= 4.5 ? 'ok' : ratio.toFixed(2)}`).toBe(
      `${text} on ${surface}: ok`,
    );
  }
});

test.each<Scheme>(['light', 'dark'])('control outlines can be seen in the %s palette', (scheme) => {
  const colors = colorsFor(scheme);
  // WCAG 1.4.11: the boundary of a control against what is around it.
  expect(contrast(colors.border, colors.background)).toBeGreaterThanOrEqual(3);
  expect(contrast(colors.border, colors.surface)).toBeGreaterThanOrEqual(3);
  expect(contrast(colors.border, colors.surfaceRaised)).toBeGreaterThanOrEqual(3);
  expect(contrast(colors.primary, colors.background)).toBeGreaterThanOrEqual(3);
});

test.each<Scheme>(['light', 'dark'])('shapes that mean something can be seen in the %s palette', (scheme) => {
  const colors = colorsFor(scheme);
  for (const [shape, surface] of shapesOn) {
    const ratio = contrast(colors[shape], colors[surface]);
    expect(`${shape} on ${surface}: ${ratio >= 3 ? 'ok' : ratio.toFixed(2)}`).toBe(
      `${shape} on ${surface}: ok`,
    );
  }
});

test('touch targets are at least as large as both platforms ask', () => {
  // 44 points on iOS, 48 density-independent pixels on Android.
  expect(TOUCH_TARGET).toBeGreaterThanOrEqual(48);
});
