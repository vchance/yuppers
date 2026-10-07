import { createElement } from 'react';

import { useColors } from '../lib/theme';
import type { DateFieldProps } from './DateField';
import { Field } from './ui';

/*
 * TEST HARNESS ONLY, like `token-store.web.ts`. The native date picker has no
 * web implementation, so when the app is run in a browser to exercise its
 * screens, the browser's own date input stands in. The bundler resolves this
 * file for the web target alone; iOS and Android get `DateField.tsx`.
 */
export function DateField({ label, hint, value, onChange, error, disabled }: DateFieldProps) {
  const colors = useColors();
  return (
    <Field label={label} hint={hint} error={error} labelled>
      {createElement('input', {
        type: 'date',
        'aria-label': label,
        value,
        disabled,
        onChange: (event: { target: { value: string } }) => onChange(event.target.value),
        style: {
          minHeight: 48,
          fontSize: 17,
          padding: 12,
          borderRadius: 16,
          border: `2px solid ${error ? colors.danger : colors.border}`,
          color: colors.text,
          backgroundColor: colors.surface,
          alignSelf: 'flex-start',
        },
      })}
    </Field>
  );
}
