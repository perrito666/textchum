# Le navigateur

Chaque fenêtre d'édition porte un tiroir de navigation à sa gauche
(bascule avec **⌘0**, ou View → Toggle Navigator). Il comporte deux
volets empilés.

## Tampons ouverts, groupés par projet

Le volet supérieur liste tous les documents ouverts dans l'application,
groupés par le **projet** auquel ils appartiennent. Le projet d'un fichier
est le répertoire ancêtre le plus proche qui ressemble à une racine de
projet : un répertoire de contrôle de versions (`.git`, `.hg`, `.svn`) ou
un fichier de build/manifeste (`Cargo.toml`, `go.mod`, `package.json`,
`pyproject.toml`, `Package.swift`, `build.zig`, `Makefile`, …). Le plus
proche gagne : dans un monorepo, un fichier au sein d'un *crate* doté de
son propre `Cargo.toml` appartient à ce *crate*, pas à la racine du
dépôt. Les fichiers hors de tout projet se rassemblent sous **Other**.

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

Les fichiers cachés ne sont pas listés.

## Pas encore là

- Les actions renommer / afficher dans le Finder sur les entrées de
  l'arborescence.
- Le respect de `.gitignore` dans l'arborescence.
- Une affectation manuelle « ce fichier appartient à ce projet » pour les
  cas où l'heuristique des marqueurs se trompe.
