# Garanties de persistance du state store

Ce document décrit les garanties actuellement fournies par `ModuleStateStore`. Il distingue la récupération logique de la durabilité face à une coupure de courant : ces propriétés ne sont pas interchangeables.

## Protocole de sauvegarde

1. L'état est encodé et refusé s'il dépasse 4 Mio.
2. Un fichier temporaire est créé dans le même répertoire que le fichier principal, écrit intégralement puis synchronisé avec `File::sync_all`.
3. Le fichier principal existant est déplacé vers une sauvegarde avant l'installation du temporaire.
4. Si l'installation échoue, le code tente de restaurer le fichier principal. Si cette restauration échoue également, la sauvegarde est conservée pour permettre une récupération ultérieure.
5. Une erreur de synchronisation du répertoire après remplacement est renvoyée à l'appelant. À ce stade, le nouveau principal peut déjà être visible; une erreur ne signifie donc pas que l'opération n'a rien modifié.

Les écritures concurrentes sur un même chemin doivent être sérialisées par l'application hôte.

## Garanties par plateforme

- **Unix (dont Linux et macOS)** : après le remplacement, le code ouvre le répertoire parent et appelle `sync_all`. Les erreurs de cette opération sont propagées. Cela renforce la persistance de l'entrée de répertoire, sans garantir à lui seul le comportement de tous les systèmes de fichiers ou de tous les périphériques.
- **Windows** : la synchronisation explicite du répertoire parent n'est pas disponible via `FlushFileBuffers` sur un handle de répertoire. À la place, chaque renommage de la transaction de sauvegarde (primaire vers backup, temporaire vers primaire, restauration) passe par un petit crate isolé qui appelle `MoveFileExW` avec `MOVEFILE_WRITE_THROUGH` et `MOVEFILE_REPLACE_EXISTING`. Le contenu temporaire est également synchronisé avant le remplacement. Cette stratégie doit encore être confirmée par les quality gates Windows; elle ne simule pas une coupure d'alimentation.
- **Autres plateformes non Unix** : le helper de synchronisation de répertoire reste sans effet; le renommage utilise `std::fs::rename`. Aucune garantie de durabilité supplémentaire n'est revendiquée pour ces cibles.

## Couverture et travaux restants

La suite de tests injecte les échecs de renommage et de synchronisation, vérifie la restauration du principal, la conservation d'une sauvegarde récupérable et le nettoyage des fichiers temporaires. Le workflow CI exécute les tests sur Linux, Windows et macOS; cette exécution vérifie le comportement fonctionnel sur ces plateformes, mais ne simule pas une coupure d'alimentation.

La tâche P0 reste ouverte tant qu'une stratégie explicite de synchronisation/récupération Windows et sa validation dans la matrice CI n'ont pas été décidées. Ne pas remplacer la synchronisation non prise en charge par un succès silencieux tout en affirmant une durabilité complète.

