# Coloration syntaxique

Textchum colore le code avec
[tree-sitter](https://tree-sitter.github.io) : chaque document dont le
langage est reconnu conserve un véritable arbre d'analyse, mis à jour de
façon incrémentale à chaque édition, et la coloration se calcule à partir
de cet arbre — pas avec des expressions régulières.

## Langages

La détection se fait par extension de fichier, à l'ouverture et au premier
enregistrement d'un document sans titre. Sont actuellement reconnus :
Rust, Python, Go, C, JavaScript, JSON, Bash, HTML, CSS, TOML, YAML, Swift,
Zig et Markdown. Le sous-titre de la fenêtre affiche le langage actif ;
les fichiers non reconnus restent simplement en texte brut.

Les grammaires sont compilées dans l'application : la coloration
fonctionne hors ligne et à l'identique partout.

## Injections

Les documents qui incorporent d'autres langages colorent le contenu
incorporé avec la grammaire du langage incorporé :

- Les blocs de code clôturés de Markdown sont colorés selon le langage
  nommé sur la clôture (` ```rust ` et consorts), et l'emphase, les liens
  et le code en ligne de Markdown viennent d'une grammaire dédiée aux
  éléments en ligne.
- Les éléments `<script>` et `<style>` de HTML se colorent comme du
  JavaScript et du CSS.

## Comment ça marche

Le partage des rôles suit la règle architecturale du projet :

- Le **noyau** possède l'analyse. Chaque édition transmet à l'arbre une
  description exacte du changement, et tree-sitter réanalyse de façon
  incrémentale — un travail à l'échelle de la frappe, quelle que soit la
  taille du fichier. À la demande, il exécute la requête de coloration du
  langage sur une plage et répond par des *segments stylés* : des plages
  plus des indices dans une table de styles.
- La **coque** possède les pixels. Les segments sont peints comme
  attributs de rendu TextKit — une surcouche couleur seule qui ne peut pas
  invalider la mise en page du texte, si bien que colorer ne concurrence
  jamais la frappe.

La table de styles porte une couleur par apparence du système : passer du
mode clair au mode sombre recolore instantanément, avec des palettes
réglées pour chacun.

Les très grands documents (au-delà de quelques mégaoctets) sautent
délibérément la coloration ; l'éditeur lui-même reste rapide à toute
taille.

La palette elle-même est un thème — sept sont fournis d'origine, et les
thèmes de l'utilisateur sont des fichiers JSON ; voir
[les thèmes dans la configuration](configuration.fr.md).

Si un artefact de coloration survivait à une édition, **View → Redraw**
(⌥⌘L, réassignable comme `redraw`) reconstruit chaque couche visuelle
depuis zéro : attributs de base, couleurs de syntaxe, marques de
diagnostic et la marge.

## Pas encore là

- Les nuances gras/italique — la surcouche est couleur seule pour le
  moment.
- Le choix manuel du langage depuis l'interface pour les fichiers aux
  extensions inhabituelles.
- Les requêtes limitées à la zone visible pour les documents de plusieurs
  centaines de kilooctets (pour l'instant ils sont colorés en entier ou,
  au-delà d'un plafond, pas du tout).
