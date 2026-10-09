import { createI18n, wordingFor } from '@yuppers/shared';
import { fireEvent, render, screen } from '@testing-library/react-native';
import * as Clipboard from 'expo-clipboard';
import { Platform, Share } from 'react-native';

import { readQr } from '../../__tests__/read-qr';
import { WEB_URL } from '../../lib/config';
import { I18nContext } from '../../lib/context';
import { InvitationLink } from '../InvitationLink';

/*
 * The invitation link on the app (DESIGN.md §8): the system's share sheet
 * first, copying beside it, and a QR code for someone in the same room to
 * scan. The service sends nothing, and the screen says so.
 */

jest.mock('expo-secure-store', () => ({}));
jest.mock('expo-crypto', () => ({ randomUUID: () => '00000000-0000-4000-8000-000000000000' }));
jest.mock('expo-clipboard', () => ({ setStringAsync: jest.fn(async () => true) }));

const wording = wordingFor('en');
const w = wording.invitationLink;
const i18n = createI18n('en', wording, () => {});
const TOKEN = 'Tok3n_with-dashes_0123456789abcdefghij';
const LINK = `${WEB_URL.replace(/\/+$/, '')}/en/i#${TOKEN}`;

function show() {
  return render(
    <I18nContext value={i18n}>
      <InvitationLink token={TOKEN} boundTo={null} />
    </I18nContext>,
  );
}

afterEach(() => {
  jest.restoreAllMocks();
});

test('it says, where the link is, that sending it is the person’s own job', async () => {
  await show();
  screen.getByText(
    'Yuppers doesn’t send this for you. Share the link with them, and they can read the proposal after signing in.',
  );
  screen.getByText(w.shownOnce);
  expect(w.shownOnce).toContain(w.reissue);
  screen.getByText(LINK);
});

test('“Share link” opens the system’s share sheet, with the link and fixed wording', async () => {
  const share = jest.spyOn(Share, 'share').mockResolvedValue({ action: 'sharedAction' });
  await show();
  await fireEvent.press(screen.getByRole('button', { name: w.share }));
  expect(share).toHaveBeenCalledTimes(1);
  if (Platform.OS === 'android') {
    // Android's sheet takes text only: the link goes in the message.
    expect(share).toHaveBeenCalledWith({
      title: wording.linkPreview.title,
      message: `I’ve sent you a yup to review: ${LINK}`,
    });
  } else {
    expect(share).toHaveBeenCalledWith({
      title: wording.linkPreview.title,
      message: w.shareText,
      url: LINK,
    });
  }
});

test('copying is beside it', async () => {
  await show();
  await fireEvent.press(screen.getByRole('button', { name: w.copy }));
  expect(Clipboard.setStringAsync).toHaveBeenCalledWith(LINK);
  await screen.findByText(w.copied);
});

test('the QR code, drawn on the device, reads back as the link', async () => {
  await show();
  expect(screen.queryByTestId('qr-code')).toBeNull();
  const toggle = screen.getByRole('button', { name: w.shareQr });
  expect(toggle.props.accessibilityState).toMatchObject({ expanded: false });
  await fireEvent.press(toggle);

  const code = screen.getByTestId('qr-code');
  expect(code.props.accessibilityRole).toBe('image');
  expect(code.props.accessibilityLabel).toBe(w.qrLabel);
  expect(readQr(code as never)).toBe(LINK);
  screen.getByText(w.qrHint);
  expect(screen.getByRole('button', { name: w.hideQr }).props.accessibilityState).toMatchObject({
    expanded: true,
  });

  await fireEvent.press(screen.getByRole('button', { name: w.hideQr }));
  expect(screen.queryByTestId('qr-code')).toBeNull();
});
