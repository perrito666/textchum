# Recherche

Deux façons de trouver au-delà du fichier courant, avec une règle
commune : **la portée est un chemin visible et éditable.** Les deux
panneaux montrent exactement où ils regardent — le projet du document
courant par défaut — et élargir la recherche, c'est littéralement éditer
ce chemin (jusqu'à `~` ou `/` si l'on veut). La recherche ne regarde
jamais en silence là où on ne l'attend pas.

Étant un chemin, la portée se complète comme tel sur macOS : en tapant
vers l'avant, les dossiers qui terminent le composant sous le curseur
sont proposés, `~` compris. Une portée qui n'existe pas le dit, et nomme
le dossier le plus proche qui existe — d'ordinaire à une faute de frappe
de celui qu'on voulait. Les blancs autour d'un chemin collé sont
ignorés.

Les deux parcours respectent `.gitignore`, sautent les fichiers cachés et
plafonnent la taille des fichiers, grâce au moteur de ripgrep lui-même
embarqué dans le noyau — pas un sous-processus.

## Chercher partout (⇧⌘A)

Une boîte pour les fichiers, les symboles, les commandes et les
réglages (Ctrl+Maj+A sous Linux). Ce qui est tapé est comparé aux
quatre à la fois, et chaque sorte de réponse vient dans un groupe à
elle, le groupe à la meilleure correspondance d'abord : taper `save`
place les commandes Save au-dessus d'un `save.md`, taper `main` place
`main.rs` au-dessus d'une commande qui le mentionne. Un groupe montre
quelques lignes, assez pour choisir avec le groupe suivant encore en
vue. Les fichiers sont ceux du projet, cherchés d'abord par leur nom ;
les symboles sont ceux du serveur de langage, et arrivent quand il
répond ; les commandes sont celles des menus ; les réglages sont les
interrupteurs de l'éditeur, chacun avec son état. Sans rien de tapé,
elle liste les fichiers ouverts et les commandes lancées récemment. Le
fichier ou le symbole sélectionné montre son code à côté de la liste.
⏎ ouvre un fichier ou un symbole, lance une commande, et bascule un
réglage — enregistré, et en vigueur aussitôt, comme s'il était changé
dans les Réglages.


Tapez des fragments du nom de fichier — `editwc` trouve
`EditorWindowController.swift` — avec correspondance floue et classement. ↑/↓ déplacent la sélection, ⏎ ouvre (en ramenant la fenêtre au
premier plan si le fichier est déjà ouvert), ⎋ ferme. Une requête vide
liste la portée par ordre alphabétique. Les mêmes filtres empilés que
Chercher dans le projet s'appliquent ici — chaque type de filtre
affine les chemins trouvés — et la ligne d'état dit combien de
correspondances les filtres ont élaguées.

**⏎ ouvre la ligne sélectionnée dès que les lignes répondent à ce qui
est tapé** ; tant qu'une recherche reste due — la requête vient de
changer — il cherche à la place, pour qu'affiner une requête n'ouvre
jamais un fichier par accident. ⌘⏎ (Ctrl+⏎ sous Linux) ouvre dans tous
les cas. ↑/↓ déplacent la sélection, ⎋ ferme, et le double-clic ouvre
aussi. La ligne d'état le rappelle. Le panneau peut être élargi et garde
sa taille ; dans une ligne, le répertoire cède en premier, perdant son
début, de sorte que le nom du fichier reste toujours lisible.

Sous la liste, l'endroit de la ligne sélectionnée est affiché : les
premières lignes d'un fichier dans Ouvrir rapidement, la ligne d'une
correspondance parmi celles qui l'entourent dans Chercher dans le
projet, la déclaration d'un symbole dans Aller au symbole — lues dans
le document ouvert quand le fichier l'est, qui peut être en avance sur
le disque. Le séparateur entre les deux se déplace.

La portée est parcourue une fois à l'ouverture du panneau, puis
filtrée en mémoire — la frappe reste instantanée sur un vrai dépôt, et
le nombre de fichiers en ligne d'état dit ce qui est cherché.

## Chercher dans le projet (⇧⌘F)

La requête est le texte à chercher, tel qu'il est tapé — ce qu'on colle
dans une recherche est du code, et le code est plein de `(`, `.` et
`[`. La bascule **`.*`** à côté de la requête la lit comme une
expression régulière, et reste comme on l'a laissée jusqu'à ce que
l'éditeur quitte. Les résultats arrivent en `chemin:ligne: texte`. ⏎ saute directement à la ligne correspondante.
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

## Le mot sélectionné

Sélectionner un mot entier marque les autres endroits où il apparaît
dans le texte visible — une teinte grise, distincte de la sélection et
des correspondances de la barre de recherche, pour que les trois se
distinguent. Sélectionner une partie d'un mot, ou une étendue qui en
couvre plusieurs, ne marque rien : ces sélections ont été faites pour
autre chose.

⎋ range les marques et laisse la sélection, le mot reste donc là pour
agir dessus. Modifier ou sélectionner ailleurs repose la question. La
recherche couvre ce qui est à l'écran, là où une marque se voit, et un
fichier long coûte donc ce que coûte un court.

Deux réglages dans Réglages ▸ Éditeur décident de ce qui compte : si
`Item` vaut `item`, et si `item` dans `items` en est un. Les deux sont
désactivés par défaut — un nom qui diffère par la casse est un autre
nom, et le mot a été sélectionné en tant que mot.

## La pile de sauts

Chaque saut — un résultat de recherche, Aller à la Définition, une
référence, une entrée du plan, un `chum`, Aller à la ligne, la
découverte suivante — retient d'où il est parti. Find Next et Aller au
début/à la fin du bloc aussi, quand leur destination est hors de vue ;
un pas vers une correspondance déjà à l'écran n'est pas un saut. **Go
Back** (⌃⌘←) remonte ces origines ; **Go Forward** (⌃⌘→) les
redescend. Chacun remet ce qu'il y avait quand l'endroit a été quitté :
le curseur, ce qui était sélectionné, et le défilement de la vue. L'histoire se réécrit depuis le point courant : sauter vers
un nouvel endroit jette la piste avant. Les positions survivent aux
éditions en ligne/colonne : revenir après un changement atterrit près,
pas perdu.

**Aller à la dernière modification** (⇧⌘⌫, Ctrl+Maj+Retour arrière
sous Linux) va là où quelque chose a été changé en dernier, dans le
fichier ouvert où c'était ; appuyé à nouveau avec le curseur encore là,
il va au changement d'avant, et ainsi de suite. Une rafale de frappe
sur quelques lignes est un seul endroit, charger ou recharger un
fichier n'en est aucun, et les endroits suivent le texte quand on
l'édite autour d'eux. Chacun est un saut, donc Go Back revient. La
piste dure la session : un fichier fermé emporte ses endroits.

**Fichiers récents** (⌘E, Ctrl+E sous Linux) liste les fichiers ouverts
récemment dans le projet au premier plan, le plus récent d'abord, sans
celui qui est devant : ⌘E puis Entrée revient au fichier d'avant.
**Endroits récents** (⌥⌘E, Ctrl+Alt+Maj+E sous Linux) liste où le
curseur est passé — les endroits que les sauts ont quittés et les
endroits modifiés, dans n'importe quel fichier — le plus récent
d'abord, une entrée par endroit, chacune avec le code autour à côté de
la liste. Les deux filtrent pendant la frappe. Les endroits, comme la
piste des modifications, durent la session.

Un saut vers une ligne déjà à l'écran laisse la vue où elle est. Un
saut hors de vue la déplace, en une brève animation, pour que la ligne
se trouve au tiers de la hauteur, avec ce qui la précède en vue ; un
saut lointain commence le mouvement un écran avant la cible, pour
montrer de quel côté il est allé sans balayer tout le fichier. Quand
le mouvement réduit est demandé dans les réglages du système, la vue se
déplace d'un coup.

## Pas encore là

- Chercher partout ne liste que les réglages qui s'activent ou se
  désactivent ; une largeur de tabulation ou un thème se choisissent
  dans les Réglages. Deux appuis sur Maj ne l'ouvrent pas.
- Le remplacement entre fichiers.
- Les bascules casse/mot entier dans le panneau (le motif lui-même peut
  exprimer les deux).
- L'historique des recherches persistant.
- L'édition de la portée dans l'interface GTK, dont les panneaux ne
  cherchent que dans le projet courant ; la complétion des chemins
  viendra avec.
