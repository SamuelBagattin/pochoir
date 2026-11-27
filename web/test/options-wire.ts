/**
 * The browser's half of the packed options wire.
 *
 * Run it:
 *
 *     pnpm --dir web run test
 *     node --experimental-transform-types web/test/options-wire.ts
 *
 * It throws on the first disagreement and prints the count otherwise. No test
 * framework and no new dependency: Node 24 runs TypeScript directly, the two
 * modules this imports reach nothing else at run time, and every assertion
 * below is a comparison this file makes itself. `--experimental-transform-types`
 * rather than bare type stripping because `EngineError` uses constructor
 * parameter properties, which strip-only mode cannot erase.
 *
 * The cost is that `make check` does not run it: adding a Makefile target is
 * outside this session's scope, and the handover says so.
 *
 * WHY THIS FILE EXISTS. Offset 36's top byte was padding the engine ignored
 * until ADR-0010 gave it a colour it can choose itself; it is now a mode byte,
 * and **both sides have to agree about that or the failure is silent and
 * ugly** — a misread colour byte is a wrong-coloured mark, not an error. So
 * the exact bytes are pinned here and the same literal is pinned in
 * `crates/pochoir-wasm/src/options.rs`
 * (`the_bytes_the_browser_packs_by_default_are_the_automatic_choice`). If one
 * side moves, the hex string in that side's test has to move with it, and the
 * two strings are there to be read side by side.
 */

import {
  AUTOMATIC_COLOUR,
  AUTOMATIC_COLOUR_WORD,
  COLOUR_MODE_AUTOMATIC,
  COLOUR_MODE_FIXED,
  colourWord,
  DEFAULT_OPTIONS,
  OPTIONS_BYTES,
  OPTIONS_VERSION,
  packOptions,
  type MarkOptions,
} from '../src/engine/options.ts';
import { warningsOf, WARNING } from '../src/engine/module.ts';

let checks = 0;

function check(condition: boolean, what: string): void {
  if (!condition) throw new Error(`FAILED: ${what}`);
  checks += 1;
}

function equal(got: unknown, want: unknown, what: string): void {
  const a = String(got);
  const b = String(want);
  if (a !== b) throw new Error(`FAILED: ${what}\n  got      ${a}\n  expected ${b}`);
  checks += 1;
}

function hex(bytes: Uint8Array): string {
  return [...bytes].map((b) => b.toString(16).padStart(2, '0')).join('');
}

function wordAt(bytes: Uint8Array, offset: number): number {
  return new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength).getUint32(
    offset,
    true,
  );
}

// --- the colour field ------------------------------------------------------

equal(AUTOMATIC_COLOUR_WORD, 0x0100_0000, 'the automatic word is 0x01000000');
equal(COLOUR_MODE_FIXED, 0, 'mode 0 is an explicit colour');
equal(COLOUR_MODE_AUTOMATIC, 1, 'mode 1 is the automatic choice');

equal(colourWord(AUTOMATIC_COLOUR), 0x0100_0000, "'auto' packs as mode 1");
equal(colourWord('#1a1a1a'), 0x001a_1a1a, 'a fixed colour keeps a zero mode byte');
equal(colourWord('#000000'), 0, 'black is a colour, not the automatic choice');
equal(colourWord('#ffffff'), 0x00ff_ffff, 'white fills the low 24 bits and no more');
equal(colourWord('#E5E5E5'), 0x00e5_e5e5, 'upper case hex is accepted');

// A fallback here would be how a colour the user chose becomes a different
// colour in their document with nothing said.
for (const bad of ['', '#abc', '#12345', '#1234567', 'red', '#xyzxyz', 'auto ']) {
  let threw = false;
  try {
    colourWord(bad as `#${string}`);
  } catch {
    threw = true;
  }
  check(threw, `an unparseable colour (${JSON.stringify(bad)}) is refused, not defaulted`);
}

// --- the default request ---------------------------------------------------

equal(DEFAULT_OPTIONS.colour, AUTOMATIC_COLOUR, 'the default asks the engine to choose');

const defaults = packOptions({ ...DEFAULT_OPTIONS } as MarkOptions);
equal(defaults.length, OPTIONS_BYTES, 'the struct is 48 bytes');
equal(wordAt(defaults, 0), OPTIONS_VERSION, 'version 1');
equal(wordAt(defaults, 36), AUTOMATIC_COLOUR_WORD, 'offset 36 carries the automatic word');

/**
 * The whole default struct, little-endian, as the engine receives it.
 *
 * THE SAME LITERAL IS IN `crates/pochoir-wasm/src/options.rs`. Spelled out
 * rather than rebuilt from the constants, because a check that recomputes what
 * it is checking tests nothing.
 */
const DEFAULT_STRUCT_HEX =
  '01000000' + // 0  version 1
  '01000000' + // 4  quality mode: standard
  '00000000' + // 8  page sizing: cap to A4
  'c8000000' + // 12 dpi 200
  '00006040' + // 16 font size 3.5 per cent
  '0000f0c1' + // 20 angle -30 degrees
  '9a99993e' + // 24 opacity 0.30
  '8fc2753c' + // 28 blur ratio 0.015
  '01000000' + // 32 tile density: normal
  '00000001' + // 36 colour: 0x01000000, the automatic choice
  '0000000000000000'; // 40 seed 0: derive one from the document

equal(hex(defaults), DEFAULT_STRUCT_HEX, 'the default struct, byte for byte');

// The one byte this whole change is about. 0x01 sits in the high byte of a
// little-endian word, so it is the LAST of the four at offset 36 — which is
// exactly the kind of thing a hand-written packer gets backwards.
equal(defaults[39], COLOUR_MODE_AUTOMATIC, 'the mode byte is byte 39, not byte 36');
equal(defaults[36], 0, 'the colour bytes are zero under mode 1');
equal(defaults[37], 0, 'the colour bytes are zero under mode 1');
equal(defaults[38], 0, 'the colour bytes are zero under mode 1');

const pinned = packOptions({ ...DEFAULT_OPTIONS, colour: '#1a1a1a' } as MarkOptions);
equal(wordAt(pinned, 36), 0x001a_1a1a, 'a pinned colour packs as mode 0');
equal(
  hex(pinned.subarray(0, 36)),
  hex(defaults.subarray(0, 36)),
  'choosing a colour changes offset 36 and nothing else',
);
equal(
  hex(pinned.subarray(40)),
  hex(defaults.subarray(40)),
  'choosing a colour changes offset 36 and nothing else',
);

// --- the warning mask ------------------------------------------------------

equal(WARNING.MARK_SHRUNK, 1, 'bit 0 is the shrink warning');
equal(WARNING.POOR_CONTRAST, 2, 'bit 1 is the contrast warning');
equal(JSON.stringify(warningsOf(0)), '{"shrunk":false,"poorContrast":false}', 'no bits');
equal(JSON.stringify(warningsOf(1)), '{"shrunk":true,"poorContrast":false}', 'bit 0');
equal(JSON.stringify(warningsOf(2)), '{"shrunk":false,"poorContrast":true}', 'bit 1');
equal(JSON.stringify(warningsOf(3)), '{"shrunk":true,"poorContrast":true}', 'both bits');
// An unknown bit from a later engine must not read as a warning we know, and
// must not read as a failure either.
equal(
  JSON.stringify(warningsOf(0xffff_fffc)),
  '{"shrunk":false,"poorContrast":false}',
  'unknown bits are ignored rather than mistaken for ours',
);

console.log(`options-wire: ${checks} checks passed`);
