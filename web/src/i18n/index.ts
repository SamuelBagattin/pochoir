import { fr, type Strings } from './fr';

/**
 * The locale registry. Today it has one entry; adding English means adding
 * `en` here and a language switch, not touching any component.
 */
export const locales = { fr } satisfies Record<string, Strings>;

export type LocaleCode = keyof typeof locales;

export const defaultLocale: LocaleCode = 'fr';

/** The active string catalogue. Components import this and nothing else. */
export const t: Strings = locales[defaultLocale];

export type { Strings };

/** Today's date, formatted for the active locale — used by the presets. */
export function todayFormatted(now: Date = new Date()): string {
  return new Intl.DateTimeFormat(t.locale, {
    day: '2-digit',
    month: '2-digit',
    year: 'numeric',
  }).format(now);
}

/** A file size in the active locale, for the file summary line. */
export function formatBytes(bytes: number): string {
  const mb = bytes / (1024 * 1024);
  if (mb >= 1) {
    return `${new Intl.NumberFormat(t.locale, { maximumFractionDigits: 1 }).format(mb)} Mo`;
  }
  const kb = Math.max(1, Math.round(bytes / 1024));
  return `${new Intl.NumberFormat(t.locale).format(kb)} ko`;
}

/** 0.30 -> "0,30" in French. Used by the opacity readout. */
export function formatNumber(value: number, digits = 2): string {
  return new Intl.NumberFormat(t.locale, {
    minimumFractionDigits: digits,
    maximumFractionDigits: digits,
  }).format(value);
}

/** How many offending characters we will name before summarising the rest. */
const CHARACTER_LIST_CAP = 5;

/**
 * Turn the engine's unsupported-character payload into something that can sit
 * inside a sentence.
 *
 * Formatting, deliberately kept out of the string catalogue: an English
 * catalogue would want the same list built with " and " instead of " et ".
 *
 * Four things happen here, and the last is the one that is easy to miss:
 *
 *  1. **De-duplicate**, preserving first-seen order. The engine already does,
 *     but somebody who typed the same symbol four times should be told about
 *     one character whichever side does it.
 *  2. **Cap the list**, so a pasted line of emoji does not become a paragraph.
 *  3. **French enumeration** — guillemets with narrow no-break spaces, commas,
 *     and « et » before the last.
 *  4. **Prefix a combining mark with U+25CC DOTTED CIRCLE.** ADR-0007
 *     Decision 6 reports a leftover combining mark like any other unsupported
 *     character, and the engine really does return a bare U+0301. Dropped
 *     into a sentence it would attach itself to the closing guillemet and
 *     render as a corrupt quotation mark rather than as the thing being
 *     named. The dotted circle is the standard way to show one in isolation.
 */
export function formatCharacterList(payload: string): { list: string; count: number } {
  const distinct = [...new Set([...payload])];
  const shown = distinct.slice(0, CHARACTER_LIST_CAP);
  const quoted = shown.map((character) => {
    const printable = /\p{M}/u.test(character) ? `\u25CC${character}` : character;
    return `\u00AB\u202F${printable}\u202F\u00BB`;
  });

  const hidden = distinct.length - shown.length;
  if (hidden > 0) {
    const others = hidden === 1 ? '1 autre' : `${hidden} autres`;
    return { list: `${quoted.join(', ')} et ${others}`, count: distinct.length };
  }
  if (quoted.length === 1) return { list: quoted[0]!, count: 1 };
  return {
    list: `${quoted.slice(0, -1).join(', ')} et ${quoted.at(-1)}`,
    count: distinct.length,
  };
}
