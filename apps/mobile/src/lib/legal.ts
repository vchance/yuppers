import { legalAddress, type LegalDocument } from '@yuppers/shared';
import { Linking } from 'react-native';

import { WEB_URL } from './config';

/*
 * The privacy policy and the terms are the web app's: `/privacy` and
 * `/terms`, and `/{language}/privacy` and `/{language}/terms` in every
 * language but the default, on the web origin. The app does not carry them;
 * it opens them in the system's browser, as it does help (`help.ts`). The
 * address names the app's language, and is a page the service writes in
 * full, so it reads in that language whether or not the browser runs its
 * scripts.
 */

/** The address of a document, or of one of its sections, in `language`. */
export function legalUrl(document: LegalDocument, language: string, section?: string): string {
  return legalAddress(WEB_URL, document, language, section);
}

/** Opens a document, or one of its sections, in the browser. */
export function openLegal(
  document: LegalDocument,
  language: string,
  section?: string,
): Promise<void> {
  return Linking.openURL(legalUrl(document, language, section)).catch(() => {
    // Only a device without any browser refuses an https address, and on it
    // there is nothing better to offer.
  });
}
