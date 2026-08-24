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
liste la portée par ordre alphabétique.

## Chercher dans le projet (⇧⌘F)

La requête est une expression régulière ; les résultats arrivent en
`chemin:ligne: texte`. ⏎ saute directement à la ligne correspondante.
Les résultats sont plafonnés (200) pour rester instantanés ; affinez le
motif plutôt que de faire défiler.

## Pas encore là

- Le remplacement entre fichiers.
- Les bascules casse/mot entier dans le panneau (le motif lui-même peut
  exprimer les deux).
- L'historique des recherches persistant.
