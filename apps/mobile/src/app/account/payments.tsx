import { Gate } from '../../screens/AccountSetup';
import { PaymentOptionsScreen } from '../../screens/PaymentOptionsScreen';

// The same address as on the web, `/account/payments`: the account's payment options.
export default function AccountPayments() {
  return (
    <Gate>
      <PaymentOptionsScreen />
    </Gate>
  );
}
