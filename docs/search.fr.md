# Recherche

Deux façons de trouver au-delà du fichier courant, avec une règle
commune : **la portée est un chemin visible et éditable.** Les deux
panneaux montrent exactement où ils regardent — le projet du document
courant par défaut — et élargir la recherche, c'est littéralement éditer
ce chemin (jusqu'à `~` ou `/` si l'on veut). La recherche ne regarde
jamais en silence là où on ne l'attend pas.

Les deux parcours respectent `.gitignore`, sautent les fichiers cachés et
plafonnent la taille des fichiers, grâce au moteur de ripgrep lui-même
embarqué dans le noyau — pas un sous-processus.

## Ouvrir rapidement (⌘T)

Tapez des fragments du nom de fichier — `editwc` trouve
`EditorWindowController.swift` — avec correspondance floue et classement
à la fzf. ↑/↓ déplacent la sélection, ⏎ ouvre (en ramenant la fenêtre au
premier plan si le fichier est déjà ouvert), ⎋ ferme. Une requête vide
liste la portée par ordre alphabétique. Les mêmes filtres empilés que
Chercher dans le projet s'appliquent ici — chaque type de filtre
affine les chemins trouvés — et la ligne d'état dit combien de
correspondances les filtres ont élaguées.

**⏎ cherche, ⌘⏎ ouvre** (Ctrl+⏎ sous Linux) : affiner une requête ne
devrait jamais ouvrir un fichier par accident. ↑/↓ déplacent la
sélection, ⎋ ferme, et le double-clic ouvre aussi. La ligne d'état le
rappelle.

La portée est parcourue une fois à l'ouverture du panneau, puis
filtrée en mémoire — la frappe reste instantanée sur un vrai dépôt, et
le nombre de fichiers en ligne d'état dit ce qui est cherché.

## Chercher dans le projet (⇧⌘F)

La requête est une expression régulière ; les résultats arrivent en
`chemin:ligne: texte`. ⏎ saute directement à la ligne correspondante.
Les résultats sont plafonnés (200) pour rester instantanés ; affinez le
motif plutôt que de faire défiler.

La casse suit la règle **smart case** popularisée par ripgrep : une
requête tout en minuscules correspond à n'importe quelle casse, tandis
qu'une requête contenant une majuscule est cherchée telle quelle. Ainsi
`todo` trouve `TODO`, et `TODO` ne trouve que `TODO`.

Une ligne sous les résultats dit ce qu'a fait la recherche — « 18
matches in 4 files · 812 searched », « No matches in 812 files
searched », ou la raison pour laquelle rien n'a pu être cherché (une
portée inexistante, une où tout est ignoré, ou un motif invalide, cité).
Un résultat vide n'est jamais muet : un motif mal tapé ou une portée
erronée s'annoncent au lieu de ressembler à une absence de
correspondances.

## Filtres empilés

Sous la requête de Chercher dans le projet, **＋ Add Filter** empile des
raffinements :

- **line contains / line excludes** — le texte de la ligne trouvée ;
- **file contains / file excludes** — le chemin du fichier du résultat.

Les filtres sont des sous-chaînes insensibles à la casse et se combinent
par *et* : les lignes avec `foo` où `bar` apparaît aussi, mais pas dans
les fichiers avec `test` dans le nom, c'est la requête `foo` plus
`line contains bar` plus `file excludes test`. Les exclusions de fichier
élaguent des fichiers entiers avant même de les ouvrir ; les recherches
filtrées restent donc aussi rapides que les simples.

## La pile de sauts

Chaque saut — un résultat de recherche, Aller à la Définition, une
référence, une entrée du plan, un `chum` — retient d'où il est parti.
**Go Back** (⌃⌘←) remonte ces origines ; **Go Forward** (⌃⌘→) les
redescend. L'histoire se réécrit depuis le point courant : sauter vers
un nouvel endroit jette la piste avant, exactement comme la jumplist de
vim. Les positions survivent aux éditions en ligne/colonne : revenir
après un changement atterrit près, pas perdu.

## Pas encore là

- Le remplacement entre fichiers.
- Les bascules casse/mot entier dans le panneau (le motif lui-même peut
  exprimer les deux).
- L'historique des recherches persistant.
