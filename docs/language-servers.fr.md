# Serveurs de langage

Textchum valide le code via le
[Language Server Protocol](https://microsoft.github.io/language-server-protocol/),
avec un comportement fondateur : **une instance de serveur par projet**.

## Une instance par projet

Les processus serveurs sont identifiés par *(serveur, racine du projet)*,
avec la même notion de projet que [le navigateur](navigator.md) : le
répertoire ancêtre le plus proche portant un marqueur de racine. Ouvrez
des fichiers de deux projets Rust différents et deux processus
`rust-analyzer` indépendants tournent, chacun initialisé avec sa propre
racine, chacun ne voyant que les fichiers de son projet. Les fuites entre
projets — des diagnostics d'un espace de travail débordant dans un autre,
un index bâti sur tout le répertoire personnel — sont impossibles par
construction.

Les fichiers hors de tout projet reçoivent une instance par répertoire ;
les fichiers isolés ne rejoignent donc jamais l'espace de travail de
quelqu'un d'autre.

## Ce que l'on voit

- Les résultats arrivent pendant la frappe (envoyés par lots avec
  temporisation) et marquent le texte concerné : rouge pour les erreurs,
  orange pour les avertissements, bleu pour les notes.
- Le sous-titre de la fenêtre les compte (« 2 errors, 1 warning »).
- **La complétion pendant la frappe** : les suggestions apparaissent
  après les caractères d'identifiant et `.`, filtrées au fil de la
  frappe — ↑/↓ pour choisir, ⏎ ou ⇥ pour accepter, ⎋ pour fermer,
  ⌃Espace pour les demander explicitement.
- Laisser la souris sur un symbole affiche la documentation **hover** du
  serveur dans une bulle, avec le Markdown envoyé par les serveurs déjà
  rendu — blocs de code en chasse fixe, emphase et code en ligne
  stylés. Elle ne se déclenche que sur les identifiants (jamais sur les
  espaces ni les commentaires), se désactive dans Présentation ▸
  Documentation au survol (ou dans les Réglages), et **Afficher la
  documentation du symbole** (⌃⌘H) la demande pour le symbole sous le
  curseur — même souris désactivée.
- **Aller à la définition** (⌃⌘J, ou ⌘-clic) rejoint le symbole sous
  le curseur — d'un fichier à l'autre, en ouvrant ou en ramenant la
  cible au premier plan au besoin. Sur la définition il n'a nulle part
  où aller, alors il répond à la question qui reste : qui s'en sert.
  Un usage est un saut, plusieurs ouvrent la liste, et un symbole
  auquel rien ne renvoie le dit. Un serveur qui répond par plusieurs
  définitions — une déclaration et son implémentation — les propose de
  la même façon. Le raccourci de recherche des références ne change
  pas.
- **Chercher les références** (⇧⌘R) liste chaque usage du symbole sous
  le curseur dans un panneau flottant — ↑/↓ pour se déplacer, ⏎ pour
  sauter. Le code d'abord, les tests ensuite, chacun sous un titre
  avec son compte : ce qui appelle ceci est la question, ce qui le
  vérifie est la suite. Quels fichiers sont des tests est une
  convention et non un fait — un répertoire `tests`, un
  `parser_test.go`, un `Button.test.ts`, un `ParserTests.swift` — donc
  la règle est prudente, et `latest.rs` n'est pas un test. Un
  `#[cfg(test)] mod tests` de Rust dans un fichier ordinaire est
  listé comme du code, ce que dit son chemin. Si tout tombe du même
  côté, il n'y a pas de titres.
- **Formater le document** (⌥⇧⌘F) demande d'abord au serveur puis
  retombe sur la chaîne de préprocesseurs de sauvegarde — le formatage
  marche donc sur les documents sans titre et les langages sans
  serveur, dès qu'une chaîne est configurée.
- **Une ligne marquée se lit.** Poser le pointeur sur un soulignement
  montre ce qu'a dit le serveur, et **Afficher le diagnostic de la
  ligne** (⌃⌘E, Ctrl+Alt+E sous Linux) dit la même chose pour la ligne
  du curseur — le curseur est d'ordinaire en fin de ligne plutôt que
  dans la marque, c'est donc la ligne qui répond. Le message nomme sa
  gravité : un soulignement dit seulement que quelque chose ne va pas,
  et un avertissement ne doit pas se lire comme une erreur. Sans
  aller-retour : le diagnostic est déjà là.
- **Diagnostics…** (⇧⌘E, Ctrl+Shift+E sous Linux) liste tous les
  diagnostics du document dans l'ordre où ils apparaissent — celui dans
  lequel on les corrige et celui de la gouttière — avec la gravité sur
  chaque ligne. ⏎ y saute, et le saut entre dans la pile de retour.
- **Actions de code…** (⌘., Ctrl+. sous Linux) demande ce que le
  serveur peut faire de l'endroit où est le curseur — importer ce nom,
  ajouter la branche manquante, ôter la variable inutilisée — et liste
  ce qui revient, la suggestion du serveur étant signalée comme telle.
  Les signalements sous le curseur accompagnent la requête tels que le
  serveur les a publiés, `code` et `data` compris : c'est ainsi qu'un
  serveur reconnaît ce qu'il a lui-même trouvé, et un signalement
  reconstruit ne lui dit rien. Une action que le serveur a envoyée sans
  sa modification lui est renvoyée pour qu'il la termine avant qu'elle
  soit appliquée, et une action qui porte une commande plutôt qu'une
  modification est exécutée par le serveur.
- **Renommer le symbole…** (⌃⌘R) renomme dans tout l'espace de
  travail : les fenêtres ouvertes sont éditées sur place (l'annulation
  fonctionne par fenêtre) et les fichiers que personne n'a ouverts sont
  réécrits sur disque.
- **Formater le document** (⌥⇧⌘F) reformate via le serveur, en gardant
  les tabulations si le document indente avec des tabulations, des
  espaces sinon.
- **Plan du document** (⇧⌘O) liste les symboles du fichier —
  l'imbrication rendue par l'indentation, filtrable en flou — et ⏎
  saute vers la sélection.
- Un serveur manquant est signalé une seule fois, avec la commande qui
  l'installe ; tout le reste de l'éditeur continue de fonctionner sans
  lui.

## Serveurs

Textchum trouve les serveurs sur le `PATH` — il ne les installe pas :

| Langage | Serveur | Installation |
|---|---|---|
| Rust | rust-analyzer | `rustup component add rust-analyzer` |
| Python | pyright | `npm install -g pyright` |
| Go | gopls | `go install golang.org/x/tools/gopls@latest` |
| C | clangd | Xcode CLT, ou `brew install llvm` |
| JavaScript | typescript-language-server | `npm install -g typescript-language-server typescript` |
| Swift | sourcekit-lsp | fourni avec la chaîne d'outils Xcode |
| Zig | zls | `brew install zls` |
| Bash | bash-language-server | `npm install -g bash-language-server` |
| JSON | vscode-json-language-server | `npm install -g vscode-langservers-extracted` |
| HTML | vscode-html-language-server | `npm install -g vscode-langservers-extracted` |
| CSS | vscode-css-language-server | `npm install -g vscode-langservers-extracted` |
| YAML | yaml-language-server | `npm install -g yaml-language-server` |
| TOML | taplo | `brew install taplo` |
| Markdown | marksman | `brew install marksman` |

Les modèles Go sont servis par `gopls` également. Plusieurs langages ont
plus d'un serveur enregistré : Python dispose de `pyright`,
`basedpyright`, `pylsp`, `ruff`, `jedi`, `ty` et `pyrefly` ; JavaScript
de `typescript-language-server`, `vtsls`, `deno` et `biome`. Le tableau
nomme celui qui sert quand la configuration ne dit rien ; les autres se
demandent par identifiant.

## Choisir ses serveurs

Settings → Language Servers permet de décider quelle commande sert un
langage — pour tous les projets (un *défaut*) ou pour une racine de
projet précise. Les entrées de projet l'emportent sur les défauts ; les
langages sans entrée utilisent le tableau ci-dessus. Les entrées vivent
dans `config.json` sous `"lsp"`, avec les garanties d'édition à la main
habituelles du fichier :

```json
{
  "lsp": {
    "defaults": {"python": "pylsp"},
    "projects": {"/work/projA": {"python": "pyright-langserver --stdio"}}
  }
}
```

Le champ de langage propose les langages que cette compilation connaît
et accepte toujours n'importe quel texte : un langage peut être
configuré avant qu'une grammaire existe pour lui, et l'entrée sert
encore quand elle arrive.

### Définir un serveur que l'éditeur ne connaît pas

`lsp.servers` contient des entrées de la même forme que la table
intégrée : un serveur peut donc être ajouté sans changement de code, et
un serveur déjà connu redéfini en réutilisant son identifiant :

```json
{
  "lsp": {
    "servers": {
      "basedpyright": {
        "command": "{project}/.venv/bin/basedpyright-langserver",
        "args": ["--stdio"],
        "languages": ["python"],
        "install": "uv tool install basedpyright"
      }
    },
    "defaults": {"python": "basedpyright"}
  }
}
```

`command` est obligatoire ; le reste peut être omis. La table intégrée
reste disponible à côté de ces entrées : une configuration muette a donc
quand même des serveurs, et une version qui en apprend un nouveau le
propose sans réécriture de la configuration. Définir un serveur ne change
pas celui qu'un langage utilise par défaut ; c'est `lsp.defaults` qui en
décide.

### Nommer un serveur, et en désigner un dans le projet

L'entrée d'un langage accepte l'identifiant d'un serveur connu de
l'éditeur ou une ligne de commande.

Un identifiant apporte avec lui les arguments du serveur. Un langage qui
a plusieurs serveurs enregistrés utilise le premier tant que la
configuration n'en nomme pas un autre.

Une ligne de commande est exécutée telle quelle, avec deux
substitutions :

- `{project}` — la racine du projet sur laquelle l'instance est indexée.
- `{home}` — le répertoire personnel de l'utilisateur.

```json
{"lsp": {"defaults":
  {"python": "{project}/.venv/bin/basedpyright-langserver --stdio"}}}
```

C'est ce qu'il faut à un dépôt qui embarque ses propres outils : un
environnement virtuel, une entrée de `node_modules/.bin`, un serveur
inclus dans le dépôt. La substitution se fait argument par argument après
le découpage de la ligne : un chemin de projet contenant des espaces
reste un seul argument.

La commande d'une entrée s'édite sur place — corriger une coquille ou
ajouter un `--stdio` manquant se fait dans la ligne elle-même, avec ⏎
ou en cliquant ailleurs, sans supprimer puis recréer. Les changements
s'appliquent aux serveurs démarrés ensuite ; le bouton
**Restart Servers Now** de l'onglet retire les instances en cours et les
relance sous la nouvelle configuration.

## Quand il n'y a pas de serveur

Deux filets de sécurité couvrent le cas sans serveur :

- **Le repli ctags.** Avec **Ctags fallback** activé dans
  Réglages → Projects (par défaut ou par projet, comme chaque drapeau de
  projet), Aller à la Définition est répondu depuis un index
  [Universal Ctags](https://ctags.io) du projet dès qu'aucun serveur de
  langage n'est disponible — et aussi quand un serveur en marche n'a pas
  de réponse. L'index se construit au premier usage et se rafraîchit au
  fil des sauts ; ctags connaît des noms, pas la sémantique : c'est un
  repli, pas un remplacement. Il faut *Universal* Ctags (`brew install
  universal-ctags`) : le `ctags` livré par macOS dans `/usr/bin` est un
  autre programme, bien plus ancien, incapable de produire l'index JSON
  que ceci lit. Textchum regarde au-delà de celui-ci pour trouver un
  vrai Universal Ctags plus loin dans le `PATH`.
- **Le journal de débogage.** Chaque décision sur le chemin de « fichier
  ouvert » à « serveur en marche » — la racine de projet résolue, quel
  serveur a été choisi et pourquoi, les échecs de lancement avec le
  `PATH` exact consulté et chaque transition d'état — est ajoutée à :

  ```
  ~/Library/Logs/Textchum/lsp.log
  ```

  La sortie d'erreur propre à chaque serveur (stderr) y est capturée
  aussi : un serveur qui sort pendant le démarrage laisse sa plainte au
  dossier — une commande privée de son option de transport (le
  `--stdio` de pyright, par exemple) se diagnostique d'un coup d'œil,
  et le journal signale explicitement quand une commande personnalisée
  omet des arguments que le registre intégré sait requis. Quand un
  projet se retrouve mystérieusement sans support de langage, ce
  fichier nomme la pièce manquante.

Une cause classique mérite sa note : les applications lancées depuis le
Finder héritaient du `PATH` minimal de macOS, qui ne contient aucun des
endroits où vivent réellement les serveurs de langage (Homebrew, npm,
cargo, go). Textchum adopte désormais au démarrage le `PATH` du shell de
connexion — plus quelques répertoires d'outils conventionnels — si bien
qu'un serveur qui fonctionne depuis le terminal fonctionne aussi depuis
le Dock.

## Sous le capot

Le client vit dans le noyau, derrière la même frontière que tout le
reste : JSON-RPC sur stdio, une poignée de main d'initialisation avant
tout trafic de documents, et une synchronisation en document complet (la
synchronisation incrémentale est une optimisation ultérieure). Les
messages du serveur sont traités hors du fil d'interface et la
rejoignent par l'unique canal d'événements du noyau ; un processus
serveur bloqué reçoit un délai de grâce borné à la fermeture puis est
tué, si bien que quitter Textchum ne peut jamais rester suspendu à un
serveur défaillant. Tout le chemin du protocole est exercé en CI contre
un serveur scripté.

Les instances prennent aussi soin d'elles-mêmes : un serveur qui
**plante** en cours de session est relancé automatiquement avec recul
(1 → 2 → 4 → 8 secondes ; quatre échecs d'affilée et il reste à terre
jusqu'à un redémarrage ou un changement de configuration), et une
instance dont **aucun document ouvert n'a eu besoin depuis cinq
minutes** est arrêtée — l'ouverture suivante en lance une fraîche.

- Les snippets de la complétion se déplient et se parcourent. Le
  premier marqueur revient sélectionné, donc taper le remplace ; ⇥
  passe au suivant et ⇧⇥ au précédent ; un marqueur écrit plusieurs
  fois recopie celui qu'on tape. Arriver au bout, appuyer sur ⎋ ou
  cliquer ailleurs rend les touches.
- **Présentation ▸ État des serveurs** liste les instances en cours et
  les transitions récentes de la session, rafraîchi en direct, avec un
  pointeur vers le journal complet.
