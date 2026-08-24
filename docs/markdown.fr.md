# Markdown

Markdown est un citoyen de première classe : le même fichier reçoit la
[coloration](highlighting.md) tree-sitter dans l'éditeur — y compris les
blocs de code clôturés colorés dans leur propre langage — et un **aperçu
en direct** à côté.

## L'aperçu

Ouvrir un document Markdown ouvre automatiquement le volet d'aperçu, à
droite de la source. **View → Toggle Markdown Preview** (⌥⌘P) le masque
et l'affiche.

- L'aperçu se met à jour pendant la frappe, rapiécé sur place — pas de
  rechargement, pas de scintillement, pas de position de défilement
  perdue.
- Faire défiler l'un des volets entraîne l'autre.
- Le rendu prend en charge CommonMark plus les tableaux, le barré, les
  listes de tâches et les notes de bas de page, et ses styles suivent
  l'apparence du système (ou celle configurée).

Le rendu se fait dans le noyau (la coque ne possède que le volet) ; le
même HTML alimentera plus tard d'autres sorties.

## Pas encore là

- La synchronisation de défilement précise par ancres de source (celle
  d'aujourd'hui est proportionnelle et dérive sur les documents aux blocs
  très inégaux).
- Les couleurs de syntaxe dans les blocs de code de l'aperçu (l'éditeur
  les colore ; l'aperçu les montre bruts).
- Le mode d'édition hybride/WYSIWYG — le volet d'aperçu est délibérément
  le premier des trois niveaux Markdown.
