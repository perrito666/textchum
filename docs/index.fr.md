# Textchum

Textchum est un éditeur de texte pour macOS dans l'esprit de TextMate :
natif, rapide et concentré sur une seule mission — **éditer et valider une
grande variété de types de fichiers** — plutôt que d'être un IDE. Pas de
bouton d'exécution, pas de débogueur, pas de place de marché d'extensions à
l'horizon ; ce qu'il y a (ou aura), c'est la coloration syntaxique pour de
nombreux langages et une validation appuyée sur des serveurs de langage qui
respecte les frontières de chaque projet.

## Comment il est construit

Textchum est scindé en deux moitiés :

- **Le noyau** (`libtextchum`), écrit en Rust, possède tout ce qui touche au
  texte : tampons, éditions et le flux d'événements qui tient l'interface
  informée. Il se compile en bibliothèque statique avec une interface C
  simple et ignore tout de macOS.
- **La coque** (*shell*), écrite en Swift avec AppKit, possède tout ce qui
  touche à la plateforme : fenêtres, rendu, saisie et menus. Elle ne détient
  jamais d'état du document en propre — chaque édition transite par le noyau.

Cette séparation garde la logique intéressante portable et testable sans
interface graphique, tandis que la couche visible reste entièrement native.
La [page d'architecture](architecture.md) explique le raisonnement et les
règles de la frontière.

## État actuel

Textchum est jeune. Ce qui existe et fonctionne aujourd'hui :

- Un noyau Rust exposant des tampons de texte fondés sur des *ropes* à
  travers une ABI C, avec édition par décalages d'octets et par unités
  UTF-16 (cette dernière correspondant à la façon dont AppKit et le Language
  Server Protocol adressent le texte).
- Des documents au-dessus des tampons : ouverture et enregistrement avec
  détection d'encodage et écritures atomiques, annuler/rétablir avec fusion
  des frappes, et suivi des modifications ancré au dernier enregistrement —
  voir [Documents](documents.md).
- Un canal d'événements asynchrone des fils d'exécution du noyau vers
  l'interface, avec un contrat strict de livraison sur un seul fil.
- Un éditeur macOS à fenêtres multiples (avec onglets natifs), panneaux
  d'ouverture/enregistrement et invites d'enregistrement à la fermeture ;
  chaque vue de texte est maintenue au pas avec son document du noyau par un
  protocole de synchronisation qui interdit toute divergence.
- Un test de fumée sans interface qui exerce l'aller-retour complet
  Swift ↔ noyau — édition, annulation, enregistrement, réouverture,
  événements —, utilisé par l'intégration continue comme par les humains
  pressés.

La suite, dans l'ordre approximatif : coloration syntaxique, serveurs de
langage par projet et aperçu Markdown.

## Pour aller plus loin

- [Premiers pas](getting-started.md) — compiler et lancer Textchum depuis
  les sources.
- [Architecture](architecture.md) — la séparation noyau/coque et ses règles.
- [Documents](documents.md) — annulation, modifications, encodages,
  enregistrements atomiques.
- [La frontière C](ffi.md) — conventions de l'interface entre les deux.
