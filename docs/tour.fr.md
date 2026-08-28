# Une visite

Chaque écran de Textchum, dans l'ordre où vous les rencontreriez, sur
les deux interfaces : l'app macOS à gauche et celle en GTK à droite.
Les captures viennent d'un petit projet fictif — *Harbor*, un courtier
de ports qui n'existe que pour donner à ces images quelque chose
d'honnête à montrer — et chacune suit votre réglage clair ou sombre.

Les deux interfaces sont le même éditeur sur le même cœur : les images
diffèrent surtout par ce qu'apporte la plateforme — l'habillage de la
fenêtre, l'endroit où un panneau se dessine, et la police que le
système fournit.

## La fenêtre

Un document par fenêtre, des onglets par défaut, et un tiroir de
navigation en deux moitiés : en haut les tampons ouverts groupés par
projet, en bas l'arbre de fichiers de ce projet.

<div class="shots" markdown>
<figure markdown>
[![La fenêtre de l'éditeur : barre latérale avec les tampons ouverts et l'arbre du projet, un fichier Rust coloré avec ses numéros de ligne (macOS)](images/editor.png#only-light)](images/editor.png)
[![La fenêtre de l'éditeur : barre latérale avec les tampons ouverts et l'arbre du projet, un fichier Rust coloré avec ses numéros de ligne (macOS)](images/editor-dark.png#only-dark)](images/editor-dark.png)
<figcaption>macOS</figcaption>
</figure>
<figure markdown>
[![La fenêtre de l'éditeur : barre latérale avec les tampons ouverts et l'arbre du projet, un fichier Rust coloré avec ses numéros de ligne (Linux)](images/editor-gtk.png#only-light)](images/editor-gtk.png)
[![La fenêtre de l'éditeur : barre latérale avec les tampons ouverts et l'arbre du projet, un fichier Rust coloré avec ses numéros de ligne (Linux)](images/editor-gtk-dark.png#only-dark)](images/editor-gtk-dark.png)
<figcaption>Linux</figcaption>
</figure>
</div>

La barre de titre porte les faits du document — encodage, taille,
langage, et le nombre de problèmes dès qu'un serveur de langage a un
avis. L'arbre suit : changer d'onglet déplie le chemin du fichier
courant et le met en évidence.

## Serveurs de langage

Les diagnostics arrivent en marques teintées dans le texte et en
compteur dans la barre de titre. Rien dans l'éditeur n'attend le
serveur : il s'attache quand il peut, et le dit quand il ne peut pas.

<div class="shots" markdown>
<figure markdown>
[![Un avertissement du serveur marqué dans le texte et compté dans la barre de titre (macOS)](images/diagnostics.png#only-light)](images/diagnostics.png)
[![Un avertissement du serveur marqué dans le texte et compté dans la barre de titre (macOS)](images/diagnostics-dark.png#only-dark)](images/diagnostics-dark.png)
<figcaption>macOS</figcaption>
</figure>
<figure markdown>
[![Un avertissement du serveur marqué dans le texte et compté dans la barre de titre (Linux)](images/diagnostics-gtk.png#only-light)](images/diagnostics-gtk.png)
[![Un avertissement du serveur marqué dans le texte et compté dans la barre de titre (Linux)](images/diagnostics-gtk-dark.png#only-dark)](images/diagnostics-gtk-dark.png)
<figcaption>Linux</figcaption>
</figure>
</div>

Laisser le pointeur sur un symbole affiche la documentation du
serveur, avec le Markdown qu'il envoie déjà rendu — blocs de code en
chasse fixe, emphase stylée. ⌃⌘H la demande pour le symbole sous le
curseur, ce qui marche même souris désactivée.

<div class="shots" markdown>
<figure markdown>
[![Documentation au survol d'une fonction, signature et prose rendues (macOS)](images/hover.png#only-light)](images/hover.png)
[![Documentation au survol d'une fonction, signature et prose rendues (macOS)](images/hover-dark.png#only-dark)](images/hover-dark.png)
<figcaption>macOS</figcaption>
</figure>
<figure markdown>
[![Documentation au survol d'une fonction, signature et prose rendues (Linux)](images/hover-gtk.png#only-light)](images/hover-gtk.png)
[![Documentation au survol d'une fonction, signature et prose rendues (Linux)](images/hover-gtk-dark.png#only-dark)](images/hover-gtk-dark.png)
<figcaption>Linux</figcaption>
</figure>
</div>

La complétion apparaît à la frappe après les caractères d'identifiant
et `.` ; ↑/↓ choisissent, ⏎ ou ⇥ acceptent, ⎋ ferme. Un snippet arrive
avec son premier marqueur sélectionné, donc taper le remplace ; ⇥ passe
au marqueur suivant et ⇧⇥ au précédent, et un marqueur écrit deux fois
se recopie à la frappe. Le dernier ⇥ laisse le curseur là où le snippet
l'a demandé et rend les touches.

<div class="shots" markdown>
<figure markdown>
[![La liste de complétion avec les membres et leurs types (macOS)](images/completion.png#only-light)](images/completion.png)
[![La liste de complétion avec les membres et leurs types (macOS)](images/completion-dark.png#only-dark)](images/completion-dark.png)
<figcaption>macOS</figcaption>
</figure>
<figure markdown>
[![La liste de complétion avec les membres et leurs types (Linux)](images/completion-gtk.png#only-light)](images/completion-gtk.png)
[![La liste de complétion avec les membres et leurs types (Linux)](images/completion-gtk-dark.png#only-dark)](images/completion-gtk-dark.png)
<figcaption>Linux</figcaption>
</figure>
</div>

**⇧⌘O** liste les symboles du fichier, filtrables au clavier.

<div class="shots" markdown>
<figure markdown>
[![Le panneau de plan du document, une structure et ses méthodes (macOS)](images/outline.png#only-light)](images/outline.png)
[![Le panneau de plan du document, une structure et ses méthodes (macOS)](images/outline-dark.png#only-dark)](images/outline-dark.png)
<figcaption>macOS</figcaption>
</figure>
<figure markdown>
[![Le panneau de plan du document, une structure et ses méthodes (Linux)](images/outline-gtk.png#only-light)](images/outline-gtk.png)
[![Le panneau de plan du document, une structure et ses méthodes (Linux)](images/outline-gtk-dark.png#only-dark)](images/outline-gtk-dark.png)
<figcaption>Linux</figcaption>
</figure>
</div>

**Présentation ▸ État des serveurs** répond à « mon serveur est-il
vivant ? » — ce qui tourne et où, plus les transitions récentes de la
session, rafraîchi en direct.

<div class="shots" markdown>
<figure markdown>
[![Le panneau d'état des serveurs avec une instance et ses transitions (macOS)](images/server-status.png#only-light)](images/server-status.png)
[![Le panneau d'état des serveurs avec une instance et ses transitions (macOS)](images/server-status-dark.png#only-dark)](images/server-status-dark.png)
<figcaption>macOS</figcaption>
</figure>
<figure markdown>
[![Le panneau d'état des serveurs avec une instance et ses transitions (Linux)](images/server-status-gtk.png#only-light)](images/server-status-gtk.png)
[![Le panneau d'état des serveurs avec une instance et ses transitions (Linux)](images/server-status-gtk-dark.png#only-dark)](images/server-status-gtk-dark.png)
<figcaption>Linux</figcaption>
</figure>
</div>

## Trouver des choses

**⌘T** ouvre les fichiers par nom flou dans le projet. La portée est
parcourue une fois puis filtrée en mémoire, donc la frappe reste
instantanée ; la ligne d'état dit combien de fichiers sur combien
correspondent et ce que font les touches — **⏎ cherche, ⌘⏎ ouvre**,
pour qu'affiner une requête n'ouvre jamais un fichier par accident.

<div class="shots" markdown>
<figure markdown>
[![Ouvrir rapidement : une requête floue, un chemin trouvé et la ligne d'état nommant les touches (macOS)](images/open-quickly.png#only-light)](images/open-quickly.png)
[![Ouvrir rapidement : une requête floue, un chemin trouvé et la ligne d'état nommant les touches (macOS)](images/open-quickly-dark.png#only-dark)](images/open-quickly-dark.png)
<figcaption>macOS</figcaption>
</figure>
<figure markdown>
[![Ouvrir rapidement : une requête floue, un chemin trouvé et la ligne d'état nommant les touches (Linux)](images/open-quickly-gtk.png#only-light)](images/open-quickly-gtk.png)
[![Ouvrir rapidement : une requête floue, un chemin trouvé et la ligne d'état nommant les touches (Linux)](images/open-quickly-gtk-dark.png#only-dark)](images/open-quickly-gtk-dark.png)
<figcaption>Linux</figcaption>
</figure>
</div>

**⇧⌘F** cherche dans le contenu avec une expression régulière, avec
des filtres empilés qui affinent par texte de ligne ou par chemin. La
ligne d'état dit toujours ce que la recherche a fait.

<div class="shots" markdown>
<figure markdown>
[![Chercher dans le projet : résultats de regex avec un filtre de fichier (macOS)](images/find-in-project.png#only-light)](images/find-in-project.png)
[![Chercher dans le projet : résultats de regex avec un filtre de fichier (macOS)](images/find-in-project-dark.png#only-dark)](images/find-in-project-dark.png)
<figcaption>macOS</figcaption>
</figure>
<figure markdown>
[![Chercher dans le projet : résultats de regex avec un filtre de fichier (Linux)](images/find-in-project-gtk.png#only-light)](images/find-in-project-gtk.png)
[![Chercher dans le projet : résultats de regex avec un filtre de fichier (Linux)](images/find-in-project-gtk-dark.png#only-dark)](images/find-in-project-gtk-dark.png)
<figcaption>Linux</figcaption>
</figure>
</div>

**⇧⌘P** est la palette de commandes : chaque action de menu, cherchable
en flou, avec son raccourci à côté.

<div class="shots" markdown>
<figure markdown>
[![La palette de commandes listant les actions et leurs raccourcis (macOS)](images/palette.png#only-light)](images/palette.png)
[![La palette de commandes listant les actions et leurs raccourcis (macOS)](images/palette-dark.png#only-dark)](images/palette-dark.png)
<figcaption>macOS</figcaption>
</figure>
<figure markdown>
[![La palette de commandes listant les actions et leurs raccourcis (Linux)](images/palette-gtk.png#only-light)](images/palette-gtk.png)
[![La palette de commandes listant les actions et leurs raccourcis (Linux)](images/palette-gtk-dark.png#only-dark)](images/palette-gtk-dark.png)
<figcaption>Linux</figcaption>
</figure>
</div>

## Markdown et prose

Les documents Markdown s'ouvrent avec un aperçu vivant à côté du
texte, et le correcteur orthographique de la prose — inactif jusqu'à
ce que vous choisissiez un dictionnaire — marque les fautes en
violet, distinct des diagnostics. Dans le code il ne regarde que les
commentaires ; les identifiants ne sont jamais signalés.

<div class="shots" markdown>
<figure markdown>
[![Un document Markdown avec son aperçu rendu à côté (macOS)](images/preview.png#only-light)](images/preview.png)
[![Un document Markdown avec son aperçu rendu à côté (macOS)](images/preview-dark.png#only-dark)](images/preview-dark.png)
<figcaption>macOS</figcaption>
</figure>
<figure markdown>
[![Un document Markdown avec son aperçu rendu à côté (Linux)](images/preview-gtk.png#only-light)](images/preview-gtk.png)
[![Un document Markdown avec son aperçu rendu à côté (Linux)](images/preview-gtk-dark.png#only-dark)](images/preview-gtk-dark.png)
<figcaption>Linux</figcaption>
</figure>
</div>

<div class="shots" markdown>
<figure markdown>
[![Des fautes marquées dans la prose, avec l'aperçu à côté (macOS)](images/spell-check.png#only-light)](images/spell-check.png)
[![Des fautes marquées dans la prose, avec l'aperçu à côté (macOS)](images/spell-check-dark.png#only-dark)](images/spell-check-dark.png)
<figcaption>macOS</figcaption>
</figure>
<figure markdown>
[![Des fautes marquées dans la prose, avec l'aperçu à côté (Linux)](images/spell-check-gtk.png#only-light)](images/spell-check-gtk.png)
[![Des fautes marquées dans la prose, avec l'aperçu à côté (Linux)](images/spell-check-gtk-dark.png#only-dark)](images/spell-check-gtk-dark.png)
<figcaption>Linux</figcaption>
</figure>
</div>

## Réglages

Les réglages sont un fichier JSON que la fenêtre édite ; le fichier
est la trappe de secours, et il est surveillé, donc une édition
ailleurs s'applique aussitôt.

<div class="shots" markdown>
<figure markdown>
[![Réglages, onglet Général : apparence, thème, placement, police et les interrupteurs de l'éditeur (macOS)](images/settings-general.png#only-light)](images/settings-general.png)
[![Réglages, onglet Général : apparence, thème, placement, police et les interrupteurs de l'éditeur (macOS)](images/settings-general-dark.png#only-dark)](images/settings-general-dark.png)
<figcaption>macOS</figcaption>
</figure>
<figure markdown>
[![Réglages, onglet Général : apparence, thème, placement, police et les interrupteurs de l'éditeur (Linux)](images/settings-general-gtk.png#only-light)](images/settings-general-gtk.png)
[![Réglages, onglet Général : apparence, thème, placement, police et les interrupteurs de l'éditeur (Linux)](images/settings-general-gtk-dark.png#only-dark)](images/settings-general-gtk-dark.png)
<figcaption>Linux</figcaption>
</figure>
</div>

**Projets** décide comment les racines sont trouvées, ce que l'arbre
cache, et quels réglages d'éditeur une racine remplace.

<div class="shots" markdown>
<figure markdown>
[![Réglages, onglet Projets : détection, motifs cachés et remplacements par projet (macOS)](images/settings-projects.png#only-light)](images/settings-projects.png)
[![Réglages, onglet Projets : détection, motifs cachés et remplacements par projet (macOS)](images/settings-projects-dark.png#only-dark)](images/settings-projects-dark.png)
<figcaption>macOS</figcaption>
</figure>
<figure markdown>
[![Réglages, onglet Projets : détection, motifs cachés et remplacements par projet (Linux)](images/settings-projects-gtk.png#only-light)](images/settings-projects-gtk.png)
[![Réglages, onglet Projets : détection, motifs cachés et remplacements par projet (Linux)](images/settings-projects-gtk-dark.png#only-dark)](images/settings-projects-gtk-dark.png)
<figcaption>Linux</figcaption>
</figure>
</div>

Les noms cachés sont des motifs glob, édités un par ligne, avec un
menu qui ajoute un préréglage nommé en un clic.

<div class="shots" markdown>
<figure markdown>
[![L'éditeur de motifs cachés en popover, un motif par ligne, avec le menu des préréglages (macOS)](images/hide-globs.png#only-light)](images/hide-globs.png)
[![L'éditeur de motifs cachés en popover, un motif par ligne, avec le menu des préréglages (macOS)](images/hide-globs-dark.png#only-dark)](images/hide-globs-dark.png)
<figcaption>macOS</figcaption>
</figure>
<figure markdown>
[![L'éditeur de motifs cachés en popover, un motif par ligne, avec le menu des préréglages (Linux)](images/hide-globs-gtk.png#only-light)](images/hide-globs-gtk.png)
[![L'éditeur de motifs cachés en popover, un motif par ligne, avec le menu des préréglages (Linux)](images/hide-globs-gtk-dark.png#only-dark)](images/hide-globs-gtk-dark.png)
<figcaption>Linux</figcaption>
</figure>
</div>

**Presets** édite ces ensembles nommés de la même façon. Ils
commencent intégrés ; éditez-en un et votre liste prend la main, donc
celui que vous supprimez le reste jusqu'à restauration. Cet écran et
le suivant n'ont pas d'image de l'interface GTK parce qu'ils n'y sont
pas des écrans : les presets vivent dans Projects, et les
préprocesseurs dans Language Servers.

![Réglages, onglet Presets : ensembles glob nommés, chacun éditable un
motif par ligne](images/settings-presets.png#only-light)
![Réglages, onglet Presets : ensembles glob nommés, chacun éditable un
motif par ligne](images/settings-presets-dark.png#only-dark)

**Serveurs de langage** remplace la commande qui sert un langage, pour
tous les projets ou pour une racine.

<div class="shots" markdown>
<figure markdown>
[![Réglages, onglet Serveurs de langage : commandes par défaut et par projet (macOS)](images/settings-servers.png#only-light)](images/settings-servers.png)
[![Réglages, onglet Serveurs de langage : commandes par défaut et par projet (macOS)](images/settings-servers-dark.png#only-dark)](images/settings-servers-dark.png)
<figcaption>macOS</figcaption>
</figure>
<figure markdown>
[![Réglages, onglet Serveurs de langage : commandes par défaut et par projet (Linux)](images/settings-servers-gtk.png#only-light)](images/settings-servers-gtk.png)
[![Réglages, onglet Serveurs de langage : commandes par défaut et par projet (Linux)](images/settings-servers-gtk-dark.png#only-dark)](images/settings-servers-gtk-dark.png)
<figcaption>Linux</figcaption>
</figure>
</div>

**Préprocesseurs** lance des formateurs avant chaque sauvegarde : une
commande par ligne, chacune lisant le document sur l'entrée standard
et le réécrivant sur la sortie standard.

![Réglages, onglet Préprocesseurs : chaînes de commandes par
langage](images/settings-preprocessors.png#only-light)
![Réglages, onglet Préprocesseurs : chaînes de commandes par
langage](images/settings-preprocessors-dark.png#only-dark)

## Petites choses

**⇧⌘N** démarre un document dans le langage choisi, filtré au clavier,
pour que la coloration marche avant la première sauvegarde.

<div class="shots" markdown>
<figure markdown>
[![Le sélecteur Nouveau avec format, filtrant la liste des langages (macOS)](images/new-with-format.png#only-light)](images/new-with-format.png)
[![Le sélecteur Nouveau avec format, filtrant la liste des langages (macOS)](images/new-with-format-dark.png#only-dark)](images/new-with-format-dark.png)
<figcaption>macOS</figcaption>
</figure>
<figure markdown>
[![Le sélecteur Nouveau avec format, filtrant la liste des langages (Linux)](images/new-with-format-gtk.png#only-light)](images/new-with-format-gtk.png)
[![Le sélecteur Nouveau avec format, filtrant la liste des langages (Linux)](images/new-with-format-gtk-dark.png#only-dark)](images/new-with-format-gtk-dark.png)
<figcaption>Linux</figcaption>
</figure>
</div>

Et le panneau À propos dit quelle version vous utilisez — une vraie
version, même pour une compilation locale.

<div class="shots" markdown>
<figure markdown>
[![Le panneau À propos avec la version, l'auteur, le dépôt et la licence (macOS)](images/about.png#only-light)](images/about.png)
[![Le panneau À propos avec la version, l'auteur, le dépôt et la licence (macOS)](images/about-dark.png#only-dark)](images/about-dark.png)
<figcaption>macOS</figcaption>
</figure>
<figure markdown>
[![Le panneau À propos avec la version, l'auteur, le dépôt et la licence (Linux)](images/about-gtk.png#only-light)](images/about-gtk.png)
[![Le panneau À propos avec la version, l'auteur, le dépôt et la licence (Linux)](images/about-gtk-dark.png#only-dark)](images/about-gtk-dark.png)
<figcaption>Linux</figcaption>
</figure>
</div>
