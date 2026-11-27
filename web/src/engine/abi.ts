import { ENGINE_INTEGRITY, ENGINE_URL } from './engine-manifest';
import { packOptions, type MarkOptions } from './options';
import {
  EngineError,
  EngineFault,
  errorForStatus,
  STATUS,
  warningsOf,
  type MarkWarnings,
  type PochoirModule,
  type PreviewBitmap,
  type RenderedDocument,
} from './module';

/**
 * The raw WebAssembly ABI, exactly as agreed with the bindings work.
 *
 *   - Synchronous, called from a Web Worker, no wasm-bindgen: plain exports
 *     over linear memory.
 *   - The caller copies bytes in with `pochoir_alloc` and releases them with
 *     `pochoir_free`.
 *   - Both render entry points return a handle to a small result header
 *     `{ status, ptr, len, width, height }`, read through the
 *     `pochoir_result_*` accessors and released with `pochoir_result_free`.
 *   - The mark text is a separate UTF-8 buffer; the options are the packed
 *     little-endian struct in `options.ts`, not JSON.
 */
export interface PochoirExports {
  readonly memory: WebAssembly.Memory;

  pochoir_alloc(len: number): number;
  pochoir_free(ptr: number, len: number): void;

  pochoir_render_pdf(
    imgPtr: number,
    imgLen: number,
    textPtr: number,
    textLen: number,
    optsPtr: number,
    optsLen: number,
  ): number;

  pochoir_render_preview(
    imgPtr: number,
    imgLen: number,
    textPtr: number,
    textLen: number,
    optsPtr: number,
    optsLen: number,
    maxPx: number,
  ): number;

  pochoir_result_status(handle: number): number;
  pochoir_result_ptr(handle: number): number;
  pochoir_result_len(handle: number): number;
  pochoir_result_width(handle: number): number;
  pochoir_result_height(handle: number): number;
  /**
   * The warning bitmask: facts about a result the engine produced anyway.
   * See `MarkWarnings`. Mask for the bits we know; a later engine may add one.
   */
  pochoir_result_warnings(handle: number): number;
  pochoir_result_free(handle: number): void;
}

export async function instantiate(
  url: string = ENGINE_URL,
  integrity: string | undefined = ENGINE_INTEGRITY,
): Promise<PochoirExports> {
  const response = await fetch(url, integrity ? { integrity } : undefined);
  if (!response.ok) {
    throw new Error(`engine module ${url} responded ${response.status}`);
  }
  // `Content-Type: application/wasm` is required here: instantiateStreaming
  // rejects with a TypeError on anything else.
  const { instance } = await WebAssembly.instantiateStreaming(response, {});
  return instance.exports as unknown as PochoirExports;
}

/** A `PochoirModule` backed by the real engine. */
export class WasmModule implements PochoirModule {
  readonly implementation = 'wasm' as const;

  private constructor(private readonly ex: PochoirExports) {}

  static async load(url?: string, integrity?: string): Promise<WasmModule> {
    return new WasmModule(await instantiate(url, integrity));
  }

  async renderPreview(
    image: Uint8Array,
    text: string,
    options: MarkOptions,
    maxPx: number,
  ): Promise<PreviewBitmap> {
    return this.call(image, text, options, (ip, il, tp, tl, op, ol) =>
      this.ex.pochoir_render_preview(ip, il, tp, tl, op, ol, maxPx),
    ) as Promise<PreviewBitmap>;
  }

  async renderPdf(
    image: Uint8Array,
    text: string,
    options: MarkOptions,
  ): Promise<RenderedDocument> {
    return this.call(image, text, options, (ip, il, tp, tl, op, ol) =>
      this.ex.pochoir_render_pdf(ip, il, tp, tl, op, ol),
    ) as Promise<RenderedDocument>;
  }

  private async call(
    image: Uint8Array,
    text: string,
    options: MarkOptions,
    invoke: (
      imgPtr: number,
      imgLen: number,
      textPtr: number,
      textLen: number,
      optsPtr: number,
      optsLen: number,
    ) => number,
  ): Promise<PreviewBitmap | RenderedDocument> {
    // Packed before the boundary is crossed, so that a rejected option is an
    // ordinary argument error and not mistaken for a fault.
    const packed = packOptions(options);
    const encoded = new TextEncoder().encode(text);

    let img: Span | null = null;
    let txt: Span | null = null;
    let opts: Span | null = null;
    let handle = 0;
    try {
      img = this.copyIn(image);
      txt = this.copyIn(encoded);
      opts = this.copyIn(packed);

      handle = invoke(img.ptr, img.len, txt.ptr, txt.len, opts.ptr, opts.len);
      const status = this.ex.pochoir_result_status(handle);
      if (status !== STATUS.OK) throw errorForStatus(status, this.detailOf(handle));

      const ptr = this.ex.pochoir_result_ptr(handle);
      const len = this.ex.pochoir_result_len(handle);
      const width = this.ex.pochoir_result_width(handle);
      const height = this.ex.pochoir_result_height(handle);
      // Read after the status, and only on a success: the engine reports 0
      // here on every failure, and a caller that read it first would have
      // nothing to attach the warning to anyway.
      const warnings = this.warningsFor(handle);

      // A bitmap is discriminated from a document by its dimensions: a
      // finished PDF reports 0 x 0. Measured against the module, which returns
      // 1200 x 800 for a preview and 0 x 0 for a document.
      const isBitmap = width > 0 && height > 0;

      // RGBA8, row-major, no padding — so the length is implied by the
      // dimensions, and a mismatch means the engine and this wrapper disagree
      // about the pixel format. Checking it here turns what would otherwise be
      // a thrown `ImageData` constructor inside a Svelte effect, with a blank
      // preview and a console trace, into a reported failure.
      if (isBitmap && len !== width * height * 4) {
        throw new EngineError(
          'internal',
          `preview is ${len} bytes, expected ${width * height * 4} for ${width}x${height} RGBA8`,
        );
      }

      // Copy out before any further wasm call: growing the memory detaches the
      // existing ArrayBuffer and any view over it. `slice()` gives a buffer
      // with byteOffset 0, which is what makes the Uint8ClampedArray view over
      // it correct.
      const bytes = new Uint8Array(this.ex.memory.buffer, ptr, len).slice();
      return isBitmap
        ? { pixels: new Uint8ClampedArray(bytes.buffer), width, height, warnings }
        : { bytes, warnings };
    } catch (error) {
      // An EngineError is a status that came back cleanly; anything else
      // thrown between here and the module is the module misbehaving.
      if (error instanceof EngineError) throw error;
      throw new EngineFault(error);
    } finally {
      // A trapped instance traps again on every call, so the cleanup must not
      // be able to throw over the original error. Leaking a handle in a worker
      // that is about to be discarded costs nothing.
      try {
        if (handle !== 0) this.ex.pochoir_result_free(handle);
        if (opts) this.release(opts);
        if (txt) this.release(txt);
        if (img) this.release(img);
      } catch {
        /* the instance is already gone; the worker goes with it */
      }
    }
  }

  /**
   * The warning bits, masked to the ones this build understands.
   *
   * An older engine that does not export this accessor is not a failure: the
   * module is loaded from the same build as this bundle (ADR-0004 attests to
   * exactly that), but a missing export must degrade to "no warnings" rather
   * than throw a TypeError in the middle of a successful render and lose the
   * document with it.
   */
  private warningsFor(handle: number): MarkWarnings {
    const read = this.ex.pochoir_result_warnings;
    return warningsOf(typeof read === 'function' ? read.call(this.ex, handle) : 0);
  }

  /**
   * Reads `ptr`/`len` as a UTF-8 detail on a failing result, if the engine put
   * anything there.
   *
   * **Measured against the merged module: it never does.** `Outcome::failed`
   * carries `bytes: Vec::new()`, so every failure reports `len` 0 and this
   * returns `undefined`. It is kept, bounds-checked and free, because it is
   * the half of the payload mechanism that lives on this side: the Rust side
   * can start naming the character it refused (ADR-0007 Decision 6) without a
   * second change here. Until it does, the interface cannot tell the user
   * which character was the problem — see `errors.refused` in `i18n/fr.ts`.
   */
  private detailOf(handle: number): string | undefined {
    const ptr = this.ex.pochoir_result_ptr(handle);
    const len = this.ex.pochoir_result_len(handle);
    const size = this.ex.memory.buffer.byteLength;
    if (ptr === 0 || len === 0 || len > 4096 || ptr + len > size) return undefined;
    try {
      return new TextDecoder('utf-8', { fatal: true }).decode(
        new Uint8Array(this.ex.memory.buffer, ptr, len),
      );
    } catch {
      return undefined;
    }
  }

  private copyIn(bytes: Uint8Array): Span {
    const ptr = this.ex.pochoir_alloc(bytes.length);
    if (ptr === 0 && bytes.length > 0) {
      throw errorForStatus(STATUS.OUT_OF_MEMORY);
    }
    new Uint8Array(this.ex.memory.buffer, ptr, bytes.length).set(bytes);
    return { ptr, len: bytes.length };
  }

  private release(span: Span): void {
    if (span.len > 0) this.ex.pochoir_free(span.ptr, span.len);
  }
}

interface Span {
  ptr: number;
  len: number;
}
