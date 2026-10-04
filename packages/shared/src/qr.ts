/*
 * Drawing a QR code of an invitation link, for someone in the same room to
 * scan with their phone's camera. The code itself is made on the device by
 * `lean-qr`, which each app loads only when the code is shown; nothing about
 * the link is sent anywhere to draw it. This turns the code's modules into
 * what both apps draw: one rectangle per run of dark modules in a row.
 */

/** A QR code as `lean-qr` makes it: `size` modules a side, `get(x, y)` dark. */
export interface QrModules {
  readonly size: number
  get(x: number, y: number): boolean
}

/** A run of dark modules: `width` of them from column `x` of row `y`. */
export interface QrRun {
  x: number
  y: number
  width: number
}

/** The quiet zone a reader needs around the code, in modules. */
export const QR_QUIET_ZONE = 4

/** Every run of dark modules, row by row, left to right. */
export function qrRuns(code: QrModules): QrRun[] {
  const runs: QrRun[] = []
  for (let y = 0; y < code.size; y += 1) {
    let start = -1
    for (let x = 0; x <= code.size; x += 1) {
      const dark = x < code.size && code.get(x, y)
      if (dark && start < 0) start = x
      if (!dark && start >= 0) {
        runs.push({ x: start, y, width: x - start })
        start = -1
      }
    }
  }
  return runs
}
