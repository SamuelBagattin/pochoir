import type { PreviewBitmap } from './module';
import type { MarkOptions } from './options';
import type { Failure, Request, Response } from './protocol';

export type ClientFailureKind = Failure['kind'] | 'timeout' | 'unsupported-browser';

/** `Omit` over a union collapses it to its common keys; this distributes. */
type WithoutId<T> = T extends { id: number } ? Omit<T, 'id'> : never;

export class ClientError extends Error {
  constructor(
    readonly kind: ClientFailureKind,
    message: string,
    /**
     * Carried through from the engine untouched: data to interpolate into a
     * French sentence, not a sentence. See `Failure.detail`.
     */
    readonly detail?: string,
  ) {
    super(message);
    this.name = 'ClientError';
  }
}

export interface ClientOptions {
  previewTimeoutMs?: number;
  pdfTimeoutMs?: number;
}

/**
 * The main thread's handle on the engine.
 *
 * It owns a worker it is willing to kill. A render that overruns its deadline
 * is not waited on: the worker is terminated, every in-flight request is
 * rejected, and the next call starts a fresh one and re-uploads the image. A
 * stuck render therefore costs the user a message, not a dead tab.
 */
export class EngineClient {
  #worker: Worker | null = null;
  #nextId = 1;
  #pending = new Map<
    number,
    { resolve: (r: Response) => void; reject: (e: unknown) => void; timer: number }
  >();
  #image: Uint8Array | null = null;
  #uploaded = false;
  #implementation: 'wasm' | 'stub' | null = null;
  readonly #previewTimeout: number;
  readonly #pdfTimeout: number;

  constructor(options: ClientOptions = {}) {
    this.#previewTimeout = options.previewTimeoutMs ?? 15_000;
    this.#pdfTimeout = options.pdfTimeoutMs ?? 60_000;
  }

  static get supported(): boolean {
    return (
      typeof Worker !== 'undefined' &&
      typeof WebAssembly === 'object' &&
      typeof OffscreenCanvas !== 'undefined'
    );
  }

  get implementation(): 'wasm' | 'stub' | null {
    return this.#implementation;
  }

  /** Hand the engine a new document. Does not render. */
  setImage(bytes: Uint8Array): void {
    this.#image = bytes;
    this.#uploaded = false;
  }

  async preview(
    text: string,
    options: MarkOptions,
    maxPx: number,
  ): Promise<PreviewBitmap> {
    await this.#ensureUploaded();
    const reply = await this.#send(
      { kind: 'preview', text, options, maxPx },
      this.#previewTimeout,
    );
    if (!reply.ok || reply.kind !== 'preview') throw failureOf(reply);
    return {
      pixels: new Uint8ClampedArray(reply.pixels),
      width: reply.width,
      height: reply.height,
    };
  }

  async pdf(text: string, options: MarkOptions): Promise<Uint8Array> {
    await this.#ensureUploaded();
    const reply = await this.#send({ kind: 'pdf', text, options }, this.#pdfTimeout);
    if (!reply.ok || reply.kind !== 'pdf') throw failureOf(reply);
    return new Uint8Array(reply.bytes);
  }

  /** Kill the worker and reject everything in flight. Safe to call at any time. */
  terminate(reason: ClientError = new ClientError('timeout', 'render cancelled')): void {
    this.#worker?.terminate();
    this.#worker = null;
    this.#uploaded = false;
    for (const [, entry] of this.#pending) {
      clearTimeout(entry.timer);
      entry.reject(reason);
    }
    this.#pending.clear();
  }

  dispose(): void {
    this.terminate(new ClientError('internal', 'client disposed'));
    this.#image = null;
  }

  async #ensureUploaded(): Promise<void> {
    if (!this.#image) throw new ClientError('no-image', 'no document loaded');
    if (this.#uploaded && this.#worker) return;
    // A copy, transferred: the client keeps its own bytes so it can re-upload
    // after the worker has been killed.
    const copy = this.#image.slice().buffer;
    const reply = await this.#send({ kind: 'load', image: copy }, 10_000, [copy]);
    if (!reply.ok || reply.kind !== 'load') throw failureOf(reply);
    this.#implementation = reply.implementation;
    this.#uploaded = true;
  }

  #ensureWorker(): Worker {
    if (this.#worker) return this.#worker;
    if (!EngineClient.supported) {
      throw new ClientError('unsupported-browser', 'this browser cannot run the engine');
    }
    const worker = new Worker(new URL('./worker.ts', import.meta.url), {
      type: 'module',
      name: 'pochoir-engine',
    });
    worker.onmessage = (event: MessageEvent<Response>) => {
      const message = event.data;
      const entry = this.#pending.get(message.id);
      if (entry) {
        clearTimeout(entry.timer);
        this.#pending.delete(message.id);
        entry.resolve(message);
      }
      // Settle the request that faulted *first*, so its caller gets the real
      // reason, then discard the worker. The recovery path is the one that
      // already exists for a timeout: the next call builds a fresh worker and
      // re-uploads the image.
      if (!message.ok && message.failure.fatal) {
        this.terminate(
          new ClientError('internal', 'the engine instance faulted and was discarded'),
        );
      }
    };
    worker.onerror = () => {
      this.terminate(new ClientError('internal', 'the engine worker stopped'));
    };
    this.#worker = worker;
    return worker;
  }

  #send(
    body: WithoutId<Request>,
    timeoutMs: number,
    transfer: Transferable[] = [],
  ): Promise<Response> {
    const worker = this.#ensureWorker();
    const id = this.#nextId++;
    return new Promise<Response>((resolve, reject) => {
      const timer = self.setTimeout(() => {
        this.#pending.delete(id);
        this.terminate(new ClientError('timeout', `request ${id} timed out`));
        reject(new ClientError('timeout', `request ${id} timed out`));
      }, timeoutMs);
      this.#pending.set(id, { resolve, reject, timer });
      worker.postMessage({ ...body, id } as Request, transfer);
    });
  }
}

function failureOf(reply: Response): ClientError {
  if (!reply.ok) {
    return new ClientError(reply.failure.kind, reply.failure.message, reply.failure.detail);
  }
  return new ClientError('internal', 'unexpected reply from the engine');
}
