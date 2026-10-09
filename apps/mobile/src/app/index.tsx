import { isComplete } from '@yuppers/shared';
import { Stack, useRouter } from 'expo-router';

import { AccountDeleted } from '../components/AccountDeleted';
import { BrandTitle } from '../components/Brand';
import { Button } from '../components/ui';
import { useI18n, useSession } from '../lib/context';
import { Gate } from '../screens/AccountSetup';
import { HomeScreen, InvitedEntry } from '../screens/HomeScreen';

/**
 * The first screen: your exchanges, or the way to sign in and the way to an
 * invitation. Signed in, the bar carries the brand small and the way to the
 * account; before that there is no bar, and the screen opens with the brand
 * large, as the launch screen it follows did (`BrandHero`).
 */
export default function Home() {
  const { wording } = useI18n();
  const { ready, failure, account } = useSession();
  const router = useRouter();
  const able = account !== null && isComplete(account);
  // Signing in, or setting up a new account: the screen opens with the brand.
  const settingUp = ready && !failure && !able;

  return (
    <>
      <Stack.Screen
        options={{
          headerShown: !settingUp,
          // An element, not the component: the bar calls `headerTitle` as a
          // plain function, so a component's hooks would run as its own.
          ...(able ? { headerTitle: () => <BrandTitle /> } : {}),
          headerRight: able
            ? () => (
                <Button
                  variant="link"
                  label={wording.nav.account}
                  onPress={() => router.push('/account')}
                />
              )
            : undefined,
        }}
      />
      <AccountDeleted />
      <Gate signedOut={<InvitedEntry />}>
        <HomeScreen />
      </Gate>
    </>
  );
}
