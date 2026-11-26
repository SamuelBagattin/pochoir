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
