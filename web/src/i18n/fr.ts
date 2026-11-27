/**
 * French strings. French first because the market, the search traffic and the
 * presets are all French (decided by Samuel).
 *
 * ADDING A LANGUAGE IS AN ADDITION, NOT A REWRITE: copy this file to `en.ts`,
 * translate the values, add it to the `locales` map in `./index.ts`. Nothing
 * else in the codebase holds a user-visible string — if you are about to type
 * a French word inside a component, it belongs here instead.
 */

export const fr = {
  locale: 'fr-FR',
  htmlLang: 'fr',

  site: {
    name: 'Pochoir',
    tagline: 'Filigraner un document sans le téléverser',
  },

  nav: {
    tool: 'Le filigrane',
    privacy: 'Confidentialité',
    source: 'Code source',
    skipToContent: 'Aller au contenu principal',
  },

  home: {
    title: 'Filigranez vos papiers sans les envoyer à personne',
    lede:
      'Déposez une photo ou un scan de votre pièce d’identité, écrivez à qui vous la destinez, et récupérez un PDF filigrané. Le calcul a lieu dans votre navigateur : le fichier ne quitte pas votre appareil.',
    note:
      'Formats acceptés : JPEG et PNG. Le PDF en entrée et les documents à plusieurs pages ne sont pas encore pris en charge.',
  },

  drop: {
    groupTitle: 'Le document',
    prompt: 'Déposez votre document ici',
    hint: 'ou choisissez un fichier sur votre appareil. JPEG ou PNG, jusqu’à 50 Mo.',
    pick: 'Choisir un fichier',
    change: 'Changer de fichier',
    remove: 'Retirer le fichier',
    inputLabel: 'Document à filigraner (JPEG ou PNG)',
    dropping: 'Relâchez pour déposer le document',
    rejectedType:
      'Ce format n’est pas pris en charge. Choisissez une image JPEG ou PNG.',
    rejectedSize: 'Ce fichier dépasse 50 Mo.',
    rejectedMultiple:
      'Un seul document à la fois pour l’instant : le premier fichier a été retenu.',
    loaded: 'Document chargé',
  },

  mark: {
    groupTitle: 'Le filigrane',
    textLabel: 'Texte du filigrane',
    textHint: '200 caractères maximum, 3 lignes au plus.',
    textPlaceholder: 'Document destiné exclusivement à …',
    charactersRemaining: (n: number) =>
      `${n} caractère${n > 1 ? 's' : ''} restant${n > 1 ? 's' : ''}`,
    tooManyLines: 'Le filigrane est limité à 3 lignes ; les suivantes sont ignorées.',
    presetsLabel: 'Modèles de texte',
    presetsHint: 'Insère la mention et la date du jour.',
    presetApplied: (label: string) => `Modèle « ${label} » appliqué.`,
  },

  /**
   * One-tap presets, decided by Samuel. Each inserts
   * « Document destiné exclusivement à <purpose> — <date du jour> ».
   */
  presets: [
    { id: 'location', label: 'Location', purpose: 'une demande de location' },
    { id: 'banque', label: 'Banque', purpose: 'une démarche bancaire' },
    { id: 'mutuelle', label: 'Mutuelle', purpose: 'une démarche auprès de ma mutuelle' },
    { id: 'employeur', label: 'Employeur', purpose: 'mon employeur' },
    { id: 'administration', label: 'Administration', purpose: 'une démarche administrative' },
  ],
  presetTemplate: (purpose: string, date: string) =>
    `Document destiné exclusivement à ${purpose} — ${date}`,

  controls: {
    groupTitle: 'Apparence',
    opacity: 'Opacité',
    angle: 'Inclinaison',
    fontSize: 'Taille du texte',
    fontSizeHint: 'En pourcentage du petit côté de la page.',
    density: 'Densité',
    densityLow: 'Faible',
    densityNormal: 'Normale',
    densityHigh: 'Forte',
    colour: 'Couleur',
    colourHint: 'Un gris foncé survit à la photocopie et au noir et blanc.',
    colourTooLight:
      'Cette couleur est claire : le filigrane risque de disparaître à la photocopie ou en noir et blanc.',
    reset: 'Rétablir les valeurs par défaut',
    degrees: '°',
    percent: '%',
  },

  preview: {
    title: 'Aperçu',
    empty:
      'L’aperçu s’affichera ici dès qu’un document sera chargé. Il se met à jour pendant que vous réglez le filigrane.',
    canvasLabel: (name: string) => `Aperçu filigrané de ${name}`,
    statusIdle: 'En attente d’un document',
    statusRendering: 'Calcul de l’aperçu',
    statusReady: 'Aperçu à jour',
    statusAnnounce: 'Aperçu à jour.',
    statusFailed: 'Échec du calcul',
    dimensions: (w: number, h: number) => `${w} × ${h} px`,
  },

  actions: {
    groupTitle: 'Export',
    download: 'Télécharger le PDF',
    preparing: 'Préparation du PDF…',
    downloadReady: (name: string) => `PDF prêt : ${name}`,
    cancel: 'Annuler',
    suffix: '-filigrane',
  },

  /**
   * One message per engine status. Two rules they are written to, both of
   * which matter more than they look:
   *
   *   - Say whose fault it is, accurately. Someone who has just handed a
   *     watermarking tool a photograph of their passport and received an
   *     error will assume they did something wrong. Where they did not, the
   *     message says so.
   *   - Name the next action, or admit there is not one. "Une erreur est
   *     survenue" is the message these replace.
   *
   * Never blame the document: "ce fichier est invalide" reads as a verdict on
   * the user's identity papers, where "n'a pas pu être lu" is a verdict on
   * our decoder, which is what it actually is.
   */
  errors: {
    title: 'Une erreur est survenue',

    /**
     * Status 1, BadArgument. A parameter out of range — the payload names it,
     * `dpi` for instance. No control can produce one, so if a user ever sees
     * this the interface sent something it had no business sending, and the
     * sentence says so rather than implying they did something wrong. A mark
     * the font cannot draw has its own status and is not this.
     */
    refused:
      'Le moteur a refusé un réglage envoyé par l’interface. C’est un défaut de notre côté, pas un problème avec votre document. Rechargez la page et réessayez.',

    /**
     * Status 6, TextDoesNotFit. ADR-0007 Decision 2 wraps, then shrinks to the
     * 1 % floor, then refuses — so this is reached by text that cannot be
     * *broken*, not by text that is merely long. Saying "trop long" would send
     * someone to delete words when the fix is a space.
     */
    textDoesNotFit:
      'Ce texte ne peut pas être découpé en lignes assez courtes pour tenir sur la page, même à la plus petite taille. Ajoutez des espaces, ou raccourcissez-le.',

    /**
     * Status 7, UnsupportedCharacter, with the characters named. The article,
     * the verb and the pronoun all agree, so the plural is a second sentence
     * rather than an `(s)`. French takes the singular at n = 1 only, and
     * n >= 1 always holds here, so two branches are correct and
     * `Intl.PluralRules` is not needed.
     */
    unsupportedCharacters: (list: string, count: number) =>
      count === 1
        ? `Le caractère ${list} n’est pas pris en charge par la police de Pochoir. Retirez-le du texte du filigrane.`
        : `Les caractères ${list} ne sont pas pris en charge par la police de Pochoir. Retirez-les du texte du filigrane.`,

    /**
     * Status 7 with nothing in the payload. Drops the instruction to remove
     * "it", because without the payload we cannot say which one, and telling
     * somebody to remove a character we decline to name is worse than
     * describing the class.
     */
    unsupportedCharactersUnnamed:
      'Le texte du filigrane contient un caractère que la police de Pochoir ne sait pas dessiner. Les emoji et les alphabets non latins ne sont pas pris en charge.',

    /** Status 2, DecodeFailed. */
    decode:
      'Ce fichier n’a pas pu être lu comme une image. Il est peut-être incomplet ou endommagé. Essayez une autre copie, ou réexportez-la depuis l’application qui l’a produite.',

    /** Status 3, UnsupportedFormat. Reachable past the picker's MIME filter. */
    unsupportedFormat:
      'Ce format d’image n’est pas pris en charge. Pochoir accepte le JPEG et le PNG.',

    /** Status 4, OutOfMemory. Not "réduisez la densité": that would be false. */
    outOfMemory:
      'Le moteur a manqué de mémoire pour traiter cette image. Elle est probablement trop grande pour cet appareil : essayez une version de plus petite définition.',

    /** Status 5, Internal — and the worker discarding a faulted instance. */
    crashed:
      'Le moteur s’est arrêté de façon inattendue. Rechargez la page. Si cela se reproduit avec le même document, c’est un défaut de notre côté.',

    /** The client's own deadline, not a status. */
    timeout:
      'Le calcul a pris trop de temps et a été interrompu. Essayez une image plus petite, ou réduisez la densité.',

    /** No worker, or no WebAssembly, or the module would not instantiate. */
    unsupportedBrowser:
      'Votre navigateur ne prend pas en charge les fonctions nécessaires (WebAssembly dans un Web Worker). Mettez-le à jour pour utiliser Pochoir.',

    /** Should be unreachable: the export button is disabled without a file. */
    noImage: 'Choisissez d’abord un document.',

    generic: 'Le calcul a échoué.',
  },

  notFound: {
    title: 'Cette page n’existe pas',
    description: 'La page demandée n’existe pas sur Pochoir.',
    lede:
      'L’adresse que vous avez suivie ne correspond à aucune page de Pochoir. Elle a peut-être été mal saisie, ou le lien que vous avez suivi est ancien.',
    backToTool: 'Filigraner un document',
  },

  privacy: {
    title: 'Ce qui reste sur votre appareil',
    lede:
      'Pochoir ne possède pas de serveur capable de recevoir votre document. Ce n’est pas une promesse de notre part : c’est une conséquence de la façon dont le site est construit, et vous pouvez le vérifier vous-même.',
    howTitle: 'Comment c’est construit',
    checkTitle: 'Comment le vérifier vous-même',
    notTitle: 'Ce que cela ne protège pas',
    limitsTitle: 'Ce que nous ne promettons pas',
  },

  footer: {
    builtBy: 'Pochoir est un logiciel libre.',
    licence: 'Interface sous licence AGPL-3.0-or-later ; moteur sous MIT ou Apache-2.0.',
    noTracking: 'Aucune mesure d’audience, aucun cookie, aucune requête vers un tiers.',
  },
} as const;

export type Strings = typeof fr;
