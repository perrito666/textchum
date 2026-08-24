# Premiers pas

La plateforme d'origine de Textchum est macOS ; une coque Linux
expérimentale sur le même noyau se compile aussi — voir
[Linux](#linux-experimental) plus bas.

## Prérequis

- macOS 14 ou plus récent.
- La chaîne d'outils Swift, version 6 ou supérieure. Les Xcode Command Line
  Tools suffisent (`xcode-select --install`) ; l'application Xcode complète
  n'est pas nécessaire.
- Une chaîne d'outils Rust (stable). Le plus simple est de l'installer avec
  [rustup](https://rustup.rs).
- `make`, fourni avec les Command Line Tools.

## Compiler et lancer

```sh
git clone https://github.com/perrito666/textchum
cd textchum
make run
```

`make run` compile le noyau Rust en bibliothèque statique, génère l'en-tête
C, compile l'application Swift contre celle-ci et lance l'éditeur.

Autres cibles utiles :

| Cible | Effet |
|---|---|
| `make build` | Compile le noyau et l'application sans la lancer. |
| `make test` | Exécute la suite de tests Rust. |
| `make smoke` | Compile tout puis exécute le test de fumée sans interface. |
| `make check` | Tout ce que lance la CI : tests, test de fumée, contrôle de dérive de l'en-tête. |
| `make app` | Construit un `Textchum.app` double-cliquable (avec icône) dans `dist/`. |
| `make docs` | Construit ce site de documentation dans `site/`. |
| `make clean` | Supprime tous les produits de compilation. |

Pas envie de compiler ? Chaque étiquette `v*` publie une
[release GitHub](https://github.com/perrito666/textchum/releases) avec
un zip de `Textchum.app` prêt à l'emploi (et son SHA-256).
L'application n'est pas signée : au premier lancement, faites un clic
droit dessus et choisissez Ouvrir.

## Structure du dépôt

```
textchum/
├── core/                    espace de travail Rust
│   ├── textchum-core/         le noyau de l'éditeur (tampons, événements)
│   └── textchum-ffi/          ABI C au-dessus du noyau ; génère textchum.h
├── macos/                   paquet Swift
│   └── Sources/
│       ├── CTextchum/         l'en-tête C généré, en module Clang
│       ├── TextchumKit/       enveloppe Swift sûre au-dessus de l'interface C
│       └── Textchum/          l'application AppKit
├── docs/                    cette documentation (MkDocs)
└── Makefile                 le point d'entrée de toutes les tâches
```

## La commande `chum`

**Textchum → Install chum Command…** (ou, depuis un checkout,
`make install-cli` — respecte `PREFIX`, `/usr/local` par défaut)
installe une petite commande de terminal qui parle à l'application en
cours ; la voie du menu ne demande les droits d'administration que si
`/usr/local/bin` l'exige :

```sh
chum notes.md                # ouvrir (onglet ou fenêtre selon vos réglages)
chum +42 src/main.rs         # ouvrir avec le curseur à la ligne 42
chum -w grand.md             # forcer une fenêtre séparée
chum -t a.rs +7 b.rs         # plusieurs fichiers, onglets, un avec ligne
chum --wait brouillon.md     # bloque jusqu'à la fermeture de la fenêtre
```

`--wait` est ce qu'il faut aux outils qui lancent un éditeur puis
lisent le fichier — enregistrez, fermez la fenêtre, et l'appelant
reprend :

```sh
git config --global core.editor "chum --wait"
```

Fermer sans enregistrer laisse le fichier intact, ce que git lit comme
un commit abandonné — le même geste que `:q!`. Si Textchum quitte (ou
n'est plus là), les chum en attente sont libérés plutôt que laissés
suspendus.

Elle passe par le schéma d'URL `textchum://` ; le paquet de
l'application (`make app`) doit donc avoir été lancé au moins une fois
pour l'enregistrer.

## Linux (expérimental)

Le même noyau anime une coque native GTK4/libadwaita, liée comme crate
Rust plutôt qu'à travers l'en-tête C (là, les deux côtés sont Rust).
Plus jeune que l'application macOS mais plus un jouet : onglets
(AdwTabView) qui focalisent au lieu de dupliquer, arbre de fichiers du
projet en barre latérale (F9), édition et annulation propriété du
noyau, coloration tree-sitter depuis la table de thèmes partagée,
recherche dans le fichier (Ctrl+F), recherche dans le projet
(Ctrl+Shift+F, regex avec smart case, filtres empilables ligne/fichier
et ligne d'état qui dit ce qu'elle a fait), ouverture floue (Ctrl+P), une liste de fichiers
ouverts groupée par projet au-dessus de l'arbre, le pool de serveurs
de langage branché (diagnostics soulignés avec compte de problèmes,
complétion pendant la frappe, survol, saut à la définition en F12,
ennuis de serveur en avis), un volet d'aperçu Markdown en direct
(Ctrl+Alt+P) et
une fenêtre de préférences (Ctrl+,) sur le même contrat `config.json`
— apparence, thème, réglages d'éditeur et serveurs de langage —
défauts, surcharges par projet et interrupteurs d'espace de travail —,
rangé dans `~/.config/textchum/config.json`.

```sh
sudo apt install libgtk-4-dev libadwaita-1-dev libgtksourceview-5-dev libwebkitgtk-6.0-dev
cargo build --release --manifest-path linux/Cargo.toml
linux/target/release/textchum-gtk notes.md
```

La CI le compile et lance sa fumée sans écran à chaque push.

## Construire la documentation

La documentation est un site [MkDocs](https://www.mkdocs.org) avec le thème
Material, publiée en anglais, espagnol et français. Elle est entièrement
statique : le répertoire `site/` généré se sert avec n'importe quel serveur
web.

```sh
python3 -m venv .docs-venv
.docs-venv/bin/pip install -r docs/requirements.txt
.docs-venv/bin/mkdocs serve    # aperçu avec rechargement sur localhost:8000
.docs-venv/bin/mkdocs build    # site statique dans site/
```

`make docs` enveloppe ces mêmes étapes.

## Dépannage

- **Erreurs `xcodebuild` mentionnant une « command line tools instance »** —
  sans gravité ; Textchum n'utilise pas `xcodebuild`. Compilez avec `make`
  (qui pilote `swift build`).
- **L'éditeur de liens ne trouve pas `-ltextchum`** — le noyau Rust n'a pas
  encore été compilé. Lancez `make core` (ou toute cible `make` qui
  l'inclut) avant d'invoquer `swift build` à la main.
