import { EngineError, type PochoirModule, type PreviewBitmap } from './module';
import { rgbOf, type MarkOptions, type TileDensity } from './options';

/**
 * A STUB. It exists so the interface can be built, reviewed and exercised
 * before the Rust/WebAssembly engine is wired up, and it is deliberately
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
    return { pixels: data.data, width: canvas.width, height: canvas.height };
  }

  async renderPdf(
    image: Uint8Array,
    text: string,
    options: MarkOptions,
  ): Promise<Uint8Array> {
    const canvas = await composite(image, text, options, Number.POSITIVE_INFINITY);
    const quality = { compact: 0.75, standard: 0.85, high: 0.92, lossless: 0.95 }[
      options.mode
    ];
    const blob = await canvas.convertToBlob({ type: 'image/jpeg', quality });
    const jpeg = new Uint8Array(await blob.arrayBuffer());
    await pretendToWork(260, 700);
    return writeSingleImagePdf(jpeg, canvas.width, canvas.height, options.pageSizing);
  }
}

/* -------------------------------------------------------------------------
 * Compositing
 * ---------------------------------------------------------------------- */

const GAP_X: Record<TileDensity, number> = { low: 2.4, normal: 1.6, high: 1.05 };
const GAP_Y: Record<TileDensity, number> = { low: 6, normal: 4, high: 2.6 };

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
  const [r, g, b] = rgbOf(options.colour);

  ctx.save();
  ctx.globalAlpha = options.opacity;
  ctx.fillStyle = `rgb(${r} ${g} ${b})`;
  ctx.font = `600 ${fontPx}px ui-sans-serif, system-ui, -apple-system, "Segoe UI", sans-serif`;
  ctx.textAlign = 'center';
  ctx.textBaseline = 'middle';
  if (options.blurPx > 0 && 'filter' in ctx) {
    // Safari does not implement OffscreenCanvas filter; the real engine blurs
    // in Rust precisely because of that (ADR-0001). Here it is best effort.
    ctx.filter = `blur(${(options.blurPx * fontPx) / 48}px)`;
  }

  const lineH = fontPx * 1.25;
  const blockH = lineH * lines.length;
  const blockW = Math.max(...lines.map((l) => ctx.measureText(l).width));

  const stepX = blockW * (1 + GAP_X[options.density]);
  const stepY = blockH + lineH * GAP_Y[options.density] * 0.5;

  // The jitter is a quarter of a step, not half: ADR-0002 wants the tiling to
  // move between runs so two marked copies cannot be differenced, but a large
  // offset on a page that only fits one column can push the whole mark off the
  // sheet. Coverage first, then jitter.
  const rand = lcg(options.seed === 0n ? 0x9e3779b9n : options.seed);
  const jitterX = (rand() - 0.5) * stepX * 0.5;
  const jitterY = (rand() - 0.5) * stepY * 0.5;

  const diagonal = Math.hypot(width, height);
  ctx.translate(width / 2, height / 2);
  ctx.rotate((options.angleDeg * Math.PI) / 180);

  // One step of overscan on every side, so a rotated grid still covers the
  // corners and a partially clipped tile at the edge is intentional.
  const reach = diagonal / 2 + Math.max(stepX, stepY);
  let row = 0;
  for (let y = -reach; y <= reach; y += stepY, row++) {
    // Alternate rows are offset by half a step: it tiles better, and each mark
    // then has to be inpainted separately rather than as a regular lattice.
    const stagger = row % 2 === 0 ? 0 : stepX / 2;
    for (let x = -reach; x <= reach; x += stepX) {
      lines.forEach((line, i) => {
        ctx.fillText(
          line,
          x + stagger + jitterX,
          y + jitterY + (i - (lines.length - 1) / 2) * lineH,
        );
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

/** A tiny seeded PRNG, so the jitter is reproducible from the seed. */
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
