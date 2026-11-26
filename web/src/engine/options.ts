/**
 * The mark options, their defaults and their packed wire format.
 *
 * Every default and every range here comes from
 * `docs/adr/0002-watermark-algorithm-and-quality-modes.md` Decision 3, and from
 * `docs/adr/0006-image-page-size-cap.md` for the page sizing. None of it was
 * invented at the keyboard; if you want to change one, change the ADR.
 */

import { EngineError } from './module';

export type QualityMode = 'compact' | 'standard' | 'high' | 'lossless';
export type PageSizing = 'a4-max' | 'native';
export type TileDensity = 'low' | 'normal' | 'high';

export interface MarkOptions {
  /** ADR-0002 Decision 2. `standard` is the default. */
  mode: QualityMode;
  /** ADR-0006: cap an image input's page at A4, shrink only. */
  pageSizing: PageSizing;
  /** Only meaningful for PDF inputs (ADR-0002); carried so the ABI is stable. */
  dpi: number;
  /** Per cent of the page's shorter edge. */
  fontSizePct: number;
  angleDeg: number;
  opacity: number;
  /**
   * Blur sigma as a FRACTION OF THE FONT SIZE, not a pixel count.
   * ADR-0007 Decision 3 re-keyed it: ADR-0002's "0.6 px at render resolution,
   * scaled with DPI" contradicts itself, and at its own default it rounds to a
   * box radius of 0.28, which an integer-radius box blur turns into a no-op.
   */
  blurRatio: number;
  density: TileDensity;
  /** `#rrggbb`. */
  colour: string;
  /** Seeds the per-run position jitter so a run is reproducible (Decision 6). */
  seed: bigint;
}

export const DEFAULT_OPTIONS: Readonly<MarkOptions> = Object.freeze({
  mode: 'standard',
  pageSizing: 'a4-max',
  dpi: 200,
  fontSizePct: 3.5,
  angleDeg: -30,
  opacity: 0.3,
  blurRatio: 0.015,
  density: 'normal',
  colour: '#1a1a1a',
  seed: 0n,
});

export const LIMITS = {
  /**
   * ADR-0002 gives the angle's default as -30 and its range as "0-90". Read as
   * a magnitude, since the default is outside the literal interval: we allow
   * both directions of tilt. Flagged for the owner rather than silently chosen.
   */
  angleDeg: { min: -90, max: 90, step: 1 },
  fontSizePct: { min: 1, max: 10, step: 0.1 },
  opacity: { min: 0.05, max: 0.8, step: 0.01 },
  /** ADR-0007 Decision 3. `Mark::validate` rejects anything outside this. */
  blurRatio: { min: 0, max: 0.06, step: 0.001 },
  textMaxChars: 200,
  textMaxLines: 3,
} as const;

/**
 * The engine's `OutputOptions::MAX_DPI`. DPI is an `f32` there and a `u32` on
 * the wire, which is fine — a user interface has no fractional DPI to offer —
 * but the conversion has to be deliberate rather than a cast.
 */
export const MAX_DPI = 10_000;

/** 50 MB. A phone photo is 3-8 MB; this is generous and still bounded. */
export const MAX_INPUT_BYTES = 50 * 1024 * 1024;

export const ACCEPTED_TYPES = ['image/jpeg', 'image/png'] as const;

const MODE_CODES: Record<QualityMode, number> = {
  compact: 0,
  standard: 1,
  high: 2,
  lossless: 3,
};
const SIZING_CODES: Record<PageSizing, number> = { 'a4-max': 0, native: 1 };
const DENSITY_CODES: Record<TileDensity, number> = { low: 0, normal: 1, high: 2 };

/**
 * The packed options struct: fixed fields, little-endian, 48 bytes. NOT JSON —
 * there is no JSON parser in the core and there is not going to be one.
 *
 * off  type  field
 *   0  u32   version (1)
 *   4  u32   mode
 *   8  u32   page sizing
 *  12  u32   dpi
 *  16  f32   font size, per cent of shorter edge
 *  20  f32   angle, degrees
 *  24  f32   opacity, 0..1
 *  28  f32   blur sigma, as a fraction of the font size (ADR-0007 D3)
 *  32  u32   tile density
 *  36  u32   colour, 0x00RRGGBB
 *  40  u64   jitter seed
 */
// Stays 1 through the ADR-0007 blur change: nothing has shipped, so there is
// no deployed reader to keep compatible with.
export const OPTIONS_VERSION = 1;
export const OPTIONS_BYTES = 48;

export function packOptions(o: MarkOptions): Uint8Array {
  // `>>> 0` on a bad value does two things quietly, and both are worse than
  // refusing: it truncates a fractional DPI, and it wraps a negative or
  // out-of-range one into a plausible-looking large number that would surface
  // months later as a mysteriously enormous page rather than as an error.
  // Nothing in the interface can reach this today — which is exactly why it
  // is cheap to make it a refusal now.
  if (!Number.isInteger(o.dpi) || o.dpi <= 0 || o.dpi > MAX_DPI) {
    throw new EngineError(
      'argument',
      `dpi must be a whole number in 1..=${MAX_DPI}, got ${o.dpi}`,
    );
  }

  const buf = new ArrayBuffer(OPTIONS_BYTES);
  const v = new DataView(buf);
  v.setUint32(0, OPTIONS_VERSION, true);
  v.setUint32(4, MODE_CODES[o.mode], true);
  v.setUint32(8, SIZING_CODES[o.pageSizing], true);
  v.setUint32(12, o.dpi, true);
  v.setFloat32(16, o.fontSizePct, true);
  v.setFloat32(20, o.angleDeg, true);
  v.setFloat32(24, o.opacity, true);
  v.setFloat32(28, o.blurRatio, true);
  v.setUint32(32, DENSITY_CODES[o.density], true);
  v.setUint32(36, colourToU32(o.colour), true);
  v.setBigUint64(40, o.seed, true);
  return new Uint8Array(buf);
}

export function colourToU32(hex: string): number {
  const m = /^#?([0-9a-f]{6})$/i.exec(hex.trim());
  const n = m ? parseInt(m[1]!, 16) : 0x1a1a1a;
  return n >>> 0;
}

export function rgbOf(hex: string): [number, number, number] {
  const n = colourToU32(hex);
  return [(n >> 16) & 0xff, (n >> 8) & 0xff, n & 0xff];
}

/**
 * WCAG relative luminance, so we can honour ADR-0002's rule that we "warn when
 * the chosen colour's luminance is too high". A pale mark vanishes under
 * photocopying, which is the attacker's cheapest next move.
 */
export function relativeLuminance(hex: string): number {
  const channel = (c: number) => {
    const s = c / 255;
    return s <= 0.04045 ? s / 12.92 : Math.pow((s + 0.055) / 1.055, 2.4);
  };
  const [r, g, b] = rgbOf(hex);
  return 0.2126 * channel(r) + 0.7152 * channel(g) + 0.0722 * channel(b);
}

/** Above this the mark is too light to survive greyscale reproduction. */
export const LUMINANCE_WARN_ABOVE = 0.35;

/** Clamp and trim the mark text to what the engine will accept. */
export function normaliseMarkText(raw: string): { text: string; truncatedLines: boolean } {
  // NFC, because ADR-0002 requires it: macOS filenames and clipboard paste
  // arrive decomposed and the glyph rasteriser does no mark positioning.
  const nfc = raw.normalize('NFC');
  const lines = nfc.split(/\r\n|\r|\n/);
  const kept = lines.slice(0, LIMITS.textMaxLines);
  const text = kept.join('\n').slice(0, LIMITS.textMaxChars);
  return { text, truncatedLines: lines.length > LIMITS.textMaxLines };
}
