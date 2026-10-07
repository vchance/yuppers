import { createI18n, wordingFor } from '@yuppers/shared';
import { act, fireEvent, render, screen } from '@testing-library/react-native';
import { Text } from 'react-native';

import { I18nContext } from '../../lib/context';
import { chooseTheme, colorsFor, useColors, useScheme } from '../../lib/theme';
import { themeStore } from '../../lib/theme-store';
import { Appearance } from '../Appearance';

/*
 * The appearance switch on the account screen: System, Light or Dark for this
 * device, applied everywhere at once and kept in device storage, never with
 * the account.
 */

jest.mock('../../lib/theme-store', () => {
  let stored: string | null = null;
  return {
    themeStore: {
      read: jest.fn(() => stored),
      write: jest.fn((value: string | null) => {
        stored = value;
      }),
    },
  };
});

const en = wordingFor('en');

afterEach(async () => {
  await act(async () => chooseTheme('system'));
});

/** What the rest of the app would draw in. */
function Probe() {
  const scheme = useScheme();
  const colors = useColors();
  return <Text testID="probe">{`${scheme} ${colors === colorsFor(scheme)}`}</Text>;
}

async function show() {
  const i18n = createI18n('en', en, () => {});
  await render(
    <I18nContext.Provider value={i18n}>
      <Appearance />
      <Probe />
    </I18nContext.Provider>,
  );
}

test('offers System, Light and Dark as radio buttons, System chosen to begin with', async () => {
  await show();
  expect(screen.getByRole('header', { name: en.appearance.heading })).toBeTruthy();
  const options = screen.getAllByRole('radio');
  expect(options.map((option) => option.props.accessibilityLabel)).toEqual([
    en.appearance.system,
    en.appearance.light,
    en.appearance.dark,
  ]);
  expect(screen.getByRole('radio', { name: en.appearance.system }).props.accessibilityState).toEqual({
    checked: true,
  });
});

test('a choice applies at once, overrides the system, and is kept on the device', async () => {
  await show();
  await act(async () => fireEvent.press(screen.getByRole('radio', { name: en.appearance.dark })));
  expect(screen.getByTestId('probe').props.children).toBe('dark true');
  expect(themeStore.write).toHaveBeenLastCalledWith('dark');
  expect(screen.getByRole('radio', { name: en.appearance.dark }).props.accessibilityState).toEqual({
    checked: true,
  });

  await act(async () => fireEvent.press(screen.getByRole('radio', { name: en.appearance.system })));
  // Jest's system setting is light.
  expect(screen.getByTestId('probe').props.children).toBe('light true');
  expect(themeStore.write).toHaveBeenLastCalledWith(null);
});
