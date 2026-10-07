#!/bin/sh
# Subsets the web app's fonts (apps/web/src/fonts/README.md).
#
# Reads the designers' full WOFF2 builds as first committed in 39f4067, never
# the subsets already in the tree, and writes the subsets over them. Needs
# `pyftsubset` from fonttools with brotli on PATH; in a throwaway venv:
#
#   python3 -m venv /tmp/fonttools && /tmp/fonttools/bin/pip install fonttools brotli
#   PATH="/tmp/fonttools/bin:$PATH" apps/web/scripts/subset-fonts.sh
#
# Then run `npm test -w @yuppers/web`: `build/fonts.test.ts` checks that every
# character the screens can show is in every subset.
set -eu

SOURCE_COMMIT=39f4067
# Basic Latin and Latin-1 (Spanish: á é í ó ú ñ ü ¿ ¡ « », and NBSP), Latin
# Extended-A, the modifier letters in the design notes (ʻ ʼ ˆ ˚ ˜), General
# Punctuation (curly quotes, dashes, ellipsis, bullet, the narrow no-break
# space dates are written with), Currency Symbols (€ and the rest the fonts
# have; $ £ ¥ are Latin-1), ™ and the minus sign.
UNICODES="U+0000-00FF,U+0100-017F,U+02BB-02BC,U+02C6,U+02DA,U+02DC,U+2000-206F,U+20A0-20CF,U+2122,U+2212"

repo=$(git rev-parse --show-toplevel)
fonts="$repo/apps/web/src/fonts"
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

for path in \
  gabarito/Gabarito-Bold.woff2 \
  gabarito/Gabarito-Black.woff2 \
  instrument-sans/InstrumentSans-Regular.woff2 \
  instrument-sans/InstrumentSans-Medium.woff2 \
  instrument-sans/InstrumentSans-Bold.woff2 \
  instrument-sans/InstrumentSans-Italic.woff2; do
  original="$work/$(basename "$path")"
  git -C "$repo" show "$SOURCE_COMMIT:apps/web/src/fonts/$path" > "$original"
  # The default layout features (among them kern, liga, calt, locl, mark,
  # mkmk) are kept; hinting is dropped, which browsers on the web do without.
  pyftsubset "$original" \
    --unicodes="$UNICODES" \
    --layout-features+=kern,liga \
    --no-hinting \
    --flavor=woff2 \
    --output-file="$fonts/$path"
  echo "$path: $(wc -c < "$original" | tr -d ' ') -> $(wc -c < "$fonts/$path" | tr -d ' ') bytes"
done
