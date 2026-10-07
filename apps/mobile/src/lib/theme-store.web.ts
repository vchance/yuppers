import type { ThemeStore } from './theme-store.types';

/*
 * TEST HARNESS ONLY, like `token-store.web.ts`: the appearance chosen in the
 * browser the app's screens are exercised in, under the key the web app
 * uses (`apps/web/src/lib/theme.ts`).
 */

const KEY = 'yuppers.theme';

export const themeStore: ThemeStore = {
  read() {
    try {
      return window.localStorage.getItem(KEY);
    } catch {
      return null;
    }
  },
  write(value) {
    try {
      if (value === null) window.localStorage.removeItem(KEY);
      else window.localStorage.setItem(KEY, value);
    } catch {
      // Not kept: the choice still holds for this page.
    }
  },
};
