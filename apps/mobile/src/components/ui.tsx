import type { ErrorCode } from '@yuppers/api-client';
import { createContext, useContext, useEffect, useRef, type ReactNode, type Ref } from 'react';
import {
  KeyboardAvoidingView,
  Platform,
  Pressable,
  RefreshControl,
  ScrollView,
  StyleSheet,
  Switch,
  Text,
  TextInput,
  View,
  type StyleProp,
  type TextInputProps,
  type TextStyle,
  type ViewStyle,
} from 'react-native';
import { useSafeAreaInsets } from 'react-native-safe-area-context';

import { announce, focusOn, notePressed } from '../lib/accessibility';
import { useI18n } from '../lib/context';
import { fonts, radius, space, TOUCH_TARGET, type, useColors } from '../lib/theme';

/*
 * The app's few building blocks: React Native's own components with the
 * app's two fonts (`lib/theme.ts`) and the platform's controls, and no UI kit. Each carries its
 * accessibility role, label and state, and nothing interactive is smaller
 * than a comfortable touch target. Text follows the size the person has
 * chosen for their device, with no upper limit, and every row wraps.
 *
 * The product's own words are marked with the language they are in, so a
 * screen reader reads them with the right voice when the app's language is
 * not the device's (iOS; Android reads with the system's). What people wrote
 * is not marked: its language is whatever they wrote it in.
 *
 * Nothing is positioned on the assumption that text runs left to right
 * (DESIGN.md §4.2): spacing and borders use `start` and `end`, and rows
 * follow the layout direction.
 */

interface ScreenProps {
  children: ReactNode;
  /** Pulling down refreshes, where there is something to refresh. */
  onRefresh?: () => void;
  refreshing?: boolean;
  scroll?: Ref<ScrollView>;
}

/** A screen's scrolling body, clear of the keyboard and the device's edges. */
export function Screen({ children, onRefresh, refreshing = false, scroll }: ScreenProps) {
  const colors = useColors();
  const insets = useSafeAreaInsets();
  return (
    <KeyboardAvoidingView
      style={[styles.fill, { backgroundColor: colors.background }]}
      behavior={Platform.OS === 'ios' ? 'padding' : undefined}
      keyboardVerticalOffset={Platform.OS === 'ios' ? 64 : 0}>
      <ScrollView
        ref={scroll}
        style={styles.fill}
        contentInsetAdjustmentBehavior="automatic"
        keyboardShouldPersistTaps="handled"
        contentContainerStyle={[
          styles.screen,
          {
            paddingBottom: space.xl + insets.bottom,
            paddingStart: space.l + insets.left,
            paddingEnd: space.l + insets.right,
          },
        ]}
        refreshControl={
          onRefresh ? (
            <RefreshControl
              refreshing={refreshing}
              onRefresh={onRefresh}
              tintColor={colors.muted}
            />
          ) : undefined
        }>
        <View style={styles.column}>{children}</View>
      </ScrollView>
    </KeyboardAvoidingView>
  );
}

/** How deep a heading sits: 1 names the screen, 2 a part of it, and so on. */
export type HeadingLevel = 1 | 2 | 3 | 4;

/**
 * A heading, announced as one. Level 1 names the screen; there is one per
 * screen. The level is also given as `aria-level`, which the screen readers
 * of iOS and Android do not use but the browser harness turns into `h1` to
 * `h4`, so the harness shows the same outline a browser would.
 */
export function Heading({
  children,
  level = 1,
  color,
}: {
  children: string;
  level?: HeadingLevel;
  /** On a party's colour, that colour's own ink. */
  color?: string;
}) {
  const colors = useColors();
  const { language } = useI18n();
  const size = level === 1 ? type.title : level === 2 ? type.heading : type.subheading;
  return (
    <Text
      accessibilityRole="header"
      accessibilityLanguage={language}
      aria-level={level}
      style={[size, styles.heading, { color: color ?? colors.text }]}>
      {children}
    </Text>
  );
}

/** A paragraph of the product's own words. */
export function P({ children, style }: { children: ReactNode; style?: StyleProp<TextStyle> }) {
  const colors = useColors();
  const { language } = useI18n();
  return (
    <Text accessibilityLanguage={language} style={[type.body, { color: colors.text }, style]}>
      {children}
    </Text>
  );
}

/** Secondary text: a hint under a label, a reference, a date. */
export function Hint({ children }: { children: ReactNode }) {
  const colors = useColors();
  const { language } = useI18n();
  return (
    <Text accessibilityLanguage={language} style={[type.hint, { color: colors.muted }]}>
      {children}
    </Text>
  );
}

/** A small label over something, such as "Done when". */
export function Label({ children }: { children: string }) {
  const colors = useColors();
  const { language } = useI18n();
  return (
    <Text
      accessibilityLanguage={language}
      style={[type.body, styles.label, { color: colors.text }]}>
      {children}
    </Text>
  );
}

interface ButtonProps {
  label: string;
  onPress(): void;
  variant?: 'primary' | 'default' | 'link';
  disabled?: boolean;
  /** For a button that opens a panel under it: whether the panel is open. */
  expanded?: boolean;
  accessibilityLabel?: string;
  /** Said after the label, when the label alone does not say what pressing does. */
  hint?: string;
  testID?: string;
}

export function Button({
  label,
  onPress,
  variant = 'default',
  disabled = false,
  expanded,
  accessibilityLabel,
  hint,
  testID,
}: ButtonProps) {
  const colors = useColors();
  const { language } = useI18n();
  const control = useRef<View>(null);
  const primary = variant === 'primary';
  const link = variant === 'link';
  return (
    <Pressable
      ref={control}
      testID={testID}
      accessibilityRole={link ? 'link' : 'button'}
      accessibilityLabel={accessibilityLabel ?? label}
      accessibilityHint={hint}
      accessibilityLanguage={language}
      // Said both ways: the first is what the platforms read, the second what a browser does.
      accessibilityState={{ disabled, expanded }}
      aria-disabled={disabled || undefined}
      aria-expanded={expanded}
      disabled={disabled}
      onPress={() => {
        // Kept so that a panel this opens can give the focus back to it.
        notePressed(control.current);
        onPress();
      }}
      style={({ pressed }) => [
        styles.button,
        link
          ? styles.buttonLink
          : {
              borderColor: primary ? colors.primary : colors.text,
              backgroundColor: primary ? colors.primary : colors.surface,
              borderWidth: 2,
            },
        (pressed || disabled) && styles.dimmed,
      ]}>
      <Text
        style={[
          type.body,
          styles.buttonText,
          primary && styles.buttonTextPrimary,
          { color: primary ? colors.onPrimary : link ? colors.link : colors.text },
          link && styles.underlined,
        ]}>
        {label}
      </Text>
    </Pressable>
  );
}

/** A group of buttons. It wraps, so a long translation never needs a fixed width. */
export function Actions({ children }: { children: ReactNode }) {
  return <View style={styles.actions}>{children}</View>;
}

interface NoteProps {
  children: ReactNode;
  tone?: 'info' | 'warning' | 'error';
  /** The text to read out when it appears. */
  spoken?: string;
}

function Note({ children, tone = 'info', spoken }: NoteProps) {
  const colors = useColors();
  const error = tone === 'error';
  // Said by announcing it, on both platforms. It is not also a live region:
  // Android would then say it twice, and say the quiet ones too.
  useEffect(() => {
    if (spoken) announce(spoken);
  }, [spoken]);
  return (
    <View
      accessibilityRole={error ? 'alert' : undefined}
      style={[
        styles.note,
        {
          borderStartColor: error
            ? colors.danger
            : tone === 'warning'
              ? colors.partyYou
              : colors.border,
          backgroundColor: error
            ? colors.dangerSurface
            : tone === 'warning'
              ? colors.warningSurface
              : colors.surfaceRaised,
        },
      ]}>
      {typeof children === 'string' ? <P>{children}</P> : children}
    </View>
  );
}

/** A refusal from the service, in the reader's language. Announced when it appears. */
export function Failure({ code }: { code: ErrorCode | null }) {
  const { errorText } = useI18n();
  if (!code) return null;
  return <ErrorNote>{errorText(code)}</ErrorNote>;
}

/** Something that went wrong, said in the product's own wording. */
export function ErrorNote({ children }: { children: string }) {
  return (
    <Note tone="error" spoken={children}>
      {children}
    </Note>
  );
}

/** Something that happened, announced without interrupting. */
export function Notice({
  children,
  tone,
  quiet,
}: {
  children: ReactNode;
  tone?: 'info' | 'warning';
  /** Shown but not read out by itself: standing information, not news. */
  quiet?: boolean;
}) {
  return (
    <Note tone={tone} spoken={!quiet && typeof children === 'string' ? children : undefined}>
      {children}
    </Note>
  );
}

/**
 * Whether a field's error is announced when it appears. A form that sums up
 * its problems in one announcement turns this off, so the summary is not
 * talked over by each field in turn; each error is still read with its field.
 */
export const FieldErrorsAnnounced = createContext(true);

/** A field's error, in the danger colour and in words, said when it appears. */
function FieldError({ children }: { children: string }) {
  const colors = useColors();
  const { language } = useI18n();
  const announced = useContext(FieldErrorsAnnounced);
  useEffect(() => {
    if (announced) announce(children);
  }, [announced, children]);
  return (
    <Text
      accessibilityRole="alert"
      accessibilityLanguage={language}
      style={[type.hint, styles.fieldError, { color: colors.danger }]}>
      {children}
    </Text>
  );
}

interface FieldProps {
  label: string;
  hint?: string;
  error?: string | null;
  /**
   * The control says the label and hint to a screen reader itself, so the
   * words above it are for the eye only and are not read a second time.
   */
  labelled?: boolean;
  children: ReactNode;
}

/** A label, its hint and its error around a control. */
export function Field({ label, hint, error, labelled = false, children }: FieldProps) {
  const colors = useColors();
  const { language } = useI18n();
  return (
    <View style={styles.field}>
      <View aria-hidden={labelled || undefined}>
        <Text
          accessibilityLanguage={language}
          style={[type.body, styles.label, { color: colors.text }]}>
          {label}
        </Text>
        {hint ? <Hint>{hint}</Hint> : null}
      </View>
      {children}
      {error ? <FieldError>{error}</FieldError> : null}
    </View>
  );
}

interface TextFieldProps extends Omit<TextInputProps, 'style' | 'editable'> {
  label: string;
  hint?: string;
  error?: string | null;
  disabled?: boolean;
  /**
   * Has to be filled in. Neither platform has a "required" state for a text
   * field, so a screen reader is told in the hint.
   */
  required?: boolean;
  input?: Ref<TextInput>;
}

/** A labelled text input. Text is laid out in whichever direction its own script runs. */
export function TextField({
  label,
  hint,
  error,
  disabled,
  required,
  input,
  ...rest
}: TextFieldProps) {
  const colors = useColors();
  const { wording, language } = useI18n();
  return (
    <Field label={label} hint={hint} error={error} labelled>
      <TextInput
        ref={input}
        accessibilityLabel={label}
        accessibilityLanguage={language}
        // The error first: it is what the person needs to hear on coming back.
        accessibilityHint={
          [error, required ? wording.a11y.required : null, hint].filter(Boolean).join(' ') ||
          undefined
        }
        accessibilityState={{ disabled: !!disabled }}
        aria-invalid={error ? true : undefined}
        aria-required={required || undefined}
        editable={!disabled}
        placeholderTextColor={colors.muted}
        {...rest}
        style={[
          type.body,
          styles.input,
          rest.multiline && styles.inputMultiline,
          {
            color: colors.text,
            borderColor: error ? colors.danger : colors.border,
            backgroundColor: colors.surface,
          },
          disabled && styles.dimmed,
        ]}
      />
    </Field>
  );
}

interface ChoiceProps<T extends string> {
  label: string;
  hint?: string;
  error?: string | null;
  value: T | null;
  options: readonly { value: T; label: string }[];
  onChange(value: T): void;
  disabled?: boolean;
}

/** One of a few options, as a group of radio buttons. */
export function Choice<T extends string>({
  label,
  hint,
  error,
  value,
  options,
  onChange,
  disabled = false,
}: ChoiceProps<T>) {
  const colors = useColors();
  const { language } = useI18n();
  return (
    <Field label={label} hint={hint} error={error}>
      <View accessibilityRole="radiogroup" accessibilityLabel={label} style={styles.choice}>
        {options.map((option) => {
          const selected = option.value === value;
          return (
            <Pressable
              key={option.value}
              accessibilityRole="radio"
              accessibilityLabel={option.label}
              accessibilityLanguage={language}
              accessibilityState={{ checked: selected, disabled }}
              aria-checked={selected}
              aria-disabled={disabled || undefined}
              disabled={disabled}
              onPress={() => onChange(option.value)}
              style={({ pressed }) => [styles.option, (pressed || disabled) && styles.dimmed]}>
              <View style={[styles.radio, { borderColor: selected ? colors.primary : colors.border }]}>
                {selected ? (
                  <View style={[styles.radioDot, { backgroundColor: colors.primary }]} />
                ) : null}
              </View>
              <Text style={[type.body, styles.optionText, { color: colors.text }]}>
                {option.label}
              </Text>
            </Pressable>
          );
        })}
      </View>
    </Field>
  );
}

interface CheckProps {
  label: string;
  value: boolean;
  onChange(value: boolean): void;
  error?: string | null;
  /** Said after the label, where the label alone is not enough. */
  hint?: string;
  disabled?: boolean;
  testID?: string;
}

/** A yes-or-no answer, as the platform's own switch. It is off until the person turns it on. */
export function Check({
  label,
  value,
  onChange,
  error,
  hint,
  disabled,
  testID,
}: CheckProps) {
  const colors = useColors();
  const { language } = useI18n();
  return (
    <View style={styles.field}>
      <View style={styles.check}>
        <Switch
          testID={testID}
          accessibilityRole="switch"
          accessibilityLabel={label}
          accessibilityLanguage={language}
          accessibilityHint={[error, hint].filter(Boolean).join(' ') || undefined}
          accessibilityState={{ checked: value, disabled: !!disabled }}
          value={value}
          onValueChange={onChange}
          disabled={disabled}
        />
        <Text
          // The switch is labelled; pressing the words works too, for sighted users.
          accessible={false}
          importantForAccessibility="no"
          aria-hidden
          onPress={disabled ? undefined : () => onChange(!value)}
          style={[type.body, styles.checkLabel, { color: colors.text }]}>
          {label}
        </Text>
      </View>
      {error ? <FieldError>{error}</FieldError> : null}
    </View>
  );
}

/** Something set apart: an exchange in a list, a proposal, the agreement. */
export function Card({ children, style }: { children: ReactNode; style?: StyleProp<ViewStyle> }) {
  const colors = useColors();
  return (
    <View
      style={[styles.card, { borderColor: colors.divider, backgroundColor: colors.surface }, style]}>
      {children}
    </View>
  );
}

export function Tags({ children }: { children: ReactNode }) {
  return <View style={styles.tags}>{children}</View>;
}

export function Tag({ children, alert }: { children: string; alert?: boolean }) {
  const colors = useColors();
  return (
    <Text
      style={[
        type.hint,
        styles.tag,
        {
          color: alert ? colors.danger : colors.text,
          borderColor: alert ? colors.danger : colors.border,
        },
      ]}>
      {children}
    </Text>
  );
}

/**
 * An action that has been opened but not sent. It appears in place, under
 * the thing it acts on, and a screen reader is taken to it.
 */
export function Panel({
  title,
  children,
  level = 3,
}: {
  title: string;
  children: ReactNode;
  /**
   * The level of the panel's heading, one below the heading of the part of
   * the screen it opens in. Most open under a level 2 heading, so 3.
   */
  level?: Exclude<HeadingLevel, 1>;
}) {
  const colors = useColors();
  const { language } = useI18n();
  const heading = useRef<Text>(null);

  useEffect(() => {
    focusOn(heading.current as unknown as View | null);
  }, []);

  return (
    <View
      accessibilityLabel={title}
      style={[styles.panel, { borderColor: colors.divider, backgroundColor: colors.surfaceRaised }]}>
      <Text
        ref={heading}
        accessibilityRole="header"
        accessibilityLanguage={language}
        aria-level={level}
        style={[type.subheading, { color: colors.text }]}>
        {title}
      </Text>
      {children}
    </View>
  );
}

/**
 * Text a person wrote: a name, terms, a description, a note. It is shown
 * exactly as written and never translated (DESIGN.md §4.2), and set apart so
 * it cannot be mistaken for the product speaking. The system lays it out in
 * whichever direction its own script runs.
 */
export function Written({ children }: { children: string }) {
  const colors = useColors();
  return (
    <Text
      style={[type.body, styles.written, { color: colors.text, borderStartColor: colors.border }]}>
      {children}
    </Text>
  );
}

/** A list of short statements, each on its own line. */
export function Lines({ children }: { children: ReactNode }) {
  return <View style={styles.lines}>{children}</View>;
}

const styles = StyleSheet.create({
  fill: { flex: 1 },
  screen: { paddingTop: space.l, flexGrow: 1 },
  // Readable line length on a tablet; the full width on a phone.
  column: { width: '100%', maxWidth: 640, alignSelf: 'center', gap: space.l },
  heading: { marginTop: space.xs },
  label: { fontFamily: fonts.textBold },
  button: {
    minHeight: TOUCH_TARGET,
    minWidth: TOUCH_TARGET,
    // A long label wraps within the row rather than running off the screen.
    // Yoga already keeps it in on a device; a browser does not without this.
    maxWidth: '100%',
    paddingVertical: space.m,
    paddingHorizontal: space.l,
    borderRadius: radius.pill,
    alignItems: 'center',
    justifyContent: 'center',
  },
  buttonLink: { paddingHorizontal: space.xs },
  buttonText: { fontFamily: fonts.textBold, textAlign: 'center' },
  buttonTextPrimary: { fontFamily: fonts.display, fontSize: 18 },
  underlined: { textDecorationLine: 'underline' },
  dimmed: { opacity: 0.5 },
  actions: { flexDirection: 'row', flexWrap: 'wrap', gap: space.m, alignItems: 'center' },
  note: { borderStartWidth: 4, borderRadius: radius.m, padding: space.m, gap: space.s },
  field: { gap: space.s },
  fieldError: { fontFamily: fonts.textBold },
  input: {
    minHeight: TOUCH_TARGET,
    borderWidth: 2,
    borderRadius: radius.m,
    paddingVertical: space.m,
    paddingHorizontal: space.m,
  },
  inputMultiline: { minHeight: TOUCH_TARGET * 2, textAlignVertical: 'top' },
  choice: { gap: space.xs },
  option: {
    minHeight: TOUCH_TARGET,
    flexDirection: 'row',
    alignItems: 'center',
    gap: space.m,
    paddingVertical: space.s,
  },
  optionText: { flex: 1 },
  radio: {
    width: 24,
    height: 24,
    borderRadius: 12,
    borderWidth: 2,
    alignItems: 'center',
    justifyContent: 'center',
  },
  radioDot: { width: 12, height: 12, borderRadius: 6 },
  check: { minHeight: TOUCH_TARGET, flexDirection: 'row', alignItems: 'center', gap: space.m },
  checkLabel: { flex: 1 },
  card: { borderWidth: 1, borderRadius: radius.l, padding: space.l, gap: space.m },
  tags: { flexDirection: 'row', flexWrap: 'wrap', gap: space.s },
  tag: {
    fontFamily: fonts.textBold,
    borderWidth: 2,
    borderRadius: radius.s,
    paddingVertical: space.xs,
    paddingHorizontal: space.s,
    overflow: 'hidden',
  },
  panel: { borderWidth: 2, borderRadius: radius.m, padding: space.l, gap: space.m },
  written: { borderStartWidth: 3, paddingStart: space.m },
  lines: { gap: space.xs },
});
