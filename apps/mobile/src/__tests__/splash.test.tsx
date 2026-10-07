import { wordingFor } from '@yuppers/shared';
import { act, renderHook, renderRouter, screen } from 'expo-router/testing-library';
import * as SplashScreen from 'expo-splash-screen';
import * as SystemUI from 'expo-system-ui';

import { SPLASH_LIMIT_MS, useSplashUntil } from '../lib/splash';
import { colorsFor } from '../lib/theme';
import { TOKEN, ana, fakeService, type FakeService } from './fake-service';

/*
 * The launch screen: held from the moment the app loads, and hidden once the
 * app knows who is signed in, with the window behind the screens painted the
 * app's own background, so that nothing white shows in between.
 */

jest.mock('expo-splash-screen', () => ({
  preventAutoHideAsync: jest.fn(async () => true),
  hide: jest.fn(),
}));
jest.mock('expo-system-ui', () => ({ setBackgroundColorAsync: jest.fn(async () => {}) }));

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
jest.mock('expo-application', () => ({ nativeApplicationVersion: '0.1.0' }));
jest.mock('expo-crypto', () => ({ randomUUID: () => '00000000-0000-4000-8000-000000000000' }));

let service: FakeService = fakeService();
/** Holds back the service's answer about who is signed in, until released. */
let holdMe: Promise<void> | null = null;
globalThis.fetch = (async (...args: Parameters<typeof fetch>) => {
  const request = args[0] instanceof Request ? args[0] : new Request(args[0], args[1]);
  if (holdMe && new URL(request.url).pathname === '/v1/me') await holdMe;
  return service.fetch(...args);
}) as typeof fetch;

const w = wordingFor('en');
const hide = jest.mocked(SplashScreen.hide);

beforeEach(() => {
  hide.mockClear();
  mockKeychain.clear();
  service = fakeService();
});

test('the launch screen stays until the app knows who is signed in, then the screen is there', async () => {
  mockKeychain.set('yuppers.session', TOKEN);
  service.account = ana;
  let release = () => {};
  holdMe = new Promise((resolve) => {
    release = resolve;
  });

  await renderRouter('src/app', { initialUrl: '/' });
  expect(SplashScreen.preventAutoHideAsync).toHaveBeenCalled();
  await act(async () => {});
  expect(hide).not.toHaveBeenCalled();

  release();
  await screen.findByText(w.home.title);
  expect(hide).toHaveBeenCalled();
  holdMe = null;
});

test('the window behind the screens takes the app background', async () => {
  await renderRouter('src/app', { initialUrl: '/' });
  await screen.findByText(w.signIn.intro);
  expect(SystemUI.setBackgroundColorAsync).toHaveBeenCalledWith(colorsFor('light').background);
  expect(hide).toHaveBeenCalled();
});

test('a service that does not answer does not keep the launch screen up', async () => {
  jest.useFakeTimers();
  try {
    const { rerender } = await renderHook(({ ready }: { ready: boolean }) => useSplashUntil(ready), {
      initialProps: { ready: false },
    });
    await act(async () => jest.advanceTimersByTime(SPLASH_LIMIT_MS - 1));
    expect(hide).not.toHaveBeenCalled();
    await act(async () => jest.advanceTimersByTime(1));
    expect(hide).toHaveBeenCalledTimes(1);

    await rerender({ ready: true });
    expect(hide).toHaveBeenCalledTimes(2);
  } finally {
    jest.useRealTimers();
  }
});
