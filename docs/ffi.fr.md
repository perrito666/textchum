# La frontière C

La coque et le noyau se rejoignent sur un unique en-tête C, `textchum.h`.
Il est généré depuis les sources Rust par
[cbindgen](https://github.com/mozilla/cbindgen) à chaque compilation du
noyau, et versionné dans le dépôt pour que l'outillage côté Swift fonctionne
sans chaîne Rust installée. La CI échoue si une compilation laisse l'en-tête
périmé : il ne peut donc jamais dériver du code.

## Conventions

Chaque fonction de l'interface suit le même petit ensemble de règles.

**Poignées opaques.** Les types du noyau (`TcApp`, `TcBuffer`) sont des
structures opaques ; l'appelant détient des pointeurs, les repasse à chaque
appel et les libère avec la fonction `tc_*_free` correspondante. L'appelant
n'alloue jamais de mémoire pour le compte du noyau.

**UTF-8 en entrée, longueurs explicites.** Les chaînes *entrant* dans le
noyau sont des paires `(pointeur, longueur)` d'octets UTF-8 — pas de
terminateur nul exigé, pas d'autre encodage que l'UTF-8. Les chaînes
*retournées* par le noyau sont en UTF-8 terminé par nul et lui
appartiennent ; on les libère avec `tc_string_free`.

**Deux unités de position.** Les fonctions adressent le texte soit en
décalages d'octets UTF-8 (l'unité native du noyau), soit en unités UTF-16
(suffixe `_utf16`), car c'est ce que comptent `NSRange` et le Language
Server Protocol. Le noyau fait la conversion ; l'appelant utilise l'unité
qu'il possède naturellement.

**Échec transactionnel.** Les appels faillibles renvoient un `bool`. `false`
signifie que l'entrée a été validée, rejetée, et que **rien n'a changé** —
décalage hors limites, position au milieu d'un caractère, UTF-8 invalide.
L'appelant peut toujours traiter l'échec comme « l'opération n'a pas eu
lieu ».

**Les paniques ne traversent pas.** Chaque point d'entrée capture les
paniques Rust et les convertit en valeur d'échec de la fonction. Un bogue du
noyau ne peut pas se dérouler dans des cadres de pile Swift.

**Un fil en entrée, un fil en sortie.** Les appels vers le noyau doivent
venir d'un seul fil. Les événements circulent en sens inverse par le
callback enregistré avec `tc_app_new`, invoqué sur un unique fil de
distribution appartenant au noyau. Le rôle du callback est de faire passer
l'événement sur le fil d'interface de la coque ; `TextchumKit` fait
exactement cela et rien d'autre.

## Le canal d'événements

Certaines informations naissent à l'intérieur du noyau (aujourd'hui : les
réponses *pong* servant à vérifier le canal ; bientôt : les diagnostics des
serveurs de langage, les invalidations de coloration). Elles parviennent à
la coque sous forme de `TcEvent` — un discriminant `kind` plus la charge de
l'événement — livré au callback enregistré.

Les coques doivent tolérer les valeurs de `kind` inconnues : un noyau plus
récent émettant un événement qu'une coque plus ancienne ne comprend pas
relève de la compatibilité ascendante, pas de l'erreur.

`tc_app_free` bloque jusqu'à la livraison des événements en file et
garantit que le callback n'est plus jamais invoqué ensuite — c'est ce qui
rend le démontage sûr à écrire côté coque.

## Surface actuelle

| Fonction | Rôle |
|---|---|
| `tc_version` | Version du noyau en chaîne statique. |
| `tc_app_new` / `tc_app_free` | Créer/détruire une instance du noyau et son canal d'événements. |
| `tc_app_ping` | Demander un *pong* asynchrone ; exerce le chemin des événements. |
| `tc_buffer_new` / `tc_buffer_free` | Créer/détruire un tampon de texte. |
| `tc_buffer_insert` | Insérer de l'UTF-8 à un décalage d'octets. |
| `tc_buffer_delete` | Supprimer une plage d'octets. |
| `tc_buffer_replace_utf16` | Remplacer une plage d'unités UTF-16 — la forme d'une édition AppKit. |
| `tc_buffer_text` | Copier le contenu complet. |
| `tc_buffer_len_bytes` / `tc_buffer_len_utf16` | Longueurs dans les deux unités. |
| `tc_document_new` / `tc_document_open` / `tc_document_free` | Créer un document (vide ou depuis un fichier) et le détruire. |
| `tc_document_replace_utf16` | Éditer un document en alimentant l'historique d'annulation. |
| `tc_document_undo` / `tc_document_redo` | Parcourir l'historique ; un paramètre de sortie décrit l'édition que la coque doit rejouer. |
| `tc_document_break_undo_group` | Terminer la séquence de fusion d'annulation en cours. |
| `tc_document_save` / `tc_document_save_as` | Enregistrements atomiques ; en cas d'échec, un paramètre de sortie optionnel reçoit un message. |
| `tc_document_text` / `tc_document_len_bytes` / `tc_document_len_utf16` | Contenu et longueurs. |
| `tc_document_is_dirty` / `tc_document_can_undo` / `tc_document_can_redo` | Requêtes d'état. |
| `tc_document_path` / `tc_document_encoding_name` | Identité du fichier et encodage. |
| `tc_string_free` | Libérer une chaîne retournée par le noyau. |

Les opérations de fichier faillibles suivent une convention de plus : elles
renvoient leur valeur d'échec et, si l'appelant a fourni un paramètre de
sortie non nul, y déposent un message UTF-8 lisible (libéré avec
`tc_string_free`) — la coque l'affiche tel quel dans l'alerte.

La surface est volontairement réduite et ne grandit que lorsqu'une
fonctionnalité de la coque l'exige. Les données volumineuses (à venir :
segments de coloration, diagnostics) traverseront sous forme de structures
compactes ou de charges sérialisées, plutôt qu'un appel par élément.

## Comment Swift la consomme

La cible `CTextchum` enveloppe l'en-tête en module Clang ; Swift l'importe
donc comme n'importe quelle bibliothèque. `TextchumKit` traduit ensuite les
appels bruts en Swift idiomatique — classes à propriété fondée sur `deinit`,
paramètres `NSRange`, erreurs levées pour les opérations rejetées et
événements typés livrés sur l'acteur principal. Le code applicatif ne touche
jamais un pointeur.
