import type { MarkOptions } from './options';

/** Messages the main thread sends into the worker. */
export type Request =
  | { id: number; kind: 'load'; image: ArrayBuffer }
  | { id: number; kind: 'preview'; text: string; options: MarkOptions; maxPx: number }
  | { id: number; kind: 'pdf'; text: string; options: MarkOptions };

/** Messages the worker sends back. Exactly one per request. */
export type Response =
  | { id: number; ok: true; kind: 'load'; implementation: 'wasm' | 'stub' }
  | {
      id: number;
      ok: true;
      kind: 'preview';
      pixels: ArrayBuffer;
      width: number;
      height: number;
    }
  | { id: number; ok: true; kind: 'pdf'; bytes: ArrayBuffer }
  | { id: number; ok: false; failure: Failure };

export interface Failure {
  kind: 'decode' | 'unsupported' | 'oom' | 'argument' | 'internal' | 'no-image';
  message: string;
  /**
   * The worker's instance faulted and it will not serve another request. The
   * client discards the worker on seeing this; the next call builds a fresh
   * one and re-uploads the image.
   */
  fatal?: boolean;
}
