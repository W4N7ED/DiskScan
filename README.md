# DiskScan — Analyseur d'espace disque (Rust)

Deuxième outil de la suite d'administration système après [NetScan](https://github.com/W4N7ED/NetScan) : même pile, même design, en un seul exécutable Windows (aucun runtime à installer, aucun droit administrateur requis).

## Fonctionnalités

| | |
|---|---|
| **Cible** | liste des lecteurs (nom, taille, espace libre, barre d'occupation) ou n'importe quel dossier (champ, bouton « Parcourir », ligne de commande) |
| **Analyse** | multi-thread en arrière-plan, annulable (Échap), résultats affichés au fil de l'eau ; un disque entier de 1,4 million de fichiers en une vingtaine de secondes |
| **Arborescence** | dépliable, triée par taille : nom, taille, part du parent (avec barre), fichiers, dossiers, date de dernière modification, évolution depuis la dernière analyse |
| **Tailles** | taille réelle ou **taille occupée sur le disque** (clusters, fichiers compressés NTFS, fichiers creux, fichiers disponibles en ligne seulement) |
| **Fiabilité** | les liens symboliques et jonctions sont listés mais jamais suivis ; les **liens physiques** ne comptent qu'une fois (toujours sous le même nom, quel que soit l'ordre de lecture) ; les dossiers illisibles (accès refusé) sont signalés et comptés sans interrompre l'analyse |
| **Vues** | carte proportionnelle (treemap) avec fil d'Ariane, 100 plus gros fichiers, répartition par extension, répartition par ancienneté, évolution |
| **Suivi entre deux analyses** | la dernière analyse est rechargée au lancement (dossiers seulement) ; à l'analyse suivante du même lecteur ou dossier (ou d'un dossier qui le contient), les dossiers qui ont **grossi**, **diminué** ou sont **nouveaux** sont signalés avec l'écart |
| **Tableau** | tri par colonne (clic sur l'en-tête), colonnes déplaçables (glisser l'en-tête ; clic droit pour rétablir l'ordre), filtre texte sur toute l'analyse |
| **Actions** (clic droit ou panneau de détails) | ouvrir dans l'Explorateur, copier le chemin ou la ligne, propriétés Windows, analyser de nouveau un dossier, afficher dans la carte, **envoyer à la corbeille** (toujours avec confirmation, jamais de suppression définitive) |
| **Export** | CSV (séparateur `;`, compatible Excel, tailles en octets) ou copie collable dans Excel |
| **Clavier** | F5 lancer · Entrée (dans le champ Dossier) lancer · Échap arrêter, fermer le menu ou désélectionner · ↑/↓ changer de ligne · ←/→ replier / déplier · Ctrl+C copier le chemin · Ctrl+Maj+C copier la ligne · Suppr envoyer à la corbeille (avec confirmation) |
| **Confort** | thème clair/sombre (celui de Windows au premier lancement), réglages mémorisés, fenêtre sans barre de titre qui retrouve sa taille, sa position et son état agrandi |

Les réglages et la dernière analyse sont enregistrés dans `%APPDATA%\DiskScan\data` (`app.ron`, `last_scan.tsv`).

Interface : design « NetScan · egui » repris tel quel (tokens de couleurs, typographie Segoe UI / Consolas, icônes Lucide).

## Utilisation

Double-cliquer sur `DiskScan.exe`, ou en ligne de commande :

```
DiskScan.exe "C:\Users" --start
```

(`--start` lance l'analyse immédiatement ; pratique dans un raccourci.)

Sans droits administrateur, quelques dossiers système restent illisibles : ils sont signalés « accès refusé » et leur contenu n'est pas compté.

## Compilation

Prérequis : [Rust](https://rustup.rs) + Visual Studio Build Tools (charge de travail C++).

```
cargo build --release
```

L'exécutable est produit dans `target\release\DiskScan.exe`.

> Si le dossier du projet a un chemin très long (> 260 caractères avec `target\...`),
> l'éditeur de liens échoue (LNK1104). Il suffit alors de définir un dossier de sortie court :
> `set CARGO_TARGET_DIR=C:\build\diskscan`

Tests : `cargo test` (agrégation des tailles, liens physiques, tri, formats, comparaison, sauvegarde, carte proportionnelle, analyse d'un petit dossier réel créé pour l'occasion). `set DISKSCAN_TEST_DIR=C:\Users` puis `cargo test -- --ignored --nocapture` analyse un vrai dossier et affiche la durée et le détail des deux premiers niveaux.

En build de débogage, la variable `DISKSCAN_SCRIPT` rejoue des clics et touches pour tester l'interface sans souris
(ex. `2:click:485,170;3:shot:carte.bmp;4:quit`, voir `src/demo.rs`). Les coordonnées sont en points depuis le coin
haut-gauche de la fenêtre ; `shot:` enregistre une capture de la fenêtre et `quit` la ferme. Ce code n'est pas compilé en release.

## Structure

- `src/scanner.rs` — moteur : parcours du disque par un pool de threads (API Windows `GetFileInformationByHandleEx`, un appel par bloc d'entrées), annulation
- `src/tree.rs` — arborescence en mémoire : agrégation des tailles, liens physiques, tri, plus gros fichiers, extensions, ancienneté, comparaison de deux analyses
- `src/treemap.rs` — carte proportionnelle (découpage « squarified »)
- `src/history.rs` — dernière analyse enregistrée sur disque (dossiers seulement)
- `src/format.rs` — tailles, pourcentages, dates et nombres à la française, CSV
- `src/app.rs` — état de l'application, analyse en cours, panneau de paramètres, barre d'état, boutons de fenêtre
- `src/views.rs` — zone principale : statistiques, onglets, arborescence, carte, tableaux, détails, menu contextuel, confirmation de mise à la corbeille
- `src/win.rs` — lecteurs, Explorateur, propriétés, corbeille, thème de Windows, présence d'un écran
- `src/widgets.rs` — composants du design (boutons, champs, interrupteurs, curseurs, badges, éléments de menu, logo)
- `src/theme.rs` — tokens du design (palettes claire/sombre), polices, style egui
- `src/icons.rs` — icônes Lucide en SVG, teintées à l'affichage
- `src/icon.rs` + `build.rs` — logo dessiné par le code et intégré à l'exe
- `src/demo.rs` — rejeu de clics et captures pour les tests (débogage uniquement)
