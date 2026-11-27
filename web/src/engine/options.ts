/**
 * The mark options, their defaults and their packed wire format.
 *
 * Every default and every range here comes from
 * `docs/adr/0002-watermark-algorithm-and-quality-modes.md` Decision 3, and from
 * `docs/adr/0006-image-page-size-cap.md` for the page sizing. None of it was
 * invented at the keyboard; if you want to change one, change the ADR.
 */

// `./module.ts` with the extension, not `./module`: it costs nothing here —
// Vite and `allowImportingTsExtensions` both take it — and it is what lets
// `node web/test/options-wire.ts` run this module's packer directly, with no
// bundler and no test framework. See that file.
import { EngineError } from './module.ts';

/**
 * ADR-0002 Decision 2 defines four modes. Only two of them have an engine:
 * ADR-0008 Decision 2 records that `compact` needs a pixel downsample and
 * `lossless` needs a PNG `/FlateDecode` image in the PDF, neither of which
 * exists, and that both return `BAD_ARGUMENT` rather than being approximated
 * onto a nearby quality number — because a user who asked for `lossless` and
 * silently received JPEG q92 has been lied to about their document.
 *
 * Confirmed by calling the module: mode 0 and mode 3 return status 1, mode 1
 * and mode 2 return status 0. So they are not offered here at all.
 */
export type QualityMode = 'standard' | 'high';
export type PageSizing = 'a4-max' | 'native';
export type TileDensity = 'low' | 'normal' | 'high';

/**
 * Ask the engine to measure the page and choose the mark's colour itself.
 *
 * ADR-0010's default, and ours. The failure it exists for: a dark-mode
 * screenshot watermarked in `#1a1a1a` at 0.30 opacity changes a pixel by
 * `0.30 x |0.102 - 0.114|` = 0.004, which is one level of 255 and less than
 * JPEG rounds away. The user gets their document back unmarked and believes
 * it is protected.
 */
export const AUTOMATIC_COLOUR = 'auto';

/** A colour for the mark: a fixed `#rrggbb`, or the engine's own choice. */
export type MarkColour = typeof AUTOMATIC_COLOUR | `#${string}`;

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
  /**
   * `#rrggbb`, or `'auto'` to have the engine choose from the page.
   *
   * An explicit colour is drawn exactly as asked, whatever the page looks
   * like — and warned about when it has poor contrast against it, which is a
   * fact only the engine can establish.
   */
  colour: MarkColour;
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
  // Not `#1a1a1a`. A default colour cannot be right for every page, and the
  // one that was there is wrong for exactly the documents this audience
  // photographs: a bank app, an insurance portal or an identity document held
  // on a phone, screenshotted in dark mode.
  colour: AUTOMATIC_COLOUR,
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

// The codes are the wire contract and keep their original numbering: 0 and 3
// remain reserved for `compact` and `lossless` should they ever be built.
const MODE_CODES: Record<QualityMode, number> = {
  standard: 1,
  high: 2,
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
 *  36  u32   colour: 0x00RRGGBB, or 0x01000000 for 'choose from the page'
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
  v.setUint32(36, colourWord(o.colour), true);
  v.setBigUint64(40, o.seed, true);
  return new Uint8Array(buf);
}

/** Top byte of offset 36 when the low 24 bits are the colour. */
export const COLOUR_MODE_FIXED = 0x00;
/** Top byte of offset 36 when the engine is to choose the colour. */
export const COLOUR_MODE_AUTOMATIC = 0x01;
/** The whole word that asks for the automatic choice. */
export const AUTOMATIC_COLOUR_WORD = COLOUR_MODE_AUTOMATIC * 0x01_00_00_00;

/**
 * Offset 36, packed.
 *
 * The top byte of this field was padding the engine ignored until ADR-0010
 * gave it a colour it can choose itself; it is now a mode byte, and the only
 * two words the engine accepts are `0x00RRGGBB` and `0x01000000`. Both sides
 * have to agree about that or the failure is silent and ugly — a misread
 * colour byte is a wrong-coloured mark, not an error — so the engine refuses
 * everything else, including `0x01RRGGBB`, which sets the automatic flag and
 * sends a colour and so means two things at once. `web/test/options-wire.ts`
 * pins what this function emits; `colour_of` in
 * `crates/pochoir-wasm/src/options.rs` pins what the engine accepts.
 *
 * An unparseable colour throws rather than falling back to a default, which is
 * what the old `colourToU32` did. A silent fallback is how a colour the user
 * chose becomes a different colour in their document with nothing said.
 */
export function colourWord(colour: MarkColour): number {
  if (colour === AUTOMATIC_COLOUR) return AUTOMATIC_COLOUR_WORD;
  const match = /^#([0-9a-f]{6})$/i.exec(colour.trim());
  if (!match) {
    throw new EngineError(
      'argument',
      `colour must be #rrggbb or '${AUTOMATIC_COLOUR}', got ${JSON.stringify(colour)}`,
    );
  }
  // Six hex digits cannot exceed 0xffffff, so the mode byte is zero by
  // construction rather than by masking.
  return parseInt(match[1]!, 16) >>> 0;
}

/** The three channels of a fixed colour. Not defined for `'auto'`. */
export function rgbOf(hex: `#${string}`): [number, number, number] {
  const n = colourWord(hex);
  return [(n >> 16) & 0xff, (n >> 8) & 0xff, n & 0xff];
}

// `relativeLuminance` and `LUMINANCE_WARN_ABOVE` were here, behind a warning
// that a mark above luminance 0.35 would not survive a photocopy. They are
// gone, with the engine's `colour_is_too_pale` that they mirrored: ADR-0010
// found that a one-sided test of a colour in the abstract is not a test about
// contrast at all, and it would now fire on `#e5e5e5` — the colour the engine
// itself chooses for a dark page, and the right one there. Whether a mark can
// be seen is a question about a particular page, and only the engine has the
// page. It answers it in the warning bits on the result.

/**
 * Clamp and trim the mark text to what the engine will accept.
 *
 * `isBlank` is a precondition, not a cosmetic test: `Mark::prepared()` refuses
 * an empty or whitespace-only mark with `BadArgument`, so a caller that sends
 * one is asking the engine for something it has already decided is invalid.
 * It lives here, beside the other rules about mark text, so there is one
 * answer rather than a `.trim()` in whichever component remembered.
 */
export function normaliseMarkText(raw: string): {
  text: string;
  truncatedLines: boolean;
  isBlank: boolean;
} {
  // NFC, because ADR-0002 requires it: macOS filenames and clipboard paste
  // arrive decomposed and the glyph rasteriser does no mark positioning.
  const nfc = raw.normalize('NFC');
  const lines = nfc.split(/\r\n|\r|\n/);
  const kept = lines.slice(0, LIMITS.textMaxLines);
  const text = kept.join('\n').slice(0, LIMITS.textMaxChars);
  return {
    text,
    truncatedLines: lines.length > LIMITS.textMaxLines,
    isBlank: text.trim().length === 0,
  };
}
