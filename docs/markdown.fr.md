# Markdown

Markdown est un citoyen de première classe : le même fichier reçoit la
[coloration](highlighting.md) tree-sitter dans l'éditeur — y compris les
blocs de code clôturés colorés dans leur propre langage — et un **aperçu
en direct** à côté.

## L'aperçu

Ouvrir un document Markdown ouvre automatiquement le volet d'aperçu, à
droite de la source. **View → Toggle Markdown Preview** (⌥⌘P) le masque
et l'affiche.

- L'aperçu se met à jour pendant la frappe, rapiécé sur place — pas de
  rechargement, pas de scintillement, pas de position de défilement
  perdue.
- Faire défiler l'un des volets entraîne l'autre.
- Un lien s'ouvre dans le navigateur quand on clique dessus ; le volet
  reste sur le document. Un lien vers le document lui-même y fait
  défiler l'aperçu.
- Un clic droit sur l'aperçu propose **Enregistrer en PDF…** — la page
  rendue, écrite droit dans un fichier, sans outil externe.
- Le rendu prend en charge CommonMark plus les tableaux, le barré, les
  listes de tâches et les notes de bas de page, et ses styles suivent
  l'apparence du système (ou celle configurée).

Le rendu se fait dans le noyau (la coque ne possède que le volet) ; le
même HTML alimentera plus tard d'autres sorties.

## Hugo

Les billets écrits pour [Hugo](https://gohugo.io) sont du Markdown
avec deux ajouts, et Textchum lit les deux sans que Hugo soit
installé.

Le **front matter** — TOML entre `+++`, YAML entre `---` — est coloré
comme le langage qu'il est vraiment, tenu hors de la prose que lit le
correcteur (un slug n'est pas une faute), et rendu dans l'aperçu comme
un petit bloc de métadonnées plutôt qu'un paragraphe de signes.

Les **shortcodes** — `{{< figure src="…" >}}` et
`{{% notice %}}…{{% /notice %}}` — sont colorés comme les appels
qu'ils sont, ignorés par le correcteur, et affichés dans l'aperçu
comme un marqueur nommé. Ils ne sont jamais exécutés : il faudrait le
moteur de gabarits de Hugo et les layouts de votre site, donc un
marqueur est la chose honnête à montrer. Le corps d'un `{{% … %}}`
apparié continue d'être rendu comme du Markdown, comme le fait Hugo.

Le **plan** (⇧⌘O) liste les titres d'un billet même sans serveur de
langage, imbriqués par profondeur. Les titres dans un bloc de code ou
dans le front matter ne sont pas pris pour de la structure.

Enfin, les fichiers sous un répertoire `layouts/` sont traités comme
des **gabarits Go** plutôt que du HTML brut : le balisage est coloré
comme du HTML et les actions `{{ … }}` s'en détachent.

Le front matter JSON (la forme à accolades) n'est pas encore reconnu ;
TOML et YAML couvrent ce que Hugo écrit par défaut.

## Pas encore là

- La synchronisation de défilement précise par ancres de source (celle
  d'aujourd'hui est proportionnelle et dérive sur les documents aux blocs
  très inégaux).
- Les couleurs de syntaxe dans les blocs de code de l'aperçu (l'éditeur
  les colore ; l'aperçu les montre bruts).
- Le mode d'édition hybride/WYSIWYG — le volet d'aperçu est délibérément
  le premier des trois niveaux Markdown.
