import { api } from './session';

/**
 * Shows the person's payment options on a yup once its terms are signed or
 * sent, if they turned on the switch beside signing (`ShowWhenSigning`).
 * Not part of what is signed: a refusal leaves the signature as it is, and
 * the yup's own switch says where things stand.
 */
export async function showAfterSigning(exchange: string, ticked: boolean): Promise<void> {
  if (!ticked) return;
  try {
    await api.setPaymentOptions(exchange, true);
  } catch {
    // The signature stands either way.
  }
}
