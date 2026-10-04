import jsQR from 'jsqr'

/**
 * Reads a QR code drawn by `components/QrCode.tsx` the way a phone's camera
 * would: the drawing is turned back into pixels, four to a module, and
 * handed to a decoder. jsdom paints nothing, so the pixels come from the
 * SVG's own path rather than a screenshot. `null` when nothing can be read.
 */
export function readQr(svg: SVGSVGElement): string | null {
  const size = Number(svg.getAttribute('viewBox')!.split(' ')[2])
  const path = svg.querySelector('path')!.getAttribute('d')!
  const dark = new Set<number>()
  for (const [, x, y, width] of path.matchAll(/M(\d+) (\d+)h(\d+)v1h-\d+z/g)) {
    for (let step = 0; step < Number(width); step += 1) {
      dark.add(Number(y) * size + Number(x) + step)
    }
  }
  const scale = 4
  const pixels = size * scale
  const data = new Uint8ClampedArray(pixels * pixels * 4)
  for (let row = 0; row < pixels; row += 1) {
    for (let column = 0; column < pixels; column += 1) {
      const module = Math.floor(row / scale) * size + Math.floor(column / scale)
      const value = dark.has(module) ? 0 : 255
      data.set([value, value, value, 255], (row * pixels + column) * 4)
    }
  }
  return jsQR(data, pixels, pixels)?.data ?? null
}
