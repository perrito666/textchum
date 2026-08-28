# Coloration syntaxique

Textchum colore le code avec
[tree-sitter](https://tree-sitter.github.io) : chaque document dont le
langage est reconnu conserve un véritable arbre d'analyse, mis à jour de
façon incrémentale à chaque édition, et la coloration se calcule à partir
de cet arbre — pas avec des expressions régulières.

## Langages

La détection se fait par extension de fichier — ou par nom exact, pour
les fichiers dont l'identité *est* le nom : `Makefile` (et `*.mk`) et
les messages de git (`COMMIT_EDITMSG`, `MERGE_MSG`, `TAG_EDITMSG`), si
bien que les messages de commit écrits via `chum --wait` arrivent
colorés. Elle s'exécute à l'ouverture et au premier enregistrement d'un
document sans titre. Sont actuellement reconnus : Rust, Python, Go, C,
JavaScript, JSON, Bash, Make, messages de commit git, HTML, CSS, TOML,
YAML, Swift, Zig et Markdown. Le sous-titre de la fenêtre affiche le
langage actif ; les fichiers non reconnus restent simplement en texte
brut. Les lignes du navigateur portent l'icône Finder propre au type quand
macOS le distingue vraiment — et un petit insigne à la couleur
conventionnelle du langage sinon. La distinction compte : une
application par défaut (un IDE, disons) estampille son *propre* icône
de document sur tous les types qu'elle revendique, identique partout ;
une icône partagée entre types compte donc comme générique et l'insigne
l'emporte.

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

La coloration suit la zone visible : c'est elle, plus une marge
généreuse, qui est interrogée et peinte, puis repeinte au défilement.
Un fichier d'un mégaoctet coûte autant qu'un petit, et il n'existe plus
de taille au-delà de laquelle la couleur s'arrête en silence — sauf le
plafond d'analyse du cœur, au-delà duquel un document est du texte brut
par choix.

Le **gras et l'italique** d'un thème sont honorés aussi. La couleur
passe par les attributs de rendu de TextKit, qui ne touchent pas à la
mise en page ; les traits typographiques sont appliqués comme polices,
d'où leur peinture sur la portion visible plutôt que sur tout le
document. Les fontes à chasse fixe gardent leur largeur d'une graisse à
l'autre, donc rien ne se replace.

## Pas encore là

- Les requêtes limitées à la portion visible pour les documents de
  plusieurs centaines de kilooctets : aujourd'hui ils sont colorés en
  entier ou, au-delà d'un plafond, pas du tout.
