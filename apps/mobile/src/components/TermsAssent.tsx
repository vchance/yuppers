import { termsAssentPieces } from '@yuppers/shared';
import { StyleSheet, Text } from 'react-native';

import { useI18n } from '../lib/context';
import { openLegal } from '../lib/legal';
import { type, useColors } from '../lib/theme';

/**
 * The sentence above the button that signs in: continuing is the assent to
 * the Terms and the Privacy policy (`signIn.agreement`), as on the web
 * (`apps/web/src/components/TermsAssent.tsx`). The names of the two
 * documents are links that open them in the browser. No box.
 */
export function TermsAssent() {
  const { wording, language } = useI18n();
  const colors = useColors();
  const names = { terms: wording.termsOfUse.link, privacy: wording.privacy.policy };
  return (
    <Text
      testID="terms-assent"
      accessibilityLanguage={language}
      style={[type.body, { color: colors.text }]}>
      {termsAssentPieces(wording.signIn.agreement, names).map((piece, index) =>
        'document' in piece ? (
          <Text
            key={index}
            testID={`terms-assent-${piece.document}`}
            accessibilityRole="link"
            accessibilityHint={wording.help.inBrowser}
            onPress={() => void openLegal(piece.document, language)}
            style={[styles.link, { color: colors.link }]}>
            {piece.text}
          </Text>
        ) : (
          piece.text
        ),
      )}
    </Text>
  );
}

const styles = StyleSheet.create({
  link: { textDecorationLine: 'underline' },
});
