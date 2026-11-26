import type { MarkOptions } from './options';

/**
 * The seam. Everything above this interface — the worker, the client, the UI —
 * is written against it and does not know whether the real WebAssembly engine
 * or the browser-side stub is behind it.
 *
 * Swapping the stub for the real module is a one-line change in `select.ts`.
 */
export interface PochoirModule {
  readonly implementation: 'wasm' | 'stub';

  /**
   * Render a preview bitmap, with the longest edge capped at `maxPx`.
   * Returns RGBA8 pixels, row-major, no padding.
   */
  renderPreview(
    image: Uint8Array,
    text: string,
    options: MarkOptions,
    maxPx: number,
  ): Promise<PreviewBitmap>;

  /** Render the final document. */
  renderPdf(image: Uint8Array, text: string, options: MarkOptions): Promise<Uint8Array>;
}

export interface PreviewBitmap {
  pixels: Uint8ClampedArray;
  width: number;
  height: number;
}

/** The status codes the ABI's result header can carry. 0 is success. */
export const STATUS = {
  OK: 0,
  BAD_ARGUMENT: 1,
  DECODE_FAILED: 2,
  UNSUPPORTED_FORMAT: 3,
  OUT_OF_MEMORY: 4,
  INTERNAL: 5,
} as const;

export class EngineError extends Error {
  constructor(
    readonly kind: 'decode' | 'unsupported' | 'oom' | 'argument' | 'internal',
    message: string,
  ) {
    super(message);
    this.name = 'EngineError';
  }
}

export function errorForStatus(status: number): EngineError {
  switch (status) {
    case STATUS.BAD_ARGUMENT:
      return new EngineError('argument', 'engine rejected the arguments');
    case STATUS.DECODE_FAILED:
      return new EngineError('decode', 'the input could not be decoded');
    case STATUS.UNSUPPORTED_FORMAT:
      return new EngineError('unsupported', 'the input format is not supported');
    case STATUS.OUT_OF_MEMORY:
      return new EngineError('oom', 'the engine ran out of memory');
    default:
      return new EngineError('internal', `engine failed with status ${status}`);
  }
}
