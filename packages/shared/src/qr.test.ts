import { expect, test } from 'vitest'

import { qrRuns } from './qr'

test('dark modules become one run per stretch of a row', () => {
  const rows = ['##.#', '....', '####', '.##.']
  const code = { size: 4, get: (x: number, y: number) => rows[y][x] === '#' }
  expect(qrRuns(code)).toEqual([
    { x: 0, y: 0, width: 2 },
    { x: 3, y: 0, width: 1 },
    { x: 0, y: 2, width: 4 },
    { x: 1, y: 3, width: 2 },
  ])
})
