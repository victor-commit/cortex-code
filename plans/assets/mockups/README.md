# Cortex Code — Galerie de mockups

Ces mockups sont des rendus terminal déterministes (données mock) qui couvrent **toutes les situations possibles** de l'UI Cortex Code. La galerie compte **42 écrans** répartis sur 5 catégories, à taille fixe **120×40**. Chaque écran est exporté en `.txt` et `.png`.

## Rendu

```bash
cd tools-throwaway/mockup-harness
cargo run --quiet --features png -- --all        # rend les 42 écrans
cargo run --quiet --features png -- --list       # liste par catégorie
cargo run --quiet --features png -- <name>       # un écran (ex. login)
cargo run --quiet --features png -- <category>   # une catégorie (ex. session)
```

La sortie est écrite dans `plans/assets/mockups/<category>/<screen>.{txt,png}`.

## orchestration (9)

| Écran | Description |
| --- | --- |
| `concurrency-metrics` | Débit des agents parallèles : workers actifs/en file, tokens/s, temps gagné |
| `intent-scorer-report` | Classement d'intention d'une requête, scores et barres de confiance |
| `mode-switch` | `/mode` : bascule entre plan mode et agent mode, carte de confirmation |
| `reviewer-output` | `/review` : verdict du relecteur (APPROVE/REJECT) et constats par fichier |
| `team-board` | Team Mode : membres, rôles, tâches assignées et statut |
| `tool-envelope` | Anatomie d'un appel d'outil : commande, arguments, cadre de sortie |
| `verification-ladder` | Échelle de vérification (RED→GREEN→SURFACE) avec barreaux pass/fail |
| `workflow-launch` | `/run` : lanceur de workflow avec sélection et paramètres |
| `workflow-monitor` | Suivi d'un workflow en cours : étapes, progression, timing par étape |

## session (11)

| Écran | Description |
| --- | --- |
| `home` | Écran d'accueil : sessions récentes et actions rapides |
| `onboarding` | Première exécution / écran de bienvenue |
| `chat-turn` | Un tour Q/R terminé : prompt utilisateur + réponse de l'agent avec encart "In short" |
| `streaming` | Réponse en cours d'écriture (streaming) avec indicateur live |
| `reasoning` | Tour affichant la trace de raisonnement du modèle dépliée |
| `tool-running` | Appel d'outil en cours avec progression |
| `tool-success` | Appel d'outil réussi avec sortie capturée |
| `tool-failed` | Appel d'outil échoué (run_shell exit 1) avec erreur et proposition de correction |
| `diff-edit` | Édition de code proposée par l'agent, carte de diff en attente d'application |
| `shell` | Cartes d'outil shell : une exit 0, une bloquée en plan mode |
| `compact` | Tour compacté / conversation résumée après `/compact` |

## input (12)

| Écran | Description |
| --- | --- |
| `slash-menu` | Menu des commandes slash (builtins) |
| `command-palette` | Palette de commandes (Ctrl+P) : actions floues et raccourcis |
| `model-picker` | Overlay de sélection du modèle |
| `agent-picker` | Overlay de sélection de l'agent actif |
| `skills-picker` | Overlay de sélection des skills à charger |
| `session-picker` | Overlay pour changer / reprendre une session |
| `permission-prompt` | Modale de permission pour une commande shell (allow once/always/deny) |
| `diff-viewer` | `/diff` : visionneuse de diff plein fichier, navigation entre fichiers |
| `status` | `/status` : état de session (branche, modèle, ctx, coût) |
| `timeline` | `/timeline` : chronologie des événements de la session |
| `mcp` | `/mcp` : serveurs MCP connectés et outils |
| `help` | `/help` : référence des raccourcis et commandes |

## auth (4)

| Écran | Description |
| --- | --- |
| `login` | Choix de la méthode de connexion (navigateur / clé API / fournisseur) |
| `connect-provider` | Catalogue des fournisseurs |
| `connect-apikey` | Flux de connexion par collage d'une clé API fournisseur |
| `connect-oauth` | Flux de connexion OAuth (navigateur) |

## states (6)

| Écran | Description |
| --- | --- |
| `empty` | État vide : aucune session |
| `error-network` | Erreur réseau injoignable avec détail retry/backoff |
| `error-context` | Erreur de fenêtre de contexte en plein refactor |
| `rate-limit` | Limite de débit atteinte : barres d'usage par minute et reprise auto |
| `interrupt` | Tour interrompu par l'utilisateur (Esc) en pleine génération |
| `toasts` | Notifications toast empilées (OK / INFO / BAD) |

## Conformité design

Tous les écrans respectent `cortex-code-design` : glyphes fonctionnels seuls (● › → ─ █ ░ · —), aucun glyphe décoratif, erreurs/statuts en anglais clair, barre du bas sans fond rempli.
