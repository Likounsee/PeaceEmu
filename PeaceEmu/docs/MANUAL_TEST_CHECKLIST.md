# PeaceEmu — checklist de validation manuelle desktop

Cette checklist valide les parcours génériques de l'application, pas la compatibilité d'un système. Le cœur Game Boy actuel est incomplet et ne doit pas être présenté comme un émulateur de jeux jouable.

## Préparation

- [ ] Construire l'application depuis un checkout propre avec la version stable de Rust.
- [ ] Démarrer PeaceEmu sans fichier de préférences existant.
- [ ] Confirmer que la fenêtre s'ouvre, que les pages principales sont accessibles et que la fermeture ne panique pas.
- [ ] Redimensionner la fenêtre jusqu'à sa taille minimale puis l'agrandir; vérifier navigation, formulaires et boutons sans débordement majeur.

## Bibliothèque et lancement

- [ ] Vérifier l'état vide de la bibliothèque et ses instructions d'import.
- [ ] Importer un fichier de test autorisé, puis vérifier titre/chemin, recherche, filtre, tri et sélection.
- [ ] Vérifier que l'extension est présentée comme un indice, que la détection par octets est distincte, et que les états en attente, détecté, ambigu et sans correspondance sont compréhensibles.
- [ ] Après un lancement accepté puis un lancement rejeté, vérifier que le dernier résultat de validation est affiché correctement.
- [ ] Importer plusieurs fichiers en mélangeant fichiers acceptés et refusés; vérifier que les erreurs sont explicites et que les imports valides restent accessibles.
- [ ] Tenter de lancer un fichier absent, vide, trop volumineux, de format inconnu ou malformé; vérifier qu'aucune session fantôme n'est créée et que la sélection existante est préservée.
- [ ] Vérifier les erreurs d'ouverture/lecture avec un fichier supprimé après import et, si l'OS le permet, avec un fichier dont les permissions interdisent la lecture.
- [ ] Vérifier le lancement réussi uniquement avec un contenu de test compatible et un cœur dont le périmètre est réellement implémenté. Ne pas compter un cœur factice comme preuve de compatibilité console.
- [ ] Supprimer une entrée et vérifier que la sélection et l'état de lancement restent cohérents.

## Sessions, entrées et médias

- [ ] Créer plusieurs sessions de test, sélectionner chacune et vérifier que la session active affichée est la bonne.
- [ ] Mettre en pause puis reprendre; vérifier que l'exécution ne progresse pas pendant la pause.
- [ ] Arrêter, réinitialiser et fermer une session; vérifier que les diagnostics, entrées et médias obsolètes sont nettoyés.
- [ ] Ouvrir les diagnostics de chaque session et vérifier identifiant, sélection active, état, capacités média et dernières erreurs propres à cette session.
- [ ] Fermer une session non active puis toutes les sessions; vérifier que la sélection active ne pointe jamais vers une session fermée.
- [ ] Changer de page pendant que des contrôles sont maintenus, puis fermer ou arrêter une session; vérifier la libération des entrées.
- [ ] Maintenir un contrôle dans une session, tenter de lancer un autre contenu, puis vérifier que les entrées de l'ancienne session sont libérées avant le changement; si la libération échoue, le lancement doit être annulé et la session active ne doit pas changer.
- [ ] Vérifier que l'absence de trame/audio est indiquée honnêtement si le cœur ne fournit pas ces médias; vérifier que l'absence de périphérique audio ne fait pas planter l'application.

## Préférences, erreurs et redémarrage

- [ ] Modifier thème, échelle UI et couleur d'accent, enregistrer, fermer et redémarrer; vérifier la persistance.
- [ ] Vérifier qu'une erreur d'écriture des préférences affiche un échec et non un succès.
- [ ] Vérifier qu'un fichier de préférences ou de récupération dépassant 4 Mio est rejeté avec une erreur explicite au démarrage.
- [ ] Simuler une préférence active absente ou corrompue avec un `desktop.conf.bak` valide; vérifier la récupération des valeurs, puis confirmer qu'une sauvegarde ultérieure ne détruit pas la dernière copie valide avant la promotion du nouveau fichier.
- [ ] Supprimer/déplacer un fichier de bibliothèque importé, redémarrer et vérifier qu'il est signalé comme indisponible sans lancement possible.
- [ ] Provoquer une erreur utilisateur récupérable; vérifier qu'un message exploitable est présenté et que l'application continue de fonctionner.
- [ ] Vérifier l'écran de paramètres et les contrôles au clavier, y compris le focus après navigation.

## Preuves à conserver

- [ ] Noter OS, version du commit et version Rust utilisée.
- [ ] Joindre captures d'écran des états vide, erreur, bibliothèque, sessions et paramètres.
- [ ] Consigner les échecs, les étapes pour les reproduire et les éventuels journaux pertinents sans données personnelles.
- [ ] Exécuter et archiver les résultats de `cargo fmt --all -- --check`, `cargo test --workspace --locked`, `cargo clippy --workspace --all-targets --locked -- -D warnings` et `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --locked`.
- [ ] Vérifier séparément les résultats GitHub Actions du commit exact sur Linux, macOS et Windows.

Une case ne doit être cochée qu'après observation réelle. Cette checklist documente des vérifications à effectuer; sa présence dans le dépôt ne signifie pas qu'elles ont déjà été réalisées.


## Updater de l'application

- [ ] Vérifier le manifeste stable officiel et confirmer que l'absence d'artefact pour la plateforme est indiquée clairement, sans proposer un téléchargement inexistant.
- [ ] Vérifier que la version publiée égale ou inférieure à la version locale n'est pas présentée comme une mise à jour disponible.
- [ ] Avec un artefact de test publié, vérifier la progression et le résultat du téléchargement, puis confirmer que le fichier finalisé correspond à la taille et au SHA-256 annoncés.
- [ ] Vérifier que les URL de téléchargement et de notes de version pointent vers les releases officielles d'PeaceEmu; les manifestes invalides doivent échouer sans créer de fichier de staging.
- [ ] Vérifier qu'un échec réseau, une taille incorrecte, un SHA-256 incorrect ou une déconnexion du worker laisse l'interface récupérable et ne supprime aucun fichier préexistant.
- [ ] Confirmer qu'un artefact seulement stagé n'est ni exécuté ni installé automatiquement.

## Windows installer acceptance

- [ ] Run `.github/workflows/windows-installer.yml` and retain the installer artifact and commit SHA.
- [ ] Verify the installer version and SHA-256 sidecar file.
- [ ] Install on a clean Windows VM without Rust, Git, the repository, or build tools.
- [ ] Launch from Start Menu and optional desktop shortcut; close and restart.
- [ ] Verify preferences and imported content paths persist across restart.
- [ ] Upgrade over a previous version and verify existing settings remain intact.
- [ ] Uninstall while choosing to preserve user data; verify settings and ROMs remain.
- [ ] Repeat uninstall choosing removal of the explicitly documented settings/log/cache files; verify unrelated files and ROMs remain.
- [ ] Record Windows version, commit SHA, checksum, and observed results.
