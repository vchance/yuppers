/// <reference types="node" />
import { readFileSync } from 'node:fs'

import { describe, expect, test } from 'vitest'

/*
 * The colours in `index.css`, checked against WCAG 2.2: 4.5:1 for text
 * (1.4.3) and 3:1 for the edges of controls and the focus ring (1.4.11), in
 * the light palette and the dark one. axe cannot judge contrast in jsdom,
 * which paints nothing, so the pairs the stylesheet actually draws are
 * listed here and computed from the tokens.
 */

const css = readFileSync(new URL('./index.css', import.meta.url), 'utf8')

type Scheme = 'light' | 'dark'

/** Each `--token: light-dark(#light, #dark)` in the stylesheet. */
const tokens = new Map<string, Record<Scheme, string>>()
for (const [, name, light, dark] of css.matchAll(
  /--([\w-]+):\s*light-dark\((#[0-9a-f]{6}),\s*(#[0-9a-f]{6})\)/gi,
)) {
  tokens.set(name, { light, dark })
}

function luminance(hex: string): number {
  const channel = (index: number) => {
    const value = parseInt(hex.slice(1 + index * 2, 3 + index * 2), 16) / 255
    return value <= 0.03928 ? value / 12.92 : ((value + 0.055) / 1.055) ** 2.4
  }
  return 0.2126 * channel(0) + 0.7152 * channel(1) + 0.0722 * channel(2)
}

function contrast(a: string, b: string): number {
  const [lighter, darker] = [luminance(a), luminance(b)].sort((x, y) => y - x)
  return (lighter + 0.05) / (darker + 0.05)
}

// Text and what it is drawn on.
const text: [string, string][] = [
  ['text', 'page'],
  ['text', 'raised'], // cards, buttons, fields
  ['text', 'surface-raised'], // panels, notices, the both-signed moment
  ['text', 'alert-surface'],
  ['text', 'warning-surface'], // notices, your entries in the history
  ['text', 'them-surface'], // their entries in the history, "Done when"
  ['muted', 'page'], // hints
  ['muted', 'raised'], // hints on a card
  ['muted', 'surface-raised'], // hints in a panel
  ['muted', 'warning-surface'], // the time of your entry in the history
  ['muted', 'them-surface'], // the time of theirs
  ['link', 'page'], // links
  ['link', 'raised'], // links and link buttons on a card
  ['link', 'surface-raised'], // links in a panel
  ['them-text', 'raised'], // their name as text
  ['them-text', 'them-surface'],
  ['on-accent', 'accent'], // the primary button, the chosen appearance
  ['on-you', 'you'], // your side of the terms, the summary card
  ['on-them', 'them'], // theirs
  ['on-strong', 'surface-strong'],
  ['on-strong-muted', 'surface-strong'],
  ['on-waiting', 'waiting'], // status chips
  ['on-disputed', 'disputed'],
  ['on-confirmed', 'confirmed'],
  ['alert', 'page'], // field errors, the overdue tag
  ['alert', 'raised'], // field errors on a card
  ['alert', 'surface-raised'], // field errors in a panel
]

// The edge of a control, or the focus ring, against what is around it, and
// the shapes that carry meaning without text.
const edges: [string, string][] = [
  ['line', 'page'],
  ['line', 'raised'],
  ['line', 'surface-raised'],
  ['text', 'page'], // buttons' edges
  ['text', 'raised'],
  ['accent', 'page'], // the primary button
  ['accent', 'raised'],
  ['focus', 'page'], // the focus ring
  ['focus', 'raised'],
  ['focus', 'surface-raised'],
  ['alert', 'page'], // a field marked invalid
  ['alert', 'raised'],
  ['agree', 'page'], // the overlap of the mark
  ['on-agree', 'agree'], // the check on it
  ['confirmed-icon', 'confirmed'], // the check on a confirmed chip
  ['text', 'raised'], // a not-yet chip's outline
]

describe.each<Scheme>(['light', 'dark'])('the %s palette', (scheme) => {
  test('has every token it is checked for', () => {
    for (const [a, b] of [...text, ...edges]) {
      expect(tokens.has(a), a).toBe(true)
      expect(tokens.has(b), b).toBe(true)
    }
  })

  test('text is at least 4.5:1', () => {
    const below = text
      .map(
        ([fore, back]) =>
          [fore, back, contrast(tokens.get(fore)![scheme], tokens.get(back)![scheme])] as const,
      )
      .filter(([, , ratio]) => ratio < 4.5)
      .map(([fore, back, ratio]) => `${fore} on ${back}: ${ratio.toFixed(2)}`)
    expect(below).toEqual([])
  })

  test('controls and focus are at least 3:1', () => {
    const below = edges
      .map(
        ([fore, back]) =>
          [fore, back, contrast(tokens.get(fore)![scheme], tokens.get(back)![scheme])] as const,
      )
      .filter(([, , ratio]) => ratio < 3)
      .map(([fore, back, ratio]) => `${fore} against ${back}: ${ratio.toFixed(2)}`)
    expect(below).toEqual([])
  })
})
