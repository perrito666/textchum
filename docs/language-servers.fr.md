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

- Complétion, survol, aller à la définition, références, renommage,
  formatage — les diagnostics sont venus d'abord parce que la validation
  est la promesse centrale du produit.
- Le redémarrage automatique des serveurs plantés (un plantage est
  signalé ; rouvrir le fichier lance une instance neuve).
- L'arrêt d'inactivité des instances inutilisées et un panneau d'état des
  serveurs.
- La configuration de serveurs personnalisée (un `servers.json` avec les
  règles de secours de la configuration).
