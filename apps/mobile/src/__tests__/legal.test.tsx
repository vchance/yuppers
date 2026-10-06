import { wordingFor } from '@yuppers/shared';
import { fireEvent, renderRouter, screen, waitFor } from 'expo-router/testing-library';
import { Linking } from 'react-native';

import { WEB_URL } from '../lib/config';
import { legalUrl } from '../lib/legal';
import { fakeService, TOKEN, ana, type FakeService } from './fake-service';

/*
 * The ways to the privacy policy and the terms from the app, as iOS and as
 * Android builds would run it: each is the web app's page, opened in the
 * system's browser, in the app's language, at the section the place is
 * about.
 */

const mockKeychain = new Map<string, string>();
jest.mock('expo-secure-store', () => ({
  WHEN_UNLOCKED_THIS_DEVICE_ONLY: 'when-unlocked-this-device-only',
  getItemAsync: jest.fn(async (key: string) => mockKeychain.get(key) ?? null),
  setItemAsync: jest.fn(async (key: string, value: string) => {
    mockKeychain.set(key, value);
  }),
  deleteItemAsync: jest.fn(async (key: string) => {
    mockKeychain.delete(key);
  }),
}));

jest.mock('@react-native-community/datetimepicker', () => ({
  __esModule: true,
  default: () => null,
  DateTimePickerAndroid: { open: jest.fn(), dismiss: jest.fn() },
}));

let service: FakeService = fakeService();
globalThis.fetch = ((...args: Parameters<typeof fetch>) => service.fetch(...args)) as typeof fetch;

const opened = jest.spyOn(Linking, 'openURL').mockResolvedValue(true);
const web = WEB_URL.replace(/\/+$/, '');

async function open(
  initialUrl: string,
  { language = 'en', signedIn = true, phone = true } = {},
) {
  mockKeychain.clear();
  opened.mockClear();
  service = fakeService();
  service.phone = phone;
  if (signedIn) {
    mockKeychain.set('yuppers.session', TOKEN);
    service.account = { ...ana, language };
  }
  await renderRouter('src/app', { initialUrl });
}

test('the address names the web app, the document, the language and the section', () => {
  expect(legalUrl('privacy', 'en')).toBe(`${web}/privacy`);
  expect(legalUrl('terms', 'es')).toBe(`${web}/es/terms`);
  expect(legalUrl('privacy', 'en', 'text-messages')).toBe(`${web}/privacy#text-messages`);
});

test('the account screen’s links open the policy and the terms in the browser', async () => {
  const w = wordingFor('en');
  await open('/account');
  const privacy = await screen.findByRole('link', { name: w.privacy.policy });
  expect(privacy.props.accessibilityHint).toBe(w.help.inBrowser);
  await fireEvent.press(privacy);
  await waitFor(() => expect(opened).toHaveBeenCalledWith(`${web}/privacy`));
  await fireEvent.press(await screen.findByRole('link', { name: w.termsOfUse.document }));
  await waitFor(() => expect(opened).toHaveBeenCalledWith(`${web}/terms`));
});

test('in Spanish, it opens them in Spanish', async () => {
  const w = wordingFor('es');
  await open('/account', { language: 'es' });
  await fireEvent.press(await screen.findByRole('link', { name: w.privacy.policy }));
  await waitFor(() => expect(opened).toHaveBeenCalledWith(`${web}/es/privacy`));
  await fireEvent.press(await screen.findByRole('link', { name: w.termsOfUse.document }));
  await waitFor(() => expect(opened).toHaveBeenCalledWith(`${web}/es/terms`));
});

test('deleting the account links to what stays', async () => {
  const w = wordingFor('en');
  await open('/account');
  await fireEvent.press(await screen.findByRole('link', { name: w.privacy.deletion }));
  await waitFor(() =>
    expect(opened).toHaveBeenCalledWith(`${web}/privacy#deleting-your-account`),
  );
});

test('signing in, where codes go to phone numbers, says what texts cost and links to the section on them', async () => {
  const w = wordingFor('en');
  await open('/', { signedIn: false });
  expect(await screen.findByText(w.privacy.sms)).toBeTruthy();
  await fireEvent.press(await screen.findByRole('link', { name: w.privacy.smsLink }));
  await waitFor(() => expect(opened).toHaveBeenCalledWith(`${web}/privacy#text-messages`));
  await fireEvent.press(await screen.findByRole('link', { name: w.privacy.policy }));
  await waitFor(() => expect(opened).toHaveBeenCalledWith(`${web}/privacy`));
  await fireEvent.press(await screen.findByRole('link', { name: w.termsOfUse.document }));
  await waitFor(() => expect(opened).toHaveBeenCalledWith(`${web}/terms`));
});

test('where codes go by email only, signing in says nothing of texts', async () => {
  const w = wordingFor('en');
  await open('/', { signedIn: false, phone: false });
  await screen.findByRole('link', { name: w.privacy.policy });
  await waitFor(() => expect(screen.queryByText(w.signIn.introEmail)).toBeTruthy());
  expect(screen.queryByText(w.privacy.sms)).toBeNull();
});
