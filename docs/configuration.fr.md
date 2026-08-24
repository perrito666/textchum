# Configuration

Les réglages de Textchum suivent un principe : **l'interface graphique est
le moyen confortable de les changer, et un simple fichier JSON est la
sortie de secours toujours disponible.** Il y a exactement un seul dépôt —
le fichier — et la fenêtre Réglages le lit et l'écrit ; rien ne vit
uniquement à l'intérieur de l'application.

## La fenêtre Réglages

**Textchum → Settings…** (⌘,) édite les réglages reconnus :

- **Police** — toute famille à chasse fixe installée sur le système, ou la
  police à chasse fixe de la plateforme.
- **Taille de police** — de 6 à 72 points.
- **Largeur de tabulation** — de 1 à 16 colonnes.

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
  "editor": {
    "font_family": "JetBrains Mono",
    "font_size": 13,
    "tab_width": 4
  }
}
```

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

## Pas encore là

- Textchum ne surveille pas encore le fichier en cours d'exécution ; les
  changements faits dans un autre éditeur s'appliquent au prochain
  lancement.
- Les réglages par projet.
