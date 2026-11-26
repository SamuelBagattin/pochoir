import { WasmModule } from './abi';
import type { PochoirModule } from './module';

/**
 * Which implementation sits behind the seam.
 *
 * It is the real Rust/WebAssembly engine, and it is the only thing this
 * function has ever been meant to return. `module-stub.ts` is still in the
 * tree because it is useful for working on the interface without a Rust
 * toolchain, but **nothing imports it any more**, so it is not in the bundle
 * and cannot be shipped by accident. That matters more than tidiness here: a
 * privacy tool that could silently serve a browser-canvas path instead of the
 * audited engine would be the wrong kind of surprise.
 *
 * The module is instantiated once per worker, lazily, on the first request.
 */
export async function loadModule(): Promise<PochoirModule> {
  return WasmModule.load();
}
