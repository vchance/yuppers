import { formatPhone } from '@yuppers/shared';
import { useRouter } from 'expo-router';
import { useState } from 'react';
import { StyleSheet, Text, View } from 'react-native';

import { Appearance } from '../components/Appearance';
import { BuildVersion } from '../components/BuildVersion';
import { NotificationsSetting } from '../components/Notifications';
import { LegalLinks } from '../components/LegalLinks';
import { PaymentHandles } from '../components/PaymentHandles';
import { Actions, Button, Heading, Label, Screen } from '../components/ui';
import { useI18n, useSession } from '../lib/context';
import { openHelp } from '../lib/help';
import { APP_BUILD } from '../lib/session';
import { space, type, useColors } from '../lib/theme';
import { ProfileForm } from './AccountSetup';
import { BlockedPeople } from './BlockedPeople';
import { DeleteAccount } from './DeleteAccount';

/**
 * The account: what it is verified with, its name and language, its
 * payment options, notifications, this device's appearance, and signing out.
 */
export function AccountScreen() {
  const { wording, language } = useI18n();
  const { account, signOut } = useSession();
  const colors = useColors();
  const router = useRouter();
  const w = wording.profile;
  const [leaving, setLeaving] = useState(false);

  if (!account) return null;

  async function leave() {
    setLeaving(true);
    // Back to the first screen before the account goes, so what is left on
    // screen is the way to sign in again.
    router.dismissTo('/');
    await signOut();
  }

  return (
    <Screen>
      <Heading>{w.title}</Heading>
      {account.email ? (
        <View style={styles.detail}>
          <Label>{w.emailLabel}</Label>
          <Text selectable style={[type.body, { color: colors.text }]}>
            {account.email}
          </Text>
        </View>
      ) : null}
      {account.phone ? (
        <View style={styles.detail}>
          <Label>{w.phoneLabel}</Label>
          {/* A phone number reads left to right in every language. */}
          <Text selectable style={[type.body, styles.ltr, { color: colors.text }]}>
            {formatPhone(account.phone)}
          </Text>
        </View>
      ) : null}
      <ProfileForm account={account} first={false} />
      <PaymentHandles />
      <NotificationsSetting account={account} />
      <Appearance />
      <BlockedPeople />
      <Actions>
        <Button
          testID="help"
          variant="link"
          label={wording.help.link}
          hint={wording.help.inBrowser}
          onPress={() => void openHelp(language)}
        />
      </Actions>
      <LegalLinks />
      <View style={[styles.rule, { backgroundColor: colors.divider }]} />
      <Actions>
        <Button label={wording.nav.signOut} disabled={leaving} onPress={() => void leave()} />
      </Actions>
      <DeleteAccount account={account} />
      <BuildVersion build={APP_BUILD} />
    </Screen>
  );
}

const styles = StyleSheet.create({
  detail: { gap: space.xs },
  ltr: { writingDirection: 'ltr' },
  rule: { height: StyleSheet.hairlineWidth },
});
