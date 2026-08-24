# Architecture

Textchum, ce sont deux programmes qui se rejoignent sur une interface C :

```
┌──────────────────────────────────────────────┐
│ Coque — Swift, AppKit                        │
│  fenêtres · rendu · saisie · menus           │
└───────────────▲──────────────┬───────────────┘
                │ appels C     │ callback d'événements
┌───────────────┴──────────────▼───────────────┐
│ Noyau — Rust, libtextchum (bibliothèque      │
│ statique)                                    │
│  tampons · éditions · événements             │
│  (bientôt : syntaxe, projets, serveurs de    │
│  langage)                                    │
└──────────────────────────────────────────────┘
```

Le partage des rôles suit une règle simple : tout ce qui répond à
*« qu'est-ce que le texte, et que sait-on de lui ? »* appartient au noyau ;
tout ce qui répond à *« quel aspect et quel comportement sur ce système ? »*
appartient à la coque. Le noyau ne dessine jamais. La coque n'analyse jamais
le texte.

## Pourquoi un noyau compilé derrière une coque native

- **Les problèmes difficiles sont indépendants de la plateforme.** *Ropes*,
  analyse incrémentale, clients de protocoles — rien de tout cela ne connaît
  AppKit. Les garder dans une bibliothèque nue les rend testables sans
  interface (`cargo test` couvre le noyau sans la moindre UI) et portables
  vers d'autres plateformes plus tard.
- **La couche visible doit être banalement native.** La saisie de texte sous
  macOS est profonde — IME, touches mortes, dictée, accessibilité. Utiliser
  de vraies vues AppKit, c'est hériter de tout cela au lieu de le
  réimplémenter.
- **Une ABI C est la frontière la plus large possible.** Swift consomme les
  en-têtes C nativement ; comme tout ce qui pourrait un jour héberger le
  noyau.

## La règle de la source de vérité

L'invariant le plus important du code : **le tampon du noyau possède le
document ; ce que l'interface détient n'est qu'un cache d'affichage.**

Concrètement, dans la fenêtre d'édition actuelle :

1. AppKit signale chaque changement de texte imminent — frappe, collage,
   dépôt, annulation — via une seule méthode déléguée, sous la forme d'une
   plage UTF-16 et d'une chaîne de remplacement.
2. La coque applique exactement cette édition au tampon du noyau *d'abord*.
3. La vue ne procède à son propre changement que si le noyau l'accepte. Un
   refus (qui révélerait un bogue) refuse aussi l'édition côté vue, si bien
   que les deux côtés ne peuvent avancer qu'ensemble.
4. Les compilations de débogage vérifient de surcroît l'égalité octet par
   octet des deux côtés après chaque changement.

Les positions franchissent la frontière dans les deux unités réellement
utilisées par l'écosystème : décalages d'octets (l'unité native du noyau) et
unités UTF-16 (l'unité native d'AppKit et de LSP). Le noyau fait toutes les
conversions ; la coque ne compte jamais de points de code.

## Contrat de fils d'exécution

Des règles simples, strictement tenues :

- La coque n'appelle le noyau **que depuis le fil principal**.
- Le noyau possède tous les fils de travail et livre ses événements par
  **un** callback invoqué depuis **un** fil de distribution dédié — jamais
  depuis le fil de l'appelant, jamais en concurrence avec lui-même.
- L'enveloppe Swift (`TextchumKit`) fait passer les événements sur l'acteur
  principal avant que l'application ne les voie ; le code applicatif vit
  ainsi entièrement sur l'acteur principal.

Cela coûte un peu de parallélisme à la frontière et achète l'absence de
toute une catégorie de situations de concurrence. Le travail qui profite du
parallélisme se déroule *à l'intérieur* du noyau, derrière l'interface
mono-fil.

## Les couches côté Swift

| Couche | Responsabilité |
|---|---|
| `CTextchum` | L'en-tête C généré, exposé en module Clang. Aucun code. |
| `TextchumKit` | API Swift sûre : classes à propriété déterministe, édition par `NSRange`, événements typés sur l'acteur principal. Le seul endroit où apparaissent des pointeurs. |
| `Textchum` | L'application : fenêtres, vues, menus. Du Swift ordinaire, sans FFI. |

Toute coque future devra suivre la même stratification : une liaison mince,
une enveloppe idiomatique sûre, puis l'application.
