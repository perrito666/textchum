# Une visite de l'app macOS

Chaque écran de Textchum, dans l'ordre où vous les rencontreriez. Les
captures viennent d'un petit projet fictif — *Harbor*, un courtier de
ports qui n'existe que pour donner à ces images quelque chose
d'honnête à montrer.

## La fenêtre

Un document par fenêtre, des onglets par défaut, et un tiroir de
navigation en deux moitiés : en haut les tampons ouverts groupés par
projet, en bas l'arbre de fichiers de ce projet.

![La fenêtre de l'éditeur : barre latérale avec les tampons ouverts et
l'arbre du projet, un fichier Rust coloré avec ses numéros de
ligne](images/editor.png)

La barre de titre porte les faits du document — encodage, taille,
langage, et le nombre de problèmes dès qu'un serveur de langage a un
avis. L'arbre suit : changer d'onglet déplie le chemin du fichier
courant et le met en évidence.

## Serveurs de langage

Les diagnostics arrivent en marques teintées dans le texte et en
compteur dans la barre de titre. Rien dans l'éditeur n'attend le
serveur : il s'attache quand il peut, et le dit quand il ne peut pas.

![Un avertissement du serveur marqué dans le texte et compté dans la
barre de titre](images/diagnostics.png)

Laisser le pointeur sur un symbole affiche la documentation du
serveur, avec le Markdown qu'il envoie déjà rendu — blocs de code en
chasse fixe, emphase stylée. ⌃⌘H la demande pour le symbole sous le
curseur, ce qui marche même souris désactivée.

![Documentation au survol d'une fonction, signature et prose
rendues](images/hover.png)

La complétion apparaît à la frappe après les caractères d'identifiant
et `.` ; ↑/↓ choisissent, ⏎ ou ⇥ acceptent, ⎋ ferme. Un snippet arrive
avec son premier marqueur sélectionné, donc taper le remplace.

![La liste de complétion avec les membres et leurs
types](images/completion.png)

**⇧⌘O** liste les symboles du fichier, filtrables au clavier.

![Le panneau de plan du document, une structure et ses
méthodes](images/outline.png)

**Présentation ▸ État des serveurs** répond à « mon serveur est-il
vivant ? » — ce qui tourne et où, plus les transitions récentes de la
session, rafraîchi en direct.

![Le panneau d'état des serveurs avec une instance et ses
transitions](images/server-status.png)

## Trouver des choses

**⌘T** ouvre les fichiers par nom flou dans le projet. La portée est
parcourue une fois puis filtrée en mémoire, donc la frappe reste
instantanée ; la ligne d'état dit combien de fichiers sur combien
correspondent et ce que font les touches — **⏎ cherche, ⌘⏎ ouvre**,
pour qu'affiner une requête n'ouvre jamais un fichier par accident.

![Ouvrir rapidement : une requête floue, un chemin trouvé et la ligne
d'état nommant les touches](images/open-quickly.png)

**⇧⌘F** cherche dans le contenu avec une expression régulière, avec
des filtres empilés qui affinent par texte de ligne ou par chemin. La
ligne d'état dit toujours ce que la recherche a fait.

![Chercher dans le projet : résultats de regex avec un filtre de
fichier](images/find-in-project.png)

**⇧⌘P** est la palette de commandes : chaque action de menu, cherchable
en flou, avec son raccourci à côté.

![La palette de commandes listant les actions et leurs
raccourcis](images/palette.png)

## Markdown et prose

Les documents Markdown s'ouvrent avec un aperçu vivant à côté du
texte, et le correcteur orthographique de la prose — inactif jusqu'à
ce que vous choisissiez un dictionnaire — marque les fautes en
violet, distinct des diagnostics. Dans le code il ne regarde que les
commentaires ; les identifiants ne sont jamais signalés.

![Un document Markdown avec son aperçu rendu à
côté](images/preview.png)

![Des fautes marquées dans la prose, avec l'aperçu à
côté](images/spell-check.png)

## Réglages

Les réglages sont un fichier JSON que la fenêtre édite ; le fichier
est la trappe de secours, et il est surveillé, donc une édition
ailleurs s'applique aussitôt.

![Réglages, onglet Général : apparence, thème, placement, police et les
interrupteurs de l'éditeur](images/settings-general.png)

**Projets** décide comment les racines sont trouvées, ce que l'arbre
cache, et quels réglages d'éditeur une racine remplace.

![Réglages, onglet Projets : détection, motifs cachés et remplacements
par projet](images/settings-projects.png)

Les noms cachés sont des motifs glob, édités un par ligne, avec un
menu qui ajoute un préréglage nommé en un clic.

![L'éditeur de motifs cachés en popover, un motif par ligne, avec le
menu des préréglages](images/hide-globs.png)

**Presets** édite ces ensembles nommés de la même façon. Ils
commencent intégrés ; éditez-en un et votre liste prend la main, donc
celui que vous supprimez le reste jusqu'à restauration.

![Réglages, onglet Presets : ensembles glob nommés, chacun éditable un
motif par ligne](images/settings-presets.png)

**Serveurs de langage** remplace la commande qui sert un langage, pour
tous les projets ou pour une racine.

![Réglages, onglet Serveurs de langage : commandes par défaut et par
projet](images/settings-servers.png)

**Préprocesseurs** lance des formateurs avant chaque sauvegarde : une
commande par ligne, chacune lisant le document sur l'entrée standard
et le réécrivant sur la sortie standard.

![Réglages, onglet Préprocesseurs : chaînes de commandes par
langage](images/settings-preprocessors.png)

## Petites choses

**⇧⌘N** démarre un document dans le langage choisi, filtré au clavier,
pour que la coloration marche avant la première sauvegarde.

![Le sélecteur Nouveau avec format, filtrant la liste des
langages](images/new-with-format.png)

Et le panneau À propos dit quelle version vous utilisez — une vraie
version, même pour une compilation locale.

![Le panneau À propos avec la version, l'auteur, le dépôt et la
licence](images/about.png)
