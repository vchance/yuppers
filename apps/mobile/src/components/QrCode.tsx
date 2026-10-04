import { QR_QUIET_ZONE, qrRuns } from '@yuppers/shared';
import { correction, generate, mode } from 'lean-qr';
import { useMemo } from 'react';
import { StyleSheet, useWindowDimensions, View } from 'react-native';

import { space } from '../lib/theme';

/** The widest the code is drawn, in points: a phone's camera reads it at arm's length. */
const LARGEST = 264;

// The ways of encoding text an invitation link can need, without Shift JIS,
// which `lean-qr` would otherwise try by making a `TextDecoder('sjis')`: the
// app's runtime decodes UTF-8 only and refuses it, and a link never needs it.
const MODES = [mode.numeric, mode.alphaNumeric, mode.ascii, mode.utf8];

/**
 * A QR code of `text`, made on the device with `lean-qr` and drawn with
 * plain views, one per run of dark modules in a row: nothing is fetched,
 * so it works offline, and it needs no native module beyond the app's own.
 * Each module is a whole number of points, so no hairline shows between
 * rows. Dark on light whatever the theme, with the quiet zone a reader needs:
 * cameras read a light-on-dark code badly.
 */
export function QrCode({ text, label }: { text: string; label: string }) {
  const { width } = useWindowDimensions();
  const { modules, runs } = useMemo(() => {
    const code = generate(text, { minCorrectionLevel: correction.M, modes: MODES });
    return { modules: code.size + 2 * QR_QUIET_ZONE, runs: qrRuns(code) };
  }, [text]);
  // The screen's own margins on both sides, and the card's.
  const room = Math.min(LARGEST, width - 4 * space.l);
  const unit = Math.max(1, Math.floor(room / modules));
  const size = unit * modules;
  return (
    <View
      accessible
      accessibilityRole="image"
      accessibilityLabel={label}
      testID="qr-code"
      style={[styles.code, { width: size, height: size }]}>
      {runs.map((run) => (
        <View
          key={`${run.x}-${run.y}`}
          style={[
            styles.module,
            {
              left: (run.x + QR_QUIET_ZONE) * unit,
              top: (run.y + QR_QUIET_ZONE) * unit,
              width: run.width * unit,
              height: unit,
            },
          ]}
        />
      ))}
    </View>
  );
}

const styles = StyleSheet.create({
  code: { backgroundColor: '#ffffff', alignSelf: 'flex-start' },
  module: { position: 'absolute', backgroundColor: '#000000' },
});
