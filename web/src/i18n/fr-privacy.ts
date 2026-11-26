/**
 * The long-form confidentiality page, kept out of `fr.ts` because prose and
 * interface labels have different lifecycles, and kept out of the component
 * for the same reason every other string is: adding English must be adding a
 * file, not rewriting a page.
 *
 * The tone rule for this page: we would rather be trusted than impressive.
 * Nothing here may overstate what a visible watermark does. If you are about
 * to write a sentence that sounds reassuring and that you cannot defend,
 * delete it — docs/adr/0002 Decision 7 is the honest version and this page is
 * downstream of it.
 */

export interface PrivacySection {
  id: string;
  heading: string;
  paragraphs?: string[];
  list?: string[];
  facts?: { term: string; detail: string }[];
}

export const frPrivacy = {
  title: 'Ce qui reste sur votre appareil',
  lede:
    'Pochoir ne dispose d’aucun serveur capable de recevoir votre document. Ce n’est pas une promesse commerciale : c’est une conséquence de la façon dont le site est construit, et vous pouvez le vérifier vous-même en quelques minutes.',

  sections: [
    {
      id: 'construction',
      heading: 'Comment c’est construit',
      paragraphs: [
        'Pochoir est un site statique : des fichiers HTML, CSS, JavaScript et un module WebAssembly, servis tels quels. Il n’y a ni application serveur, ni base de données, ni compte, ni file de traitement. Le filigrane est calculé par votre navigateur, sur votre machine, dans un Web Worker.',
        'Conséquence directe : il n’existe nulle part de code capable de recevoir votre pièce d’identité, donc nulle part où elle pourrait être conservée, journalisée ou oubliée.',
      ],
      facts: [
        {
          term: 'Aucun téléversement',
          detail:
            'Le fichier que vous déposez est lu par le navigateur et n’est jamais envoyé sur le réseau.',
        },
        {
          term: 'Aucune mesure d’audience',
          detail:
            'Pas de Google Analytics, pas de Matomo, pas de pixel, pas de rapport d’erreurs automatique.',
        },
        {
          term: 'Aucun cookie',
          detail:
            'Le site n’en dépose aucun et n’a donc pas de bandeau de consentement à vous imposer.',
        },
        {
          term: 'Aucune ressource tierce',
          detail:
            'Polices, scripts et styles sont servis depuis le même domaine. Rien n’est chargé depuis un CDN, une fonderie de polices ou un service d’images.',
        },
        {
          term: 'Politique de sécurité stricte',
          detail:
            'L’en-tête Content-Security-Policy du site interdit toute requête sortante vers un autre domaine (connect-src « self »). C’est une ligne que vous pouvez lire vous-même dans les en-têtes de réponse.',
        },
      ],
    },
    {
      id: 'verifier',
      heading: 'Comment le vérifier vous-même',
      paragraphs: [
        'Ne nous croyez pas sur parole. Les trois vérifications ci-dessous ne demandent aucune compétence particulière et prennent moins de cinq minutes.',
      ],
      list: [
        'Ouvrez les outils de développement de votre navigateur, onglet « Réseau », puis filigranez un document. Aucune requête ne part pendant le traitement.',
        'Coupez votre connexion — mode avion, Wi-Fi désactivé — après le chargement de la page, puis filigranez un document. Tout fonctionne, parce que rien n’avait besoin du réseau.',
        'Lisez le code. Le dépôt est public, l’interface est sous licence AGPL-3.0-or-later et le moteur sous licence MIT ou Apache-2.0. Vous pouvez aussi recompiler le site et comparer les empreintes des fichiers servis.',
      ],
    },
    {
      id: 'limites',
      heading: 'Ce que cela ne protège pas',
      paragraphs: [
        'Le fait que le calcul soit local protège votre document pendant le filigranage. Cela ne protège pas ce qui se passe avant et après, et il serait malhonnête de laisser croire le contraire.',
      ],
      list: [
        'Cela ne protège pas contre la personne à qui vous envoyez le document : elle en détient une copie et peut en faire ce qu’elle veut, filigrane ou non.',
        'Cela ne protège pas si quelqu’un dispose déjà d’une copie sans filigrane de votre document.',
        'Cela ne sécurise pas l’envoi : une fois le PDF téléchargé, sa transmission par courriel ou par un formulaire relève de votre messagerie et de votre destinataire.',
        'Cela ne nettoie pas l’appareil : le fichier d’origine reste dans votre dossier de téléchargements, votre pellicule photo ou votre sauvegarde automatique.',
        'Cela ne protège pas un appareil déjà compromis. Si votre machine est infectée, aucun site web ne peut y remédier.',
      ],
    },
    {
      id: 'promesses',
      heading: 'Ce que nous ne promettons pas',
      paragraphs: [
        'Un filigrane visible est un moyen de dissuasion, pas une garantie cryptographique. Il augmente le coût d’une réutilisation frauduleuse et rend l’usage détourné attribuable lorsque la mention nomme le destinataire. Il ne rend pas le document infalsifiable.',
        'Un outil qui s’exécute entièrement sur votre appareil ne peut pas détenir de clé privée : une clé livrée au navigateur est une clé publique. Nous ne pouvons donc pas produire de signature attestant que « ce filigrane a été apposé par un tiers de confiance à telle date ». Il n’existe pas d’astuce pour contourner cela et nous ne prétendrons pas le contraire.',
        'Nous n’ajoutons pas non plus de filigrane invisible ou stéganographique. Dans un outil qui tourne sur votre machine, vous pourriez comparer la sortie avec et sans marque et le localiser : ce serait du théâtre de sécurité, et une promesse que nous ne pourrions pas tenir.',
      ],
      list: [
        'Un attaquant déterminé peut retirer une marque : en 2026, les outils de retouche générative effacent correctement un filigrane sur une photographie.',
        'La marque posée en diagonale sur le corps du document résiste mieux que la même marque posée dans un coin, parce qu’il faut alors reconstituer le texte masqué. C’est un argument de coût, pas une impossibilité.',
        'Le floutage léger appliqué à la marque améliore son rendu. Nous ne l’avons mesuré contre aucun outil de suppression et nous ne le présentons donc pas comme une défense.',
      ],
    },
    {
      id: 'document',
      heading: 'Ce que le PDF produit contient',
      paragraphs: [
        'Le document exporté ne contient ni date de création, ni nom de logiciel, ni métadonnées XMP, ni les données EXIF de votre photo d’origine — y compris la position GPS que les téléphones y inscrivent souvent. Le filigrane est fusionné avec les pixels : il ne s’agit pas d’un calque que l’on retire en deux clics dans un lecteur de PDF.',
      ],
    },
  ] satisfies PrivacySection[],
} as const;
