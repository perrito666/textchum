# Documents

Un *tampon* est du texte brut ; un *document* est un tampon plus tout ce qui
en fait un fichier : un historique d'annulation, un indicateur de
modifications non enregistrées, un chemin et un encodage. Les fenêtres de
l'éditeur travaillent toujours avec des documents.

## Annuler et rétablir

L'historique d'annulation vit dans le noyau, pas dans le `NSUndoManager`
d'AppKit. Chaque édition est enregistrée comme une opération inversible ;
annuler dépile l'enregistrement le plus récent, applique son inverse et
signale le changement résultant à la fenêtre, qui le rejoue à l'écran.
Puisque l'historique se trouve derrière la même interface que tout le reste,
aucune édition ne peut lui échapper — il n'existe pas de second chemin vers
le texte.

Les enregistrements fusionnent pour qu'annuler avance à pas humains :

- **Les séquences de frappe** fusionnent : des insertions consécutives,
  chacune commençant exactement là où la précédente s'est arrêtée,
  deviennent un seul pas d'annulation.
- **Les séquences de suppression** fusionnent de la même façon, au retour
  arrière comme à la suppression avant.
- Un **saut de ligne** termine la séquence de part et d'autre ; annuler
  s'arrête donc à la granularité de la ligne.
- **Déplacer le curseur** (clic, flèches) termine la séquence en cours ; la
  frappe suivante ouvre un nouveau pas.

Les opérations composées s'enregistrent comme des groupes explicites :
Tout remplacer réécrit chaque occurrence mais s'annule en un seul pas, et
un rechargement depuis le disque (ci-dessous) est lui aussi un seul pas.

## Rechercher et remplacer

**⌘F** ouvre la barre de recherche native (**⌥⌘F** avec le champ de
remplacement, **⌘G** / **⇧⌘G** occurrence suivante et précédente, **⌘E**
recherche la sélection). Le menu d'options de la barre propose la
correspondance par sous-chaîne, mot entier et **expression régulière**.
Les remplacements sont des éditions ordinaires : ils passent par le noyau,
entrent dans l'historique d'annulation — Tout remplacer en un seul pas —
et marquent le document comme modifié, comme la frappe.

## Modifications externes

Textchum surveille le fichier de chaque document ouvert. Si un autre
programme le modifie :

- un document **propre** suit le disque en silence — la fenêtre montre
  simplement le nouveau contenu ;
- un document **modifié** pose la question : garder vos changements non
  enregistrés, ou recharger depuis le disque. Recharger abandonne le
  tampon au profit du fichier, mais le rechargement est lui-même un pas
  d'annulation : ⌘Z ramène votre version (et marque de nouveau le document
  comme modifié, puisqu'il diffère alors du disque).

Un fichier qui disparaît du disque est laissé tranquille : le tampon
reste, et enregistrer recrée le fichier.

## Modifications non enregistrées

Un document connaît le point exact de son historique où il a été enregistré
pour la dernière fois ; *modifié* signifie donc « l'état courant diffère de
l'état enregistré » — et non « une édition a eu lieu à un moment donné ».
Éditer puis annuler jusqu'au point d'enregistrement laisse un document
propre, et le bouton de fermeture de la fenêtre perd son point en
conséquence. Si de nouvelles éditions rendent l'état enregistré
inatteignable (annulé au-delà, puis autre chose tapé), le document compte
comme modifié jusqu'au prochain enregistrement, comme il se doit.

Fermer une fenêtre modifiée, ou quitter avec des fenêtres modifiées
ouvertes, pose la question habituelle : enregistrer, ne pas enregistrer ou
annuler.

## Fichiers et encodages

Textchum décode à l'ouverture et ré-encode à l'enregistrement :

- L'**UTF-8** valide se charge en UTF-8. Un BOM en tête est retiré en
  mémoire, mémorisé, et réécrit à l'enregistrement.
- Tout le reste est décodé en **ISO-8859-1** (Latin-1), qui associe un
  caractère à chaque octet et ne peut donc pas échouer. L'enregistrement
  ré-encode en Latin-1 ; si une édition a introduit des caractères que le
  Latin-1 ne peut pas contenir, l'enregistrement promeut silencieusement le
  fichier en UTF-8 — rien ne peut se perdre dans ce sens — et le sous-titre
  de la fenêtre reflète le nouvel encodage.

Les fins de ligne ne sont jamais normalisées : ce qui a été lu est ce qui
est écrit, que ce soit `\n` ou `\r\n`.

L'encodage courant est toujours visible dans le sous-titre de la fenêtre, à
côté de la taille du document.

## Les enregistrements sont atomiques

Un enregistrement écrit le document entier dans un fichier temporaire du
répertoire cible, le synchronise sur disque, puis le renomme par-dessus la
cible. Un plantage en plein enregistrement ne peut jamais laisser un fichier
tronqué, et les autres programmes qui observent le fichier voient l'ancien
contenu ou le nouveau — jamais un mélange.

## Restauration de session

Relancer Textchum rouvre les fichiers qui étaient ouverts, chacun avec sa
position de curseur et de défilement, en ramenant au premier plan celui
qui était utilisé. L'état est un simple fichier JSON (`session.json`, à
côté de la configuration), écrit en continu — pas seulement à la
fermeture — de sorte qu'un plantage perd au plus un instant de position,
jamais la liste des fichiers. Les fichiers disparus sont ignorés.

Pour démarrer sans mémoire (utile en chassant un bogue) : lancer avec
`--fresh`, maintenir ⇧ pendant le démarrage, ou supprimer
`session.json` — chacune des trois options est une remise à zéro
complète.

## Pas encore là

- Les encodages au-delà d'UTF-8 et de Latin-1.
- La recherche à l'échelle du projet, entre fichiers.
