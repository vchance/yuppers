// Draws the Yuppers mark, two overlapping circles (you in yellow, the other
// party in blue) whose overlap is green, and writes every image made from it:
// the web app's favicon, its PNG fallback and Apple touch icon, and the
// mobile app's icon with its iOS dark and tinted versions, the layers of its
// Android adaptive icon, and the splash image. The mark and its colours are
// defined once, below (the Tandem tokens of apps/web/src/index.css and
// apps/mobile/src/lib/theme.ts), so the images cannot drift apart.
//
//   node apps/mobile/scripts/make-icons.mjs
//
// Needs `rsvg-convert` on the PATH (librsvg; `brew install librsvg` on macOS,
// `apt-get install librsvg2-bin` on Debian). No fonts are involved, so the
// output is the same on every machine.
//
// The Apple Wallet pass images (backend/assets/wallet) were made by the
// earlier version of this script, from the earlier mark, and are left as
// they are: the pass's own colours are set by the service
// (backend/src/wallet), and change with them.

import { spawnSync } from 'node:child_process'
import { writeFileSync } from 'node:fs'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'

const mobile = join(dirname(fileURLToPath(import.meta.url)), '..')
const assets = join(mobile, 'assets')
const webPublic = join(mobile, '..', 'web', 'public')

/** The mark's colours in light and in dark, and the ink tile it sits on. */
const LIGHT = { you: '#FFD23F', them: '#2F4BE0', agree: '#1B9A5C' }
const DARK = { you: '#F2C94C', them: '#7B93FF', agree: '#3FCB85' }
const INK = '#1A1B2E'
const NIGHT = '#111327'

// The mark on a 112 by 80 grid: two circles of radius 34 whose centres are 32
// apart, and the lens where they overlap.
const LENS = 'M56 10A34 34 0 0 1 56 70A34 34 0 0 1 56 10Z'

/** The mark, `width` wide, centred in a `size` square. */
function mark({ size, width, colors, rings = null }) {
  const scale = width / 112
  const x = (size - 112 * scale) / 2
  const y = (size - 80 * scale) / 2
  const shapes = rings
    ? // One colour, for the monochrome and tinted layers, which the system
      // tints and reads for shape only: the circles as rings, the overlap solid.
      `<circle cx="40" cy="40" r="34" fill="none" stroke="${rings}" stroke-width="7"/>
    <circle cx="72" cy="40" r="34" fill="none" stroke="${rings}" stroke-width="7"/>
    <path d="${LENS}" fill="${rings}"/>`
    : `<circle class="you" cx="40" cy="40" r="34" fill="${colors.you}"/>
    <circle class="them" cx="72" cy="40" r="34" fill="${colors.them}"/>
    <path class="agree" d="${LENS}" fill="${colors.agree}"/>`
  return `<g transform="translate(${x} ${y}) scale(${scale})">
    ${shapes}
  </g>`
}

function svg(size, body, { background = null, radius = 0, style = '' } = {}) {
  const fill = background
    ? `<rect class="tile" width="${size}" height="${size}" rx="${radius}" fill="${background}"/>`
    : ''
  return `<svg xmlns="http://www.w3.org/2000/svg" width="${size}" height="${size}" viewBox="0 0 ${size} ${size}">
  ${style}${fill}
  ${body}
</svg>
`
}

function png(path, size, source) {
  const result = spawnSync('rsvg-convert', ['--width', String(size), '--height', String(size), '--format', 'png'], {
    input: source,
    maxBuffer: 64 * 1024 * 1024,
  })
  if (result.error || result.status !== 0) {
    throw new Error(`rsvg-convert failed for ${path}: ${result.error ?? result.stderr}`)
  }
  writeFileSync(path, result.stdout)
  console.log(`wrote ${path.slice(join(mobile, '..', '..').length + 1)} (${size}x${size})`)
}

// The favicon: the mark on a rounded ink tile, which reads on light and dark
// browser tabs alike. In a dark browser its colours are the dark ones.
const faviconStyle = `<style>
    @media (prefers-color-scheme: dark) {
      .tile { fill: ${NIGHT} }
      .you { fill: ${DARK.you} }
      .them { fill: ${DARK.them} }
      .agree { fill: ${DARK.agree} }
    }
  </style>
  `
writeFileSync(
  join(webPublic, 'favicon.svg'),
  svg(64, mark({ size: 64, width: 54, colors: LIGHT }), { background: INK, radius: 14, style: faviconStyle }),
)
console.log('wrote apps/web/public/favicon.svg')

// For browsers without SVG icons, and the home screen icon on Apple devices,
// which is square: the system rounds its corners.
const tile = (size) => svg(size, mark({ size, width: size * 0.78, colors: LIGHT }), { background: INK })
png(join(webPublic, 'favicon-32.png'), 32, svg(32, mark({ size: 32, width: 27, colors: LIGHT }), { background: INK, radius: 7 }))
png(join(webPublic, 'apple-touch-icon.png'), 180, tile(180))

// The app icon, opaque and full-bleed as the stores require: the mark on the
// ink tile. iOS 18 and later also take a dark version, the dark-tuned mark
// with no tile of its own, and a tinted one, in grey for the system to tint.
png(join(assets, 'icon.png'), 1024, tile(1024))
png(join(assets, 'icon-dark.png'), 1024, svg(1024, mark({ size: 1024, width: 800, colors: DARK })))
png(join(assets, 'icon-tinted.png'), 1024, svg(1024, mark({ size: 1024, width: 800, rings: '#FFFFFF' }), { background: '#000000' }))

// Android's adaptive icon. The launcher masks it to its own shape and shows
// only the middle two thirds for sure, so the mark stays inside that.
png(join(assets, 'android-icon-background.png'), 512, svg(512, '', { background: INK }))
png(join(assets, 'android-icon-foreground.png'), 512, svg(512, mark({ size: 512, width: 300, colors: LIGHT })))
png(join(assets, 'android-icon-monochrome.png'), 432, svg(432, mark({ size: 432, width: 252, rings: '#FFFFFF' })))

// The splash images: the mark alone, with no tile, on the app's background in
// light and in dark (app.json, the expo-splash-screen plugin), each in that
// palette's colours. It is the picture the sign-in screen then opens with
// (src/components/Brand.tsx), so the app does not drop from its mark into a
// plain form. The mark fills the image's width, so `imageWidth` in app.json
// is the mark's own width on screen.
png(join(assets, 'splash-icon.png'), 1024, svg(1024, mark({ size: 1024, width: 1024, colors: LIGHT })))
png(join(assets, 'splash-icon-dark.png'), 1024, svg(1024, mark({ size: 1024, width: 1024, colors: DARK })))
