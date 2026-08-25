# Configuration

Les réglages de Textchum suivent un principe : **l'interface graphique est
le moyen confortable de les changer, et un simple fichier JSON est la
sortie de secours toujours disponible.** Il y a exactement un seul dépôt —
le fichier — et la fenêtre Réglages le lit et l'écrit ; rien ne vit
uniquement à l'intérieur de l'application.

## La fenêtre Réglages

**Textchum → Settings…** (⌘,) édite les réglages reconnus :

- **Apparence** — suivre le système (en changeant en direct quand macOS le
  fait) ou forcer le clair ou le sombre.
- **Thème** — la palette de syntaxe ; voir [Thèmes](#thèmes) plus bas.
- **Ouvrir les fichiers dans** — des onglets de la fenêtre courante (le
  défaut) ou des fenêtres séparées. Avec des fenêtres séparées, le
  navigateur de chaque fenêtre ne liste que les documents de son propre
  groupe d'onglets.
- **Police** — toute famille à chasse fixe installée sur le système, ou la
  police à chasse fixe de la plateforme.
- **Taille de police** — de 6 à 72 points.
- **Largeur de tabulation** — de 1 à 16 colonnes.
- **Afficher les numéros de ligne** — la marge, aussi basculables par
  session avec View → Toggle Line Numbers (⇧⌘L).

Chaque changement s'applique immédiatement aux fenêtres d'édition ouvertes
et s'écrit sur disque au même instant. Il n'y a aucun bouton Appliquer ou
Enregistrer à oublier.

## Le fichier

Les réglages vivent dans :

```
~/Library/Application Support/Textchum/config.json
```

Un fichier édité à la main pourrait ressembler à :

```json
{
  "appearance": "dark",
  "editor": {
    "font_family": "JetBrains Mono",
    "font_size": 13,
    "tab_width": 4,
    "hover": false
  }
}
```

`appearance` accepte `"system"`, `"light"` ou `"dark"` ; en son absence
(le défaut), le système est suivi. `editor.hover` désactive la bulle de
documentation au survol (`true`, le défaut, la laisse active).

Tout est optionnel — fichier, section ou clé manquants signifient
simplement la valeur par défaut. Les écritures sont atomiques (fichier
temporaire puis renommage), comme toute écriture de Textchum.

Deux garanties rendent l'édition à la main sûre :

- **Les clés inconnues survivent.** La fenêtre de réglages ne réécrit que
  les clés qui lui appartiennent. Tout le reste du fichier — vos
  annotations, des clés d'une version plus récente — est préservé tel quel
  à chaque enregistrement.
- **Les fichiers cassés ne sont jamais écrasés.** Si le fichier ne peut pas
  être analysé, Textchum démarre avec les réglages par défaut, le signale
  une fois au lancement et laisse le fichier exactement tel que vous
  l'aviez écrit, pour le réparer dans n'importe quel éditeur — y compris
  Textchum lui-même. Si vous changez un réglage depuis l'interface pendant
  que le fichier est cassé, l'original inanalysable est d'abord copié vers
  `config.json.bak`, puis remplacé.

Les valeurs hors limites ou mal typées ne comptent pas comme une casse : un
`font_size` de `4000` est ramené dans la plage valide, un `font_family` de
`42` est ignoré, et le reste du fichier fonctionne normalement.

## Thèmes

Le sélecteur **Theme** de l'onglet General choisit la palette de
syntaxe. Sept sont fournis d'origine : **Textchum** (par défaut),
**Textchum High Contrast**, **Graphite** (un thème feutré presque
monochrome) et les classiques — **Molokai**, **Solarized**, **Dracula**
et **Gruvbox**. Chaque thème porte une palette claire et une palette
sombre dans un même fichier, si bien qu'un thème sert aux deux modes
d'apparence (les classiques nés sombres associent leur palette
canonique à une claire au contraste ajusté ; Solarized et Gruvbox
utilisent leurs vraies palettes claires).

Les thèmes personnels sont des fichiers JSON dans :

```
~/Library/Application Support/Textchum/themes/
```

sélectionnés par nom de fichier (sans `.json`) ; un fichier portant le
nom d'un thème intégré le remplace. **Textchum → Open Themes Folder**
ouvre (et crée) ce dossier. Le plus rapide pour en commencer un
est de générer un point de départ complet — chaque nom de capture
stylée, rempli avec la palette par défaut — et de ne changer que les
couleurs :

```bash
Textchum --emit-theme ~/Library/Application\ Support/Textchum/themes/Mien.json
```

Les entrées associent des noms de capture tree-sitter à des styles :

```json
{
  "name": "Mien",
  "styles": {
    "keyword": {"light": "#AD3DA4", "dark": "#FC5FA3", "bold": true},
    "comment": {"light": "#707F8C", "dark": "#7F8C98", "italic": true}
  }
}
```

Les couleurs s'écrivent `#RRGGBB` ou `#RRGGBBAA`. Tout ce qui est omis
— une couleur, un drapeau, une capture entière — garde la valeur de la
palette par défaut : un thème n'a besoin de dire que ce qu'il change.
Les règles de secours sont celles de la configuration : un thème qui ne
se laisse pas analyser retombe sur le thème par défaut avec un seul
avertissement et n'est jamais écrasé, et les clés inconnues survivent.
Les fichiers de thème sont lus au lancement et au changement de
sélection.

## Projets

L'onglet Projects décide où un projet commence et finit — la frontière
selon laquelle le navigateur regroupe et sur laquelle le pool de
serveurs de langage indexe ses instances. Chaque interrupteur existe en
deux exemplaires : comme valeur par défaut pour tous les projets, et par
racine de projet. Une ligne ajoutée avec le champ de chemin (qui
complète les noms de répertoire pendant la frappe et porte un bouton
Browse…) remplace les valeurs par défaut pour cette racine seulement.

- **Manifest projects** — normalement le dépôt le plus externe
  l'emporte : ouvrir un fichier n'importe où dans un dépôt fait du dépôt
  le projet, quel que soit le nombre de `Cargo.toml` ou `pyproject.toml`
  entre les deux. L'activer redécoupe une racine aux manifestes de
  langage, si bien que les modules imbriqués redeviennent des projets à
  part entière.
- **Recursive config** — fait que les réglages par projet d'une racine
  (ses commandes de serveur de langage et ces interrupteurs eux-mêmes)
  s'appliquent aux projets imbriqués qu'elle contient, l'ancêtre le plus
  proche d'abord. Utile pour les monorepos : une configuration en haut,
  beaucoup de projets en dessous.
- **Ctags fallback** — répond à Aller à la Définition depuis un index
  Universal Ctags quand aucun serveur de langage n'est disponible ; voir
  [serveurs de langage](language-servers.fr.md).

Dans le fichier, tout cela vit dans une section `workspace` :

```json
{
  "workspace": {
    "manifest_projects": false,
    "recursive_config": false,
    "ctags_fallback": false,
    "projects": {
      "/Users/you/code/monorepo": {
        "manifest_projects": true,
        "recursive_config": true
      }
    }
  }
}
```

## Préprocesseurs de sauvegarde

Les formateurs et correcteurs peuvent s'exécuter automatiquement avant
chaque sauvegarde, par langage — pour tous les projets ou pour une
racine précise, exactement comme les serveurs de langage. Chaque entrée
est une chaîne : une commande par ligne, dans l'ordre, où chaque
commande lit le document sur l'entrée standard et réécrit le document
entier sur la sortie standard (la convention `-` que suivent presque
tous les formateurs). Si un maillon échoue — code de sortie non nul,
sortie vide, ou plus de dix secondes sans répondre — rien n'est
appliqué, l'erreur (avec le stderr de l'outil) s'affiche, et la
sauvegarde demande s'il faut continuer sans traitement.

```json
{
  "preprocessors": {
    "defaults": {
      "python": ["ruff check --fix -", "black -"],
      "go": ["gofmt"]
    },
    "projects": {
      "/work/site": { "javascript": ["prettier --stdin-filepath x.js"] }
    }
  }
}
```

Une entrée de projet remplace la chaîne par défaut pour ce langage,
elle ne s'y ajoute jamais. La fenêtre Réglages édite cette même section
sous Serveurs de langage, et **Édition ▸ Lancer les préprocesseurs**
(⌃⌥⌘F, nom d'action `runPreprocessors`) lance la chaîne à la demande
sans sauvegarder — formater avec vos outils plutôt qu'avec le
formateur du serveur. Le résultat arrive en une seule édition, donc ⌘Z
l'annule.

## Correction orthographique

La prose passe par le correcteur du système — les dictionnaires que
partagent toutes les apps du Mac — restreint à là où la prose vit
vraiment : les commentaires dans le code, et le document entier en
Markdown, dans les messages de commit git et le texte brut. Les
identifiants et les littéraux de chaîne ne sont jamais signalés. Les
fautes portent une teinte violette, distincte du rouge/orange/bleu des
diagnostics.

Choisissez la langue dans Réglages ▸ Général ▸ « Spell check prose » —
Désactivé (le défaut), automatique selon le contenu, ou un dictionnaire
précis — ou réglez `editor.spell` à la main : `"auto"` ou un
identifiant comme `"fr"` ou `"en_US"`. Les dictionnaires disponibles
sont ceux activés dans Réglages Système ▸ Clavier ▸ Saisie de texte.

```json
{ "editor": { "spell": "auto" } }
```

## Raccourcis clavier

Les raccourcis des menus se réassignent via une section `keys` éditée à
la main (pas d'interface pour l'instant) : un objet de noms d'action
vers des spécifications `modificateurs+touche`, appliqué au lancement.

```json
{
  "keys": {
    "openQuickly": "cmd+p",
    "goToBlockEnd": "ctrl+alt+down",
    "findInProject": "cmd+shift+g"
  }
}
```

Modificateurs : `cmd`, `shift`, `alt`, `ctrl`. Touches : un caractère,
ou `up`/`down`/`left`/`right`/`return`/`escape`/`space`/`tab`/`delete`.
Parmi les actions : `new`, `open`, `openQuickly`, `save`, `saveAs`,
`close`, `undo`, `redo`, `find`, `findAndReplace`, `findNext`,
`findPrevious`, `useSelectionForFind`, `findInProject`,
`jumpToDefinition`, `findReferences`, `renameSymbol`, `formatDocument`,
`runPreprocessors`,
`documentOutline`, `goBack`, `goForward`,
`goToBlockStart`, `goToBlockEnd`,
`toggleNavigator`, `togglePreview`, `toggleLineNumbers`,
`toggleHover`, `showHover`,
`togglePathDisplay`, `redraw`, `commandPalette`, `settings` —
un nom inconnu est journalisé avec la liste complète. Aller au début/à
la fin du bloc (⌃⌥↑/⌃⌥↓ par défaut) saute par-dessus le bloc syntaxique
multiligne le plus interne autour du curseur, grâce à l'arbre qui
alimente déjà la coloration. Et quand un raccourci échappe tout à fait
à la mémoire, la **palette de commandes** (⇧⌘P) cherche floue n'importe
quelle action de menu par son nom et exécute la sélection.

## Pas encore là

- Textchum ne surveille pas encore le fichier en cours d'exécution ; les
  changements faits dans un autre éditeur s'appliquent au prochain
  lancement.
- Les réglages d'éditeur par projet (police, largeur de tabulation) —
  les projets portent déjà leurs propres réglages de détection et de
  serveurs de langage, mais pas ceux-ci.
