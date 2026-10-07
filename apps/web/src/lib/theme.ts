import { useSyncExternalStore } from 'react'

/*
 * The appearance: System, which follows the device's light or dark setting,
 * or Light or Dark whatever the device says. Chosen per device and kept in
 * this browser only, never with the account: a phone and a laptop often
 * differ, as their own settings do.
 *
 * The choice is `data-theme` on `<html>`, which sets `color-scheme` and with
 * it every `light-dark()` colour (`index.css`). The script that applies it
 * before the first paint reads the same key (`build/theme-script.ts`).
 */

export type ThemeChoice = 'system' | 'light' | 'dark'

export const THEME_CHOICES: readonly ThemeChoice[] = ['system', 'light', 'dark']

const KEY = 'yuppers.theme'

function read(): ThemeChoice {
  const attribute = document.documentElement.getAttribute('data-theme')
  return attribute === 'light' || attribute === 'dark' ? attribute : 'system'
}

const listeners = new Set<() => void>()

function subscribe(listener: () => void): () => void {
  listeners.add(listener)
  return () => listeners.delete(listener)
}

/** The choice in force on this page. */
export function useThemeChoice(): ThemeChoice {
  return useSyncExternalStore(subscribe, read, () => 'system')
}

/** Applies a choice at once and keeps it for this browser, if the browser lets it. */
export function chooseTheme(choice: ThemeChoice): void {
  if (choice === 'system') document.documentElement.removeAttribute('data-theme')
  else document.documentElement.setAttribute('data-theme', choice)
  try {
    if (choice === 'system') localStorage.removeItem(KEY)
    else localStorage.setItem(KEY, choice)
  } catch {
    // Storage refused, as in some private windows: the choice holds for this page.
  }
  for (const listener of listeners) listener()
}
