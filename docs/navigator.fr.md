# Le navigateur

Chaque fenêtre d'édition porte un tiroir de navigation à sa gauche
(bascule avec **⌘0**, ou View → Toggle Navigator). Il comporte deux
volets empilés.

## Tampons ouverts, groupés par projet

Le volet supérieur liste les documents ouverts du **groupe d'onglets de
cette fenêtre** — les fichiers ouverts en onglets partagent une même
liste, tandis que les fenêtres séparées gardent des mondes séparés (que
les fichiers s'ouvrent en onglets ou en fenêtres est un
[réglage](configuration.md)). Les documents sont groupés par le
**projet** auquel ils appartiennent. Le projet d'un fichier
se résout dans cet ordre :

1. le `.textchum.json` le plus proche — l'affectation explicite, posée à
   la main ;
2. la **racine de contrôle de versions la plus externe** (`.git`, `.hg`,
   `.svn`) : un dépôt est un seul projet, quel que soit le nombre de
   manifestes imbriqués — un paquet Python dans un sous-dossier
   appartient au dépôt, et les dépôts imbriqués se résolvent au plus
   externe ;
3. hors contrôle de versions, le fichier de build/manifeste le plus
   proche (`Cargo.toml`, `go.mod`, `package.json`, `pyproject.toml`,
   `Package.swift`, `build.zig`, `Makefile`, …).

Les fichiers hors de tout projet se rassemblent sous **Other**. La règle
de l'étape 2 — le dépôt l'emporte — peut être assouplie par projet :
l'interrupteur **Manifest projects** de
[Réglages → Projects](configuration.fr.md) redécoupe une racine à ses
manifestes de langage, pour les dépôts qui sont en réalité plusieurs
projets déguisés en un seul.

Les lignes affichent le nom de fichier nu — jusqu'à ce que deux
fichiers ouverts en partagent un : chacun montre alors juste assez de
fin de chemin pour les distinguer (les titres d'onglets suivent). Le
bouton en haut de la liste — ou View → Toggle Path Display (⌥⌘T) —
bascule toutes les lignes vers leur chemin
depuis la racine du projet tant qu'il est actif ; volontairement non
mémorisé d'un lancement à l'autre — c'est un coup d'œil, pas un mode.

Un clic droit sur l'**en-tête d'un projet** propose l'agencement des
fenêtres du groupe entier : **Split into New Window** sort les
documents du projet dans une fenêtre à eux (comme ses onglets) et
**Gather Into** est un sous-menu de destinations — This Window, ou
toute autre fenêtre ouverte (son groupe d'onglets, en réalité) — qui y
adopte les documents du projet comme onglets. Le séparateur entre la liste des tampons et l'arbre
des dossiers est une position unique partagée — le glisser dans un
onglet le déplace dans tous, et il est retenu avec la session.

Un clic droit sur une ligne de la liste ou une entrée de l'arbre
propose l'emplacement du fichier sous toutes ses formes utiles : le nom
nu, le chemin relatif à la racine du projet, le chemin absolu et — dans
un dépôt git avec un remote — l'URL du fichier sur sa forge, parlant
nativement les formes d'URL de GitHub, GitLab et Forgejo. Les mêmes
éléments agissent sur l'onglet de devant via **File → Copy Path**.

C'est la même notion de « projet » que le reste de Textchum utilise (et
celle qui délimitera les serveurs de langage) ; le tiroir sert donc aussi
de révélateur : si un fichier est groupé à un endroit surprenant, c'est
exactement ainsi que le reste de l'application le voit aussi.

Le document de la fenêtre courante est en gras ; les documents aux
changements non enregistrés portent un point. Cliquer sur un document
amène sa fenêtre au premier plan.

## L'arborescence du projet

Le volet inférieur montre l'arborescence du projet du document courant,
depuis sa racine. Cliquer sur un fichier l'ouvre — ou ramène sa fenêtre
au premier plan s'il est déjà ouvert. Les documents sans projet (le
groupe **Other**) n'affichent pas d'arborescence.

Les dossiers dépliés sont un état partagé : dépliez un dossier dans un
onglet et il l'est dans tous (et dans toute fenêtre montrant le même
projet).

L'arbre suit le fichier : changer d'onglet déplie le chemin du
document courant et le met en évidence (désactivable dans Réglages ▸
Général ▸ « Reveal the current file in the tree »), et
**Présentation ▸ Révéler dans l'arbre** (⇧⌘J, nom d'action
`revealInTree`, aussi dans le menu contextuel des lignes de tampons)
le fait à la demande — en rouvrant le navigateur au besoin.

Ce que l'arbre cache est de la configuration : des motifs glob sur les
noms de fichiers, `.*` (les fichiers cachés) par défaut. Cliquer un
bouton **Hide** ouvre un éditeur avec un motif par ligne et un menu
**Add preset** qui ajoute un ensemble nommé. Chaque racine de projet
peut porter sa propre liste, qui remplace celle par défaut.

Les préréglages sont à vous aussi : l'onglet **Presets** les édite de
la même façon — un motif par ligne — avec ajout, suppression et
restauration des intégrés. Éditer l'un d'eux prend possession de tout
l'ensemble, donc celui que vous supprimez le reste. Ils vivent dans
`workspace.hide_presets`, et les Préférences Linux éditent la même
section. Dans le fichier :

```json
{ "workspace": { "hide": [".*", "target", "node_modules"] } }
```

## Pas encore là

- Les actions renommer / afficher dans le Finder sur les entrées de
  l'arborescence.
- Le respect de `.gitignore` dans l'arborescence.
- Une affectation manuelle « ce fichier appartient à ce projet » pour les
  cas où l'heuristique des marqueurs se trompe.
