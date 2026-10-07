import { File, Paths } from 'expo-file-system';

import type { ThemeStore } from './theme-store.types';

/*
 * The appearance chosen on this device (`theme.ts`), in a small file in the
 * app's own documents. It is not secret, so it is not in the keychain with
 * the session, and it is read synchronously, as the app starts and while
 * the launch screen is still up, so the first screen is drawn in it.
 */

const file = () => new File(Paths.document, 'appearance.txt');

export const themeStore: ThemeStore = {
  read() {
    try {
      const stored = file();
      return stored.exists ? stored.textSync() : null;
    } catch {
      return null;
    }
  },
  write(value) {
    try {
      const stored = file();
      if (value === null) {
        if (stored.exists) stored.delete();
      } else {
        stored.write(value);
      }
    } catch {
      // Not kept: the choice still holds until the app is closed.
    }
  },
};
