# Web fonts

Gabarito 700 and 900 (naipefoundry/gabarito at 1f3fb39) and Instrument Sans 400, 500, 700 and italic 400 (Instrument/instrument-sans at 7fa2230), under the SIL Open Font License 1.1, whose text is beside each family as `OFL.txt`. Neither family has a Reserved Font Name, so the subsets keep their names.

The files here are subsets of the designers' full WOFF2 builds, which were committed in 39f4067. They are made by `apps/web/scripts/subset-fonts.sh`, which reads those originals from that commit, never the subsets, and writes over these files:

```sh
python3 -m venv /tmp/fonttools && /tmp/fonttools/bin/pip install fonttools brotli
PATH="/tmp/fonttools/bin:$PATH" apps/web/scripts/subset-fonts.sh
```

For each file it runs:

```sh
pyftsubset <original>.woff2 \
  --unicodes="U+0000-00FF,U+0100-017F,U+02BB-02BC,U+02C6,U+02DA,U+02DC,U+2000-206F,U+20A0-20CF,U+2122,U+2212" \
  --layout-features+=kern,liga --no-hinting --flavor=woff2 --output-file=<file>.woff2
```

These subsets were made with fonttools 4.66.1. The ranges are Basic Latin and Latin-1 (Spanish), Latin Extended-A, a few modifier letters, General Punctuation, Currency Symbols, ™ and the minus sign. Each font keeps only the characters it actually has: Instrument Sans has no narrow no-break space, for example. The default OpenType features are kept, among them `kern`, `liga`, `calt`, `locl`, `mark` and `mkmk`. Hinting is dropped.

`apps/web/build/fonts.test.ts` reads each file's character map and fails if the wording, the web app's text, a stylesheet's `content`, or money or a date as a language formats it uses a character the font lacks. When it fails, add the range to `UNICODES` in the script and run the script again.

The mobile app's TrueType files (`apps/mobile/assets/fonts`) are the full builds, not subset.
