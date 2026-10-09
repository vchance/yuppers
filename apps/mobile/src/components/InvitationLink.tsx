import {
  boundToLabel,
  invitationLink,
  inviteeKind,
  phoneAsTyped,
  phoneOffered,
  shareAddresses,
  type InvitationChoice,
  type SignInChannels,
} from '@yuppers/shared';
import * as Clipboard from 'expo-clipboard';
import { useEffect, useRef, useState } from 'react';
import { Linking, Platform, Share, StyleSheet, Text, View, type TextInput } from 'react-native';

import { WEB_URL } from '../lib/config';
import { useI18n } from '../lib/context';
import { space, type, useColors } from '../lib/theme';
import { QrCode } from './QrCode';
import { Actions, Button, ErrorNote, Hint, Label, Notice, P, TextField } from './ui';

/**
 * Who an invitation is for (DESIGN.md §8), with the same words, checks and
 * order as on the web. Naming them is what is expected: only an account
 * verified with that email address or phone number can use the link, and
 * needs no confirming, so they can respond in full as soon as they sign in.
 * The field says why, that we do not contact them, and that they must sign
 * in with exactly what is typed. A link for anyone is offered after it as a
 * deliberate choice, which says what it costs once chosen.
 *
 * An email address only, until the service says it texts codes (`channels`),
 * as signing in asks: a phone number nobody can sign in with would make the
 * link useless.
 */
export function InvitationFor({
  choice,
  onChange,
  channels,
  error,
}: {
  choice: InvitationChoice;
  onChange(choice: InvitationChoice): void;
  channels: SignInChannels | null;
  /** What is wrong with it, said with it once the person has tried to go on. */
  error?: string | null;
}) {
  const { wording } = useI18n();
  const w = wording.invitationLink;
  const phone = phoneOffered(channels);
  const field = useRef<TextInput>(null);
  // Set when the person goes back to naming them, so the keyboard follows,
  // and not when the form is first shown.
  const switched = useRef(false);

  useEffect(() => {
    if (!switched.current || choice.anyone) return;
    switched.current = false;
    field.current?.focus();
  }, [choice.anyone]);

  if (choice.anyone) {
    return (
      <View style={styles.block}>
        {/* Read out as it appears, so what it costs is heard at once. */}
        <Notice tone="warning">{w.forAnyoneText}</Notice>
        <Actions>
          <Button
            variant="link"
            label={w.forNamed}
            onPress={() => {
              switched.current = true;
              onChange({ ...choice, anyone: false });
            }}
          />
        </Actions>
      </View>
    );
  }

  return (
    <View style={styles.block}>
      <P>{w.forIntro}</P>
      <P>{w.forNoContact}</P>
      <TextField
        input={field}
        label={boundToLabel(w, channels)}
        hint={w.forHint}
        error={error}
        required
        inputMode="email"
        keyboardType={phone ? 'default' : 'email-address'}
        autoCapitalize="none"
        autoCorrect={false}
        autoComplete="off"
        value={choice.to}
        onChangeText={(to) => onChange({ ...choice, to })}
        // A US number is written the American way on leaving the field.
        onBlur={() => {
          const shown = phoneAsTyped(choice.to);
          if (shown !== choice.to) onChange({ ...choice, to: shown });
        }}
      />
      <Actions>
        <Button
          variant="link"
          label={w.forAnyone}
          onPress={() => onChange({ ...choice, anyone: true })}
        />
      </Actions>
    </View>
  );
}

interface ShareProps {
  token: string;
  /** Who the invitation was made for, as typed, when it names someone. */
  boundTo: string | null;
  /**
   * Called when the person opens any way to pass the link on. The service
   * is told, so the exchange's screen and the list can say the link was
   * sent, or that it was not.
   */
  onShared?: () => void;
}

/** Opens a message in another app: Messages, Mail, WhatsApp. */
function openOutside(url: string): void {
  // Nothing to do if it cannot be opened here; the link can still be copied.
  Linking.openURL(url).catch(() => {});
}

/**
 * The ways to send the link (DESIGN.md §8), led by the one that reaches the
 * person it was made for. Yuppers never sends it; one of these has to be
 * opened by the sender, so each is a real button, and the first of them is
 * the primary action:
 *
 *   - named by phone number: a text message to that number, with WhatsApp
 *     and copying the link beside it;
 *   - named by email address: an email to that address, with copying beside it;
 *   - for anyone: the system's share sheet, with copying, a text message,
 *     WhatsApp and an email to nobody in particular beside it;
 *
 * and, for someone in the same room, the link as a QR code. Each message is
 * started by this device, with the whole link inside it (`shareAddresses`);
 * the token reaches no server of ours.
 */
export function ShareActions({ token, boundTo, onShared }: ShareProps) {
  const { wording, language, fmt } = useI18n();
  const w = wording.invitationLink;
  const [copied, setCopied] = useState<'yes' | 'failed' | null>(null);
  const [qr, setQr] = useState(false);
  const link = invitationLink(WEB_URL, language, token);
  const addresses = shareAddresses({
    message: fmt(w.shareMessage, { link }),
    subject: wording.linkPreview.title,
    boundTo,
  });
  const kind = inviteeKind(boundTo);

  const shared = () => onShared?.();

  async function copy() {
    shared();
    try {
      setCopied((await Clipboard.setStringAsync(link)) ? 'yes' : 'failed');
    } catch {
      setCopied('failed');
    }
  }

  function share() {
    shared();
    // iOS takes the link as a link; Android's share sheet takes text only.
    const content =
      Platform.OS === 'android'
        ? { title: wording.linkPreview.title, message: fmt(w.shareMessage, { link }) }
        : { title: wording.linkPreview.title, message: w.shareText, url: link };
    // Dismissing the share sheet is not a failure; there is nothing to do about either.
    Share.share(content).catch(() => {});
  }

  function toggleQr() {
    // Showing the code is a way of passing the link on; hiding it is not.
    if (!qr) shared();
    setQr((shown) => !shown);
  }

  const open = (url: string) => () => {
    shared();
    openOutside(url);
  };

  return (
    <View style={styles.block}>
      <Actions>
        {kind === 'phone' && (
          <>
            <Button variant="primary" label={w.sendText} onPress={open(addresses.sms)} />
            <Button label={w.sendWhatsApp} onPress={open(addresses.whatsApp)} />
            <Button label={w.copy} onPress={() => void copy()} />
          </>
        )}
        {kind === 'email' && (
          <>
            <Button variant="primary" label={w.sendEmail} onPress={open(addresses.email)} />
            <Button label={w.copy} onPress={() => void copy()} />
          </>
        )}
        {kind === 'anyone' && (
          <>
            <Button variant="primary" label={w.share} onPress={share} />
            <Button label={w.copy} onPress={() => void copy()} />
            <Button label={w.shareSms} onPress={open(addresses.sms)} />
            <Button label={w.shareWhatsApp} onPress={open(addresses.whatsApp)} />
            <Button label={w.shareEmail} onPress={open(addresses.email)} />
          </>
        )}
        <Button label={qr ? w.hideQr : w.shareQr} expanded={qr} onPress={toggleQr} />
      </Actions>
      {qr && (
        <View style={styles.block}>
          <QrCode text={link} label={w.qrLabel} />
          <Hint>{w.qrHint}</Hint>
        </View>
      )}
      {copied === 'yes' && <Notice>{w.copied}</Notice>}
      {copied === 'failed' && <ErrorNote>{w.copyFailed}</ErrorNote>}
    </View>
  );
}

/**
 * The invitation link, shown once: only its hash is kept by the service, so
 * it cannot be shown again. The initiator sends it through a channel of
 * their own; the platform never sends it (DESIGN.md §8), and says so where
 * the link appears. Under it, the ways to send it (`ShareActions`).
 *
 * The link is the web address, in the sender's language, so it works for
 * someone without the app and previews in that language. What is shared
 * alongside it is fixed wording with no name, term or amount in it.
 */
export function InvitationLink({ token, boundTo, onShared }: ShareProps) {
  const { wording, language } = useI18n();
  const colors = useColors();
  const w = wording.invitationLink;
  const link = invitationLink(WEB_URL, language, token);

  return (
    <View style={styles.block}>
      <P>{w.intro}</P>
      <Label>{w.linkLabel}</Label>
      <Text
        selectable
        accessibilityLabel={w.linkLabel}
        // An address reads left to right in every language.
        style={[type.hint, styles.link, { color: colors.text, borderColor: colors.border }]}>
        {link}
      </Text>
      <Hint>{w.shownOnce}</Hint>
      <ShareActions token={token} boundTo={boundTo} onShared={onShared} />
    </View>
  );
}

const styles = StyleSheet.create({
  block: { gap: space.m },
  link: { borderWidth: 2, borderRadius: 16, padding: space.m, writingDirection: 'ltr' },
});
