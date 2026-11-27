import {
  EngineError,
  NO_WARNINGS,
  type PochoirModule,
  type PreviewBitmap,
  type RenderedDocument,
} from './module';
import {
  AUTOMATIC_COLOUR,
  rgbOf,
  type MarkColour,
  type MarkOptions,
  type TileDensity,
} from './options';

/**
 * A STUB, AND NO LONGER WIRED UP. `select.ts` returns the real engine; nothing
 * imports this file, so it is not in the bundle and cannot ship by accident.
 * It is kept because it is genuinely useful for working on the interface
 * without a Rust toolchain — re-point `select.ts` at it deliberately, never by
 * default.
 *
 * It existed so the interface could be built, reviewed and exercised
 * before the Rust/WebAssembly engine was wired up, and it is deliberately
 * written behind exactly the same `PochoirModule` interface as the real one:
 * replacing it is a one-line change in `select.ts` and touches nothing else.
 *
 * It is not the product. It composites with the browser's own canvas, which
 * means it resamples, it uses whatever font the OS has, and the PDF it emits
 * is a single re-encoded JPEG page rather than ADR-0002's metadata-clean,
 * deterministic, never-resampled output.
 *
 * What it does honour, because the UI has to be judged against the real thing:
 *   - the ADR-0002 controls and their defaults,
 *   - ADR-0006's A4 cap on the page,
 *   - seeded position jitter, so a given seed reproduces a given layout,
 *   - a render cost in the right order of magnitude, so the interface is
 *     exercised against latency rather than against an instant answer.
 *
 * It still never sends anything anywhere. The stub is local too.
 */
export class StubModule implements PochoirModule {
  readonly implementation = 'stub' as const;

  async renderPreview(
    image: Uint8Array,
    text: string,
    options: MarkOptions,
    maxPx: number,
  ): Promise<PreviewBitmap> {
    const canvas = await composite(image, text, options, maxPx);
    const ctx = context2d(canvas);
    const data = ctx.getImageData(0, 0, canvas.width, canvas.height);
    await pretendToWork(90, 260);
    // No warnings, ever: this stub does not measure the page, so it cannot
    // honestly raise either bit. One more reason it is not the product.
    return {
      pixels: data.data,
      width: canvas.width,
      height: canvas.height,
      warnings: NO_WARNINGS,
    };
  }

  async renderPdf(
    image: Uint8Array,
    text: string,
    options: MarkOptions,
  ): Promise<RenderedDocument> {
    const canvas = await composite(image, text, options, Number.POSITIVE_INFINITY);
    const quality = { standard: 0.85, high: 0.92 }[options.mode];
    const blob = await canvas.convertToBlob({ type: 'image/jpeg', quality });
    const jpeg = new Uint8Array(await blob.arrayBuffer());
    await pretendToWork(260, 700);
    return {
      bytes: writeSingleImagePdf(jpeg, canvas.width, canvas.height, options.pageSizing),
      warnings: NO_WARNINGS,
    };
  }
}

/**
 * The colour the stub draws in.
 *
 * `'auto'` is ADR-0010's "measure the page and choose", and measuring the page
 * is exactly what this stub does not do. It falls back to the dark candidate
 * and is therefore wrong on a dark page in precisely the way the real engine
 * no longer is — which is fine for laying out the interface and is why this
 * file must never be what ships.
 */
function stubColour(colour: MarkColour): [number, number, number] {
  return colour === AUTOMATIC_COLOUR ? [0x1a, 0x1a, 0x1a] : rgbOf(colour);
}

/* -------------------------------------------------------------------------
 * Compositing
 * ---------------------------------------------------------------------- */

/**
 * Both gaps are multiples of the LINE HEIGHT, per ADR-0007 Decision 5 — not of
 * the text width, which ADR-0002 specified and which is unstable once the mark
 * is as wide as the page. 1.6 across and 4 down at `normal`, doubled at `low`
 * and halved at `high`.
 */
const GAP_X: Record<TileDensity, number> = { low: 3.2, normal: 1.6, high: 0.8 };
const GAP_Y: Record<TileDensity, number> = { low: 8, normal: 4, high: 2 };

async function composite(
  image: Uint8Array,
  text: string,
  options: MarkOptions,
  maxPx: number,
): Promise<OffscreenCanvas> {
  let bitmap: ImageBitmap;
  try {
    // `slice()` so the view handed to the Blob owns its bytes.
    bitmap = await createImageBitmap(new Blob([image.slice().buffer]));
  } catch {
    throw new EngineError('decode', 'the browser could not decode this image');
  }

  const scale = Math.min(1, maxPx / Math.max(bitmap.width, bitmap.height));
  const width = Math.max(1, Math.round(bitmap.width * scale));
  const height = Math.max(1, Math.round(bitmap.height * scale));

  const canvas = new OffscreenCanvas(width, height);
  const ctx = context2d(canvas);
  ctx.drawImage(bitmap, 0, 0, width, height);
  bitmap.close();

  const lines = text.split('\n').filter((l) => l.trim().length > 0);
  if (lines.length === 0) return canvas;

  const fontPx = Math.max(6, (options.fontSizePct / 100) * Math.min(width, height));
  const [r, g, b] = stubColour(options.colour);

  ctx.save();
  ctx.globalAlpha = options.opacity;
  ctx.fillStyle = `rgb(${r} ${g} ${b})`;
  ctx.font = `600 ${fontPx}px ui-sans-serif, system-ui, -apple-system, "Segoe UI", sans-serif`;
  ctx.textAlign = 'center';
  ctx.textBaseline = 'middle';
  if (options.blurRatio > 0 && 'filter' in ctx) {
    // Sigma is a fraction of the font size (ADR-0007 Decision 3), so this is
    // resolution-independent in the same way the glyph size already is.
    // Safari does not implement OffscreenCanvas filter; the real engine blurs
    // in Rust precisely because of that (ADR-0001). Here it is best effort.
    ctx.filter = `blur(${options.blurRatio * fontPx}px)`;
  }

  const lineH = fontPx * 1.25;
  const blockH = lineH * lines.length;
  const blockW = Math.max(...lines.map((l) => ctx.measureText(l).width));

  const freeX = lineH * GAP_X[options.density];
  const freeY = lineH * GAP_Y[options.density];
  const stepX = blockW + freeX;
  const stepY = blockH + freeY;

  // The global phase, which is what stops two marked copies of one document
  // being differenced back to the clean original.
  const phase = lcg(options.seed === 0n ? 0x9e3779b9n : options.seed);
  const phaseX = (phase() - 0.5) * stepX;
  const phaseY = (phase() - 0.5) * stepY;

  const diagonal = Math.hypot(width, height);
  ctx.translate(width / 2, height / 2);
  ctx.rotate((options.angleDeg * Math.PI) / 180);

  // One step of overscan on every side, so a rotated lattice still covers the
  // corners and a partially clipped tile at the edge is intentional.
  const reach = diagonal / 2 + Math.max(stepX, stepY);
  const cols = Math.ceil((2 * reach) / stepX);
  const rows = Math.ceil((2 * reach) / stepY);
  const seed32 = Number(options.seed & 0xffffffffn) >>> 0;

  for (let row = 0; row <= rows; row++) {
    for (let col = 0; col <= cols; col++) {
      // ADR-0007 Decision 4: every tile is displaced independently, from a
      // generator keyed to its own lattice coordinates rather than drawn from
      // a running stream — so a page that needs one more row does not move
      // every other row. Capped at half the free gap, because two tiles that
      // meet composite over each other at twice the chosen opacity.
      const dx = (tileNoise(seed32, col, row, 0) - 0.5) * freeX;
      const dy = (tileNoise(seed32, col, row, 1) - 0.5) * freeY;
      const x = -reach + col * stepX + phaseX + dx;
      const y = -reach + row * stepY + phaseY + dy;
      lines.forEach((line, i) => {
        ctx.fillText(line, x, y + (i - (lines.length - 1) / 2) * lineH);
      });
    }
  }
  ctx.restore();
  return canvas;
}

function context2d(canvas: OffscreenCanvas): OffscreenCanvasRenderingContext2D {
  const ctx = canvas.getContext('2d', { willReadFrequently: true });
  if (!ctx) throw new EngineError('internal', 'no 2d context available');
  return ctx;
}

/**
 * A hash of (seed, column, row, axis) in [0, 1). Keyed to the tile's own
 * lattice coordinates, so each tile's displacement is independent of how many
 * tiles were drawn before it.
 */
function tileNoise(seed: number, col: number, row: number, axis: number): number {
  let h =
    (seed ^
      Math.imul(col + 1, 0x27d4eb2d) ^
      Math.imul(row + 1, 0x165667b1) ^
      Math.imul(axis + 1, 0x9e3779b1)) >>>
    0;
  h ^= h >>> 15;
  h = Math.imul(h, 0x2c1b3c6d) >>> 0;
  h ^= h >>> 12;
  h = Math.imul(h, 0x297a2d39) >>> 0;
  h ^= h >>> 15;
  return (h >>> 0) / 0x1_0000_0000;
}

/** A tiny seeded PRNG for the global phase, reproducible from the seed. */
function lcg(seed: bigint): () => number {
  let s = seed & 0xffffffffn;
  return () => {
    s = (s * 1664525n + 1013904223n) & 0xffffffffn;
    return Number(s) / 0x100000000;
  };
}

async function pretendToWork(min: number, max: number): Promise<void> {
  const ms = min + Math.random() * (max - min);
  await new Promise((resolve) => setTimeout(resolve, ms));
}

/* -------------------------------------------------------------------------
 * A minimal PDF, so the download is a real file you can open
 * ---------------------------------------------------------------------- */

const A4_PT = { w: 595.276, h: 841.89 };

function writeSingleImagePdf(
  jpeg: Uint8Array,
  pxW: number,
  pxH: number,
  sizing: 'a4-max' | 'native',
): Uint8Array {
  // ADR-0006: keep the aspect ratio, size at the effective resolution, then
  // shrink to fit A4 turned to match. Never enlarge.
  let ptW = (pxW / 200) * 72;
  let ptH = (pxH / 200) * 72;
  if (sizing === 'a4-max') {
    const bound = ptW > ptH ? { w: A4_PT.h, h: A4_PT.w } : A4_PT;
    const k = Math.min(1, bound.w / ptW, bound.h / ptH);
    ptW *= k;
    ptH *= k;
  }
  const w = ptW.toFixed(3);
  const h = ptH.toFixed(3);

  const out: Uint8Array[] = [];
  const offsets: number[] = [];
  let length = 0;
  const push = (bytes: Uint8Array) => {
    out.push(bytes);
    length += bytes.length;
  };
  const text = (s: string) => push(new TextEncoder().encode(s));
  const startObject = () => offsets.push(length);

  const content = `q ${w} 0 0 ${h} 0 0 cm /Im0 Do Q\n`;

  text('%PDF-1.7\n%\xE2\xE3\xCF\xD3\n');
  startObject();
  text('1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n');
  startObject();
  text('2 0 obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 >>\nendobj\n');
  startObject();
  text(
    `3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 ${w} ${h}] ` +
      `/Resources << /XObject << /Im0 4 0 R >> >> /Contents 5 0 R >>\nendobj\n`,
  );
  startObject();
  text(
    `4 0 obj\n<< /Type /XObject /Subtype /Image /Width ${pxW} /Height ${pxH} ` +
      `/ColorSpace /DeviceRGB /BitsPerComponent 8 /Filter /DCTDecode ` +
      `/Length ${jpeg.length} >>\nstream\n`,
  );
  push(jpeg);
  text('\nendstream\nendobj\n');
  startObject();
  text(`5 0 obj\n<< /Length ${content.length} >>\nstream\n${content}endstream\nendobj\n`);

  const xrefAt = length;
  let xref = `xref\n0 ${offsets.length + 1}\n0000000000 65535 f \n`;
  for (const o of offsets) xref += `${String(o).padStart(10, '0')} 00000 n \n`;
  // No /ID and no /Info: ADR-0002's metadata hygiene, as far as a stub can
  // honour it. The real writer derives /ID from the content.
  xref += `trailer\n<< /Size ${offsets.length + 1} /Root 1 0 R >>\nstartxref\n${xrefAt}\n%%EOF\n`;
  text(xref);

  const pdf = new Uint8Array(length);
  let at = 0;
  for (const chunk of out) {
    pdf.set(chunk, at);
    at += chunk.length;
  }
  return pdf;
}
