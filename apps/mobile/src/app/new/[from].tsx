import { useLocalSearchParams } from 'expo-router';

import { Gate } from '../../screens/AccountSetup';
import { NewDraftScreen } from '../../screens/ExchangeScreen';

// A fresh start, before the service has an exchange for it: `/new/blank` or
// `/new/{template}`. It moves to `/exchanges/{id}` on the first change.
export default function NewDraft() {
  const { from } = useLocalSearchParams<{ from: string }>();
  return (
    <Gate>
      <NewDraftScreen key={from} from={from} />
    </Gate>
  );
}
