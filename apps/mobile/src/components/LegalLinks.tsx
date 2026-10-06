import type { LegalDocument } from '@yuppers/shared';

import { useI18n } from '../lib/context';
import { openLegal } from '../lib/legal';
import { Actions, Button } from './ui';

/**
 * Small links to the privacy policy and the terms, or to one section of one
 * of them, opened in the browser, like "Learn more" (`HelpLink`). The hint
 * says each leaves the app.
 */
export function LegalLinks({
  documents = ['privacy', 'terms'],
  section,
  label,
  testID,
}: {
  documents?: LegalDocument[];
  section?: string;
  label?: string;
  testID?: string;
}) {
  const { wording, language } = useI18n();
  const names: Record<LegalDocument, string> = {
    privacy: wording.privacy.policy,
    terms: wording.termsOfUse.document,
  };
  return (
    <Actions>
      {documents.map((document) => (
        <Button
          key={document}
          testID={testID ?? document}
          variant="link"
          label={label ?? names[document]}
          hint={wording.help.inBrowser}
          onPress={() => void openLegal(document, language, section)}
        />
      ))}
    </Actions>
  );
}
