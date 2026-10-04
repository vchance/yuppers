import jsQR from 'jsqr';
import { StyleSheet } from 'react-native';

/** A node of the rendered tree, as far as reading a QR code looks at it. */
interface Node {
  type: unknown;
  props: Record<string, unknown>;
  children: (Node | string)[];
}

interface Box {
  left?: number;
  top?: number;
  width?: number;
  height?: number;
}

const boxOf = (node: Node) => (StyleSheet.flatten(node.props.style as never) ?? {}) as Box;

/**
 * Reads a QR code drawn by `components/QrCode.tsx` the way a phone's camera
 * would: the views it is drawn with are laid back out as pixels, as the
 * device would paint them, and handed to a decoder. `null` when nothing can
 * be read.
 */
export function readQr(code: Node): string | null {
  const { width: size = 0 } = boxOf(code);
  const pixels = Math.round(size);
  const dark = new Uint8Array(pixels * pixels);
  const walk = (node: Node | string) => {
    if (typeof node === 'string') return;
    if (node !== code && typeof node.type === 'string') {
      const { left = 0, top = 0, width = 0, height = 0 } = boxOf(node);
      for (let y = top; y < top + height; y += 1) {
        for (let x = left; x < left + width; x += 1) dark[y * pixels + x] = 1;
      }
    }
    for (const child of node.children ?? []) walk(child);
  };
  walk(code);
  const data = new Uint8ClampedArray(pixels * pixels * 4);
  for (let index = 0; index < pixels * pixels; index += 1) {
    const value = dark[index] ? 0 : 255;
    data.set([value, value, value, 255], index * 4);
  }
  return jsQR(data, pixels, pixels)?.data ?? null;
}
