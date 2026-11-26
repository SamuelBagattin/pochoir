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

  errors: {
    title: 'Une erreur est survenue',
    timeout:
      'Le calcul a pris trop de temps et a été interrompu. Essayez une image plus petite, ou réduisez la densité.',
    crashed:
      'Le moteur s’est arrêté de façon inattendue. Rechargez la page ou essayez un autre fichier.',
    decode:
      'Ce fichier n’a pas pu être lu comme une image. Il est peut-être corrompu ou dans un format inattendu.',
    unsupported:
      'Votre navigateur ne prend pas en charge les fonctions nécessaires (WebAssembly dans un Web Worker). Mettez-le à jour pour utiliser Pochoir.',
    generic: 'Le calcul a échoué.',
  },

  stub: {
    tag: 'Démonstration',
    banner:
      'Le moteur Rust/WebAssembly n’est pas encore branché. Le filigrane affiché est calculé par le navigateur et le PDF produit est un fichier de démonstration. Le document ne quitte pas pour autant votre appareil.',
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
