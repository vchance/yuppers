import {
  addedApps,
  appsToAdd,
  handleShown,
  phoneAsTyped,
  usePaymentOptions,
  type PaymentApp,
  type PaymentHandlesApi,
  type PaymentOptionsScreen as Screenful,
} from '@yuppers/shared';
import { useEffect, useRef } from 'react';
import { Modal, StyleSheet, Text, View, type TextInputProps } from 'react-native';
import { useSafeAreaInsets } from 'react-native-safe-area-context';

import {
  Actions,
  Button,
  Card,
  Failure,
  Heading,
  Hint,
  Notice,
  P,
  Screen,
  TextField,
} from '../components/ui';
import { focusOn } from '../lib/accessibility';
import { useI18n } from '../lib/context';
import { api } from '../lib/session';
import { radius, space, type, useColors } from '../lib/theme';

/** The wording keys of each app's field, in `payments`. */
const FIELD = {
  venmo: { label: 'venmoLabel', hint: 'venmoHint', invalid: 'venmoInvalid' },
  cash_app: { label: 'cashAppLabel', hint: 'cashAppHint', invalid: 'cashAppInvalid' },
  paypal: { label: 'paypalLabel', hint: 'paypalHint', invalid: 'paypalInvalid' },
  zelle: { label: 'zelleLabel', hint: 'zelleHint', invalid: 'zelleInvalid' },
} as const;

/** The keyboard for each: none is a sign-in, and only Zelle may be an address. */
const KEYBOARD: Record<PaymentApp, TextInputProps['keyboardType']> = {
  venmo: 'default',
  cash_app: 'default',
  paypal: 'default',
  zelle: 'email-address',
};

interface Props {
  /** For tests: the service's calls. */
  client?: PaymentHandlesApi;
}

/**
 * The payment options screen, as on the web
 * (`apps/web/src/screens/PaymentOptionsPage.tsx`): the options added, each
 * with Edit and Remove; "Add a payment option" asks which app, from those
 * not added yet, then shows that app's one field. Removing asks first, in
 * a modal sheet, and says when it is the last one, which stops showing
 * them on every yup. A screen reader is taken to each step's heading as it
 * opens, and back to the button it came from when a step is cancelled.
 */
export function PaymentOptionsScreen({ client = api }: Props) {
  const { wording, fmt } = useI18n();
  const colors = useColors();
  const w = wording.payments;
  const screen = usePaymentOptions(client);
  const buttons = useRef(new Map<string, View>());
  const asked = useRef<PaymentApp | null>(null);
  const { done, confirming, step, saved } = screen;

  useEffect(() => {
    // Saved or removed: the notice is read out. Cancelled: back where it came from.
    if (done?.what !== 'cancelled') return;
    focusOn(buttons.current.get(done.app ? `edit-${done.app}` : 'add') ?? null);
  }, [done]);

  useEffect(() => {
    if (confirming || !asked.current) return;
    focusOn(buttons.current.get(`remove-${asked.current}`) ?? null);
    asked.current = null;
  }, [confirming]);

  const track = (key: string) => (control: View | null) => {
    if (control) buttons.current.set(key, control);
    else buttons.current.delete(key);
  };

  const added = addedApps(saved);
  const toAdd = appsToAdd(saved);
  const doneText =
    done?.what === 'saved'
      ? fmt(w.savedOne, { app: w.apps[done.app] })
      : done?.what === 'removed'
        ? fmt(done.last ? w.removedLast : w.removedOne, { app: w.apps[done.app] })
        : null;

  return (
    <Screen>
      <Heading>{w.heading}</Heading>
      <P>{w.intro}</P>
      {saved === null && screen.loadFailure === null ? <Hint>{wording.common.loading}</Hint> : null}
      {screen.loadFailure ? (
        <>
          <Failure code={screen.loadFailure} />
          <Actions>
            <Button label={wording.common.tryAgain} onPress={screen.reload} />
          </Actions>
        </>
      ) : null}

      {saved !== null && step.name === 'list' ? (
        <>
          {doneText ? <Notice>{doneText}</Notice> : null}
          <Failure code={screen.failure} />
          {added.length === 0 ? <P>{w.empty}</P> : null}
          {added.map((app) => (
            <Card key={app}>
              <View testID={`payment-row-${app}`} style={styles.rowText}>
                <Heading level={2}>{w.apps[app]}</Heading>
                <Text style={[type.body, { color: colors.text }]}>{handleShown(app, saved)}</Text>
              </View>
              <Actions>
                <Button
                  label={w.edit}
                  accessibilityLabel={fmt(w.editWhat, { app: w.apps[app] })}
                  disabled={screen.busy}
                  buttonRef={track(`edit-${app}`)}
                  onPress={() => screen.edit(app)}
                />
                <Button
                  label={w.removeOne}
                  accessibilityLabel={fmt(w.removeWhat, { app: w.apps[app] })}
                  disabled={screen.busy}
                  buttonRef={track(`remove-${app}`)}
                  onPress={() => {
                    asked.current = app;
                    screen.askToRemove(app);
                  }}
                />
              </Actions>
            </Card>
          ))}
          {toAdd.length > 0 ? (
            <Actions>
              <Button
                testID="payment-add"
                variant="primary"
                label={w.add}
                disabled={screen.busy}
                buttonRef={track('add')}
                onPress={screen.startAdding}
              />
            </Actions>
          ) : (
            <Hint>{w.allAdded}</Hint>
          )}
        </>
      ) : null}

      {saved !== null && step.name === 'pick' ? (
        <>
          <StepHeading>{w.pickHeading}</StepHeading>
          <P>{w.pickIntro}</P>
          <Actions>
            {toAdd.map((app) => (
              <Button
                key={app}
                testID={`payment-pick-${app}`}
                label={w.apps[app]}
                onPress={() => screen.pick(app)}
              />
            ))}
          </Actions>
          <Actions>
            <Button label={wording.common.cancel} onPress={screen.cancel} />
          </Actions>
        </>
      ) : null}

      {saved !== null && step.name === 'field' ? (
        <OptionField screen={screen} app={step.app} adding={step.adding} />
      ) : null}

      {confirming ? (
        <ConfirmRemove
          app={confirming}
          last={added.length === 1 && added[0] === confirming}
          busy={screen.busy}
          onRemove={() => void screen.remove()}
          onKeep={screen.keep}
        />
      ) : null}
    </Screen>
  );
}

/** A step's heading, which a screen reader is taken to as the step opens. */
function StepHeading({ children }: { children: string }) {
  const heading = useRef<Text>(null);
  useEffect(() => {
    focusOn(heading.current as unknown as View | null);
  }, []);
  return (
    <Heading level={2} headingRef={heading}>
      {children}
    </Heading>
  );
}

/** Adding or editing one app's option: its one field, as on the old form. */
function OptionField({ screen, app, adding }: { screen: Screenful; app: PaymentApp; adding: boolean }) {
  const { wording, fmt } = useI18n();
  const w = wording.payments;

  // A US number for Zelle is written the American way on leaving the field;
  // anything else is left as typed, and nothing changes if it already is.
  const showZelleNumber = () => {
    const shown = phoneAsTyped(screen.input);
    if (shown !== screen.input) screen.setInput(shown);
  };

  return (
    <>
      <StepHeading>{fmt(adding ? w.addHeading : w.editHeading, { app: w.apps[app] })}</StepHeading>
      <TextField
        testID={`payment-${app}`}
        label={w[FIELD[app].label]}
        hint={w[FIELD[app].hint]}
        error={screen.invalid ? w[FIELD[app].invalid] : null}
        required
        keyboardType={KEYBOARD[app]}
        autoCapitalize="none"
        autoCorrect={false}
        autoComplete="off"
        spellCheck={false}
        value={screen.input}
        onChangeText={screen.setInput}
        onBlur={app === 'zelle' ? showZelleNumber : undefined}
        onSubmitEditing={() => void screen.save()}
      />
      <Failure code={screen.failure} />
      <Actions>
        <Button
          testID="payment-save"
          variant="primary"
          label={w.saveOne}
          disabled={screen.busy}
          onPress={() => void screen.save()}
        />
        <Button label={wording.common.cancel} disabled={screen.busy} onPress={screen.cancel} />
      </Actions>
    </>
  );
}

/**
 * "Remove Venmo?": a modal sheet in the app, as the payer's sheet is. The
 * screen behind is hidden from a screen reader, the system's back keeps
 * the option, and a screen reader starts on the question.
 */
function ConfirmRemove({
  app,
  last,
  busy,
  onRemove,
  onKeep,
}: {
  app: PaymentApp;
  last: boolean;
  busy: boolean;
  onRemove(): void;
  onKeep(): void;
}) {
  const { wording, fmt, language } = useI18n();
  const colors = useColors();
  const insets = useSafeAreaInsets();
  const w = wording.payments;
  const heading = useRef<Text>(null);
  const title = fmt(w.confirmTitle, { app: w.apps[app] });

  return (
    <Modal
      visible
      transparent
      animationType="fade"
      onRequestClose={() => {
        if (!busy) onKeep();
      }}
      onShow={() => focusOn(heading.current as unknown as View | null)}>
      <View style={[styles.backdrop, { backgroundColor: 'rgba(17, 19, 39, 0.6)' }]}>
        <View
          accessibilityViewIsModal
          aria-modal
          role="alertdialog"
          aria-label={title}
          testID="payment-confirm"
          style={[
            styles.sheet,
            {
              backgroundColor: colors.surface,
              borderColor: colors.divider,
              paddingBottom: Math.max(insets.bottom, space.l),
            },
          ]}>
          <Text
            ref={heading}
            accessibilityRole="header"
            accessibilityLanguage={language}
            aria-level={2}
            style={[type.heading, { color: colors.text }]}>
            {title}
          </Text>
          <P>{last ? w.confirmLast : w.confirmText}</P>
          <Actions>
            <Button
              testID="payment-confirm-remove"
              variant="primary"
              label={fmt(w.removeWhat, { app: w.apps[app] })}
              disabled={busy}
              onPress={onRemove}
            />
            <Button label={w.keep} disabled={busy} onPress={onKeep} />
          </Actions>
        </View>
      </View>
    </Modal>
  );
}

const styles = StyleSheet.create({
  rowText: { gap: space.xs },
  backdrop: { flex: 1, justifyContent: 'flex-end' },
  sheet: {
    width: '100%',
    maxWidth: 640,
    alignSelf: 'center',
    borderTopLeftRadius: radius.l,
    borderTopRightRadius: radius.l,
    borderWidth: 1,
    padding: space.l,
    gap: space.m,
  },
});
