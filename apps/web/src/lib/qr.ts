import { QR_QUIET_ZONE, qrRuns } from '@yuppers/shared/qr'
import { correction, generate, mode } from 'lean-qr'

/**
 * A QR code ready to draw: `size` units a side, quiet zone included, and its
 * dark modules as one path.
 */
export interface QrDrawing {
  size: number
  path: string
}

// The ways of encoding text an invitation link can need, without Shift JIS,
// which a link never needs and which `lean-qr` would otherwise try by making
// a `TextDecoder('sjis')`. The mobile app's runtime refuses that, and the
// same choice here keeps both apps' codes alike.
const MODES = [mode.numeric, mode.alphaNumeric, mode.ascii, mode.utf8]

/**
 * A QR code of `text`, made on the device with `lean-qr`. This module is
 * loaded only when a code is first wanted (`components/InvitationLink.tsx`),
 * so the encoder costs the pages that never show one nothing. It holds no
 * React and no screen, so loading it leaves how the rest of the app is split
 * into files as it was.
 *
 * The path is one rectangle per run of dark modules, offset by the quiet
 * zone a reader needs; medium error correction, which a camera reads from a
 * screen at arm's length.
 */
export function drawQr(text: string): QrDrawing {
  const code = generate(text, { minCorrectionLevel: correction.M, modes: MODES })
  const path = qrRuns(code)
    .map(({ x, y, width }) => `M${x + QR_QUIET_ZONE} ${y + QR_QUIET_ZONE}h${width}v1h-${width}z`)
    .join('')
  return { size: code.size + 2 * QR_QUIET_ZONE, path }
}
