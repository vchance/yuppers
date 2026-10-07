import type { components } from '@yuppers/api-client';
import { StyleSheet, Text, View } from 'react-native';

import { useI18n } from '../lib/context';
import { fonts, radius, space, type, useColors, type Colors } from '../lib/theme';

type Status = components['schemas']['Status'];

/**
 * Where a contribution stands, in words, on a chip whose fill and icon say
 * it as well: a ring for not yet, a half-filled ring for waiting to be
 * confirmed, an exclamation for disputed, a check for confirmed, a dash for
 * waived or removed. So no status is told from another by colour alone. The
 * icon is drawn, and only the words are read.
 */
export function StatusChip({ status, children }: { status: Status; children: string }) {
  const colors = useColors();
  const { language } = useI18n();
  const look = lookOf(status, colors);
  return (
    <View
      style={[
        styles.chip,
        {
          backgroundColor: look.fill,
          borderColor: look.edge,
          borderStyle: look.dashed ? 'dashed' : 'solid',
        },
      ]}>
      <StatusIcon status={status} color={look.icon} />
      <Text accessibilityLanguage={language} style={[type.hint, styles.text, { color: look.ink }]}>
        {children}
      </Text>
    </View>
  );
}

function lookOf(status: Status, colors: Colors) {
  switch (status) {
    case 'PENDING':
      return { fill: colors.surface, edge: colors.text, ink: colors.text, icon: colors.text, dashed: false };
    case 'CLAIMED':
      return {
        fill: colors.statusWaiting,
        edge: colors.statusWaiting,
        ink: colors.onStatusWaiting,
        icon: colors.onStatusWaiting,
        dashed: false,
      };
    case 'DISPUTED':
      return {
        fill: colors.statusDisputed,
        edge: colors.statusDisputed,
        ink: colors.onStatusDisputed,
        icon: colors.onStatusDisputed,
        dashed: false,
      };
    case 'ACCEPTED':
      return {
        fill: colors.statusConfirmed,
        edge: colors.statusConfirmed,
        ink: colors.onStatusConfirmed,
        icon: colors.statusConfirmedIcon,
        dashed: false,
      };
    default:
      return { fill: 'transparent', edge: colors.border, ink: colors.muted, icon: colors.muted, dashed: true };
  }
}

function StatusIcon({ status, color }: { status: Status; color: string }) {
  const ring = [styles.icon, styles.ring, { borderColor: color }];
  let icon;
  if (status === 'PENDING') icon = <View style={ring} />;
  else if (status === 'CLAIMED')
    icon = (
      <View style={[ring, styles.clip]}>
        <View style={[styles.half, { backgroundColor: color }]} />
      </View>
    );
  else if (status === 'DISPUTED')
    icon = (
      <View style={[styles.icon, styles.centre]}>
        <View style={[styles.bang, { backgroundColor: color }]} />
        <View style={[styles.dot, { backgroundColor: color }]} />
      </View>
    );
  else if (status === 'ACCEPTED')
    icon = (
      <View style={[styles.icon, styles.centre]}>
        <View style={[styles.tick, { borderColor: color }]} />
      </View>
    );
  else
    icon = (
      <View style={[styles.icon, styles.centre]}>
        <View style={[styles.dash, { backgroundColor: color }]} />
      </View>
    );
  return (
    <View aria-hidden accessibilityElementsHidden importantForAccessibility="no-hide-descendants">
      {icon}
    </View>
  );
}

const styles = StyleSheet.create({
  chip: {
    alignSelf: 'flex-start',
    flexDirection: 'row',
    alignItems: 'center',
    gap: space.s,
    borderWidth: 2,
    borderRadius: radius.s,
    paddingVertical: 2,
    paddingStart: space.s,
    paddingEnd: space.m,
    maxWidth: '100%',
  },
  text: { fontFamily: fonts.textBold, flexShrink: 1 },
  icon: { width: 14, height: 14 },
  centre: { alignItems: 'center', justifyContent: 'center' },
  ring: { borderRadius: 7, borderWidth: 2 },
  clip: { overflow: 'hidden', direction: 'ltr' },
  half: { position: 'absolute', top: 0, bottom: 0, left: 5, right: 0 },
  bang: { width: 2.5, height: 7, borderRadius: 1.25, marginBottom: 1.5 },
  dot: { width: 2.5, height: 2.5, borderRadius: 1.25 },
  tick: {
    width: 6,
    height: 10,
    marginTop: -2,
    borderRightWidth: 2.5,
    borderBottomWidth: 2.5,
    transform: [{ rotate: '45deg' }],
  },
  dash: { width: 9, height: 2.5, borderRadius: 1.25 },
});
