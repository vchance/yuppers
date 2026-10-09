import { wordingFor } from '@yuppers/shared';
import { fireEvent, renderRouter, screen } from 'expo-router/testing-library';

import { forgetInvitation } from '../lib/invitation';
import { fakeService, INVITATION, TOKEN, ana, type FakeService } from './fake-service';

/*
 * The brand on the screens that stand for the product itself, run as iOS
 * and as Android builds: signing in and setting up an account open with the
 * mark and the wordmark, and the list of yups with none in it shows the mark
 * with what to do. The mark is decorative and hidden; the mark and the word
 * together are one image named for the product; neither is a heading, so
 * each screen keeps its one level 1 heading.
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
jest.mock('expo-application', () => ({ nativeApplicationVersion: '1.2.0' }));
jest.mock('expo-crypto', () => {
  let next = 0;
  return { randomUUID: () => `00000000-0000-4000-8000-${String((next += 1)).padStart(12, '0')}` };
});

let service: FakeService = fakeService();
globalThis.fetch = ((...args: Parameters<typeof fetch>) => service.fetch(...args)) as typeof fetch;

const w = wordingFor('en');

async function open(
  initialUrl: string,
  { signedIn, prepare }: { signedIn: boolean; prepare?: (service: FakeService) => void },
) {
  mockKeychain.clear();
  service = fakeService();
  if (signedIn) {
    mockKeychain.set('yuppers.session', TOKEN);
    service.account = ana;
  }
  prepare?.(service);
  await renderRouter('src/app', { initialUrl });
}

afterEach(forgetInvitation);

/** The mark and wordmark together, as a screen reader meets them: one image, named for the product. */
function expectBrand() {
  const image = screen.getByRole('image', { name: w.productName });
  expect(image.props.accessible).toBe(true);
  // The mark is in it, hidden from the screen reader, as is the word: the
  // image's own name says it.
  const marks = screen.getAllByTestId('mark', { includeHiddenElements: true });
  expect(marks.length).toBeGreaterThanOrEqual(1);
  expect(marks[0].props.accessibilityElementsHidden).toBe(true);
  expect(screen.getByText(w.productName, { includeHiddenElements: true }).props.style).toEqual(
    expect.objectContaining({ textTransform: 'lowercase' }),
  );
  // What Yuppers is, in words a screen reader does read.
  expect(screen.getByText(w.tagline)).toBeTruthy();
  // Not a heading: the screen's own heading is the one at level 1.
  const top = screen.getAllByRole('header').filter((node) => node.props['aria-level'] === 1);
  expect(top).toHaveLength(1);
}

describe('signing in', () => {
  test('opens with the mark, the wordmark and what Yuppers is', async () => {
    await open('/', { signedIn: false });
    await screen.findByText(w.signIn.intro);
    expectBrand();
    expect(screen.getByRole('header', { name: w.signIn.title }).props['aria-level']).toBe(1);
  });

  test('keeps them through the code step and the profile', async () => {
    await open('/', { signedIn: false });
    await screen.findByText(w.signIn.intro);
    await fireEvent.changeText(screen.getByLabelText(w.signIn.identifierLabel), 'ana@example.test');
    await fireEvent.press(screen.getByRole('button', { name: w.signIn.sendCode }));
    await screen.findByLabelText(w.signIn.codeLabel);
    expectBrand();

    await fireEvent.changeText(screen.getByLabelText(w.signIn.codeLabel), '123456');
    await fireEvent.press(screen.getByRole('button', { name: w.signIn.submit }));
    await screen.findByText(w.profile.firstIntro);
    expectBrand();
    expect(screen.getByRole('header', { name: w.profile.firstTitle }).props['aria-level']).toBe(1);
  });

  test('below an invitation, which has a heading of its own, opens plain', async () => {
    await open(`/en/i#${INVITATION}`, { signedIn: false });
    await screen.findByText(w.invitation.signInToRead);
    expect(screen.getByRole('header', { name: w.invitation.signedOutTitle })).toBeTruthy();
    expect(screen.queryByRole('image', { name: w.productName })).toBeNull();
    expect(screen.queryByText(w.tagline)).toBeNull();
  });
});

describe('the list of yups', () => {
  test('with none yet: the mark, what to do and the ways to start, together', async () => {
    await open('/', {
      signedIn: true,
      prepare: (fake) => {
        fake.noExchanges = true;
      },
    });
    await screen.findByText(w.home.empty);
    expect(screen.getAllByTestId('mark', { includeHiddenElements: true }).length).toBeGreaterThanOrEqual(1);
    // The one way to start, and the way to an invitation, are in it.
    expect(screen.getAllByRole('button', { name: w.home.start })).toHaveLength(1);
    expect(screen.getAllByRole('button', { name: w.mobile.openInvitation.title })).toHaveLength(1);
    expect(screen.queryByText(w.tagline)).toBeNull();
  });

  test('with yups, there is no empty state', async () => {
    await open('/', { signedIn: true });
    await screen.findByText(w.home.groupOpen);
    expect(screen.queryByText(w.home.empty)).toBeNull();
    expect(screen.getAllByRole('button', { name: w.home.start })).toHaveLength(1);
  });
});
