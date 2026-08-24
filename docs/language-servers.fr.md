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
  serveur dans une bulle.
- **Aller à la définition** (⌃⌘J) rejoint le symbole sous le curseur —
  d'un fichier à l'autre, en ouvrant ou en ramenant la cible au premier
  plan au besoin.
- **Chercher les références** (⇧⌘R) liste chaque usage du symbole sous
  le curseur dans un panneau flottant — ↑/↓ pour se déplacer, ⏎ pour
  sauter.
- **Renommer le symbole…** (⌃⌘R) renomme dans tout l'espace de
  travail : les fenêtres ouvertes sont éditées sur place (l'annulation
  fonctionne par fenêtre) et les fichiers que personne n'a ouverts sont
  réécrits sur disque.
- **Formater le document** (⌥⇧⌘F) reformate via le serveur, en gardant
  les tabulations si le document indente avec des tabulations, des
  espaces sinon.
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
  repli, pas un remplacement.
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

## Pas encore là

- Les marqueurs de fragments (snippets) dans la complétion sont aplanis
  en texte brut.
- ⌘-clic comme déclencheur alternatif d'Aller à la définition.
- Le rendu Markdown dans les bulles de survol (elles montrent le texte
  brut).
- Le redémarrage automatique des serveurs plantés (un plantage est
  signalé ; rouvrir le fichier lance une instance neuve).
- L'arrêt d'inactivité des instances inutilisées et un panneau d'état des
  serveurs.
