# Cortex Code — Audit UX / UI des maquettes (45 écrans)

Audit visuel des 45 PNG du harness (`plans/assets/mockups/`), du point de vue d'un
utilisateur réel qui va utiliser le logiciel. Rendus 120×40. Sévérité : **[HIGH]** =
gêne l'usage / trompe l'utilisateur · **[MED]** = friction notable · **[LOW]** = polish.

> Note de contexte : le grand vide en haut du canvas (contenu ancré en bas) est un
> choix assumé de REPL de chat — non compté comme défaut.

---

## Résumé exécutif

Le jeu d'écrans est **cohérent et professionnel** : barre de statut partagée
(`path · main · model · ctx% · cost · ● live`), langage de points de statut
vert/ambre/rouge/creux, barres de raccourcis en pied de page. Les écrans les plus forts :
`diff-edit`, `rich-response`, `error-context`, `error-network`, `rate-limit`, `onboarding`,
`mcp`, `todo-list`.

Les gains les plus rentables sont **systémiques** (voir ci-dessous) : ils touchent
plusieurs écrans à la fois.

### Thèmes transverses prioritaires

1. **[HIGH] Rouge surchargé.** Le rouge signifie *suppression* (diff), *échec* (mcp,
   tool-failed) **et** *sélection* (`model-picker` : le `ctx` de la ligne sélectionnée
   est en rouge). Réserver le rouge à erreur/suppression ; changer la surbrillance de
   sélection pour un accent neutre.
2. **[HIGH] Statut par couleur seule.** Points vert/ambre/rouge sans glyphe
   (`shell`, `tool-running`, `tool-success`, actor dots de `timeline`). Ajouter un glyphe
   redondant (✓ / ● / ○ / ✗) — `todo-list` le fait déjà : en faire le standard.
3. **[HIGH] Bandes de statut redondantes.** Les écrans « working » répètent le même
   message interrupt/streaming dans la Working band **et** une ligne séparée **et** le
   footer (`shell`, `streaming`, `tool-running`, `tool-task`). Une seule source canonique.
4. **[HIGH] Format de temps incohérent.** `tool-task` affiche `1m04s` sur la ligne de
   tâche et `64s` dans la bande — même valeur, deux formats. Unifier partout.
5. **[HIGH] Compteurs statiques qui paraissent cassés.** `connect-oauth` « Code expires
   in 15:00 », `rate-limit` « retry in 42s » : figés, ils lisent comme des placeholders.
   En faire des comptes à rebours vivants.
6. **[MED] Tables d'état dupliquées.** `concurrency-metrics`, `intent-scorer-report`,
   `verification-ladder` listent les mêmes items (waves / D1–D5 / L0–L4) deux fois à des
   granularités différentes. Garder une seule table faisant autorité + expansion.
7. **[MED] L'échec/l'action critique a le même poids que les lignes de routine.**
   `team-board` (BLOCKED noyé parmi CLOSED), `workflow-monitor` (`failed 1`),
   `reviewer-output`, `toasts` (BAD). Escalader visuellement (barre/tint/tri par sévérité).
8. **[MED] Affordance apply/persist ambiguë dans les pickers toggle.** `skills-picker`
   n'a pas de touche « confirmer » — esc annule-t-il les bascules ? Le clarifier.
9. **[MED] Affordance de sélection faible.** Les pickers reposent sur gras + `›` sans
   barre de surbrillance (`agent-picker`, `model-picker`). Ajouter un fond de ligne.
10. **[MED] Convention de couleur inline non unifiée.** Identifiants vs codes HTTP vs
    liens partagent des accents sans règle claire (`chat-turn`, `reasoning`, `compact`).
11. **[HIGH] Tout état d'échec doit porter une affordance clavier de récupération.**
    Les erreurs bloquantes le font bien (context/network/rate-limit), sauf le toast
    **BAD** de `toasts` (« check permissions » sans hotkey).

---

## auth/

### connect-apikey — Ajouter une clé API
L'écran le plus fort du lot. Clé masquée `sk-ant-••••1234`, validation verte
« Format looks valid — 108 characters », rassurance « stored locally in auth.json ».
- **[MED]** Clarifier si `Provider: Anthropic` est éditable ici ou verrouillé ; si
  verrouillé, l'estomper.
- **[LOW]** Reformuler « paste insert key » → `paste · insert key from clipboard`.
- **[LOW]** Ajouter l'attendu post-sauvegarde (« retour au chat une fois vérifié »).

### connect-oauth — Autoriser l'appareil
Flow device clair, code `WDJB-MJHT` proéminent, étapes numérotées.
- **[HIGH]** `Code expires in 15:00` figé = paraît cassé → compte à rebours vivant.
- **[MED]** Lier visuellement `o open browser` à l'URL de l'étape 1 (`[o] open` en ligne).
- **[LOW]** Ajouter une affordance de copie pour le code à usage unique.

### connect-provider — Connecter un fournisseur
Liste scannable (nom, description, statut de connexion), curseur `›` sur Anthropic.
- **[HIGH]** Logique de statut confuse : OpenRouter/OpenAI « connected » (vert) mais
  Anthropic (fournisseur actif, footer `enter connect`) sans badge. Distinguer
  « actif » vs simplement « connecté ».
- **[MED]** Indicateurs non-couleur (`✓ connected` / `○ not connected`) pour daltoniens.
- **[LOW]** Contextualiser `d disconnect` (visible seulement sur lignes connectées).

### login — Se connecter / Bienvenue
Trois chemins d'auth bien libellés avec descriptions ; « stored locally in auth.json
(0600) » = bon signal de confiance.
- **[MED]** « Welcome back » contredit un tout premier lancement → conditionner le texte.
- **[MED]** Clarifier la relation entre les 3 options (les 2 premières = chemins rapides ;
  « Connect a provider » = catalogue complet). Grouper / séparateur.
- **[LOW]** Utiliser le haut de canvas pour l'identité (logo/version).

---

## input/

### agent-picker — Sélectionner un agent
Liste d'agents nommés + descriptions, marqueur `● current` sur Build, curseur `›` sur Plan.
- **[MED]** Ajouter une barre de surbrillance sur la ligne sélectionnée (curseur ≠ actif).
- **[LOW]** Ajouter une ligne `filter` (cohérence avec model-picker).
- **[LOW]** Montrer l'effet de la sélection (« grants: read-only ») pour renforcer l'intention.

### command-palette — Palette de commandes
Filtre, commandes + raccourcis à droite, compteur « 5 of 28 actions ».
- **[HIGH]** La 2ᵉ colonne cryptique (`H / L / G / F / 8`) doit fusionner visuellement
  avec le chord (`Ctrl+G H`) pour se lire comme un seul raccourci.
- **[MED]** Affordance de sous-menu (`›`/`▸`) sur « Palette submenu: queued prompts ».
- **[LOW]** Désambiguïser stash/recall/restore (sémantiques qui se chevauchent).

### diff-viewer — Revue de diff (fichier 2/6)
Excellent : progression `file 2 of 6 · +9 −4`, aperçu fichier suivant, footer complet.
- **[HIGH]** Ajouter des actions au niveau hunk (stage/skip) + un « stage all / finish
  review » — le staging par fichier seul est limitant sur 6 fichiers.
- **[MED]** Afficher un compteur de staging courant (« staged 1/6 »).
- **[LOW]** Dédupliquer la ligne `summary +9 −4` (déjà dans l'en-tête).

### help — Raccourcis clavier
Très bon : groupé par scope (Session/Views/Composer), concept de leader `ctrl+g` expliqué.
- **[MED]** Pointeur vers la palette complète (« Ctrl+P pour les 28 actions ») — indiquer
  que c'est un aide-mémoire curé.
- **[LOW]** Filtre inline pour sauter à un raccourci.
- **[LOW]** Astuce « Getting started » en haut pour le premier lancement.

### mcp — Serveurs MCP
Fort : résumé `4 configured · 3 connected · 1 failed`, ligne échouée rouge avec note
d'erreur inline `spawn error: command not found`, footer `r reconnect`.
- **[MED]** Rendre la note d'erreur actionnable (« vérifier le binaire `playwright` dans
  le PATH » / commande copiable) plutôt qu'un spawn error brut.
- **[LOW]** Clarifier ce que `enter details` révèle vs la note inline.
- **[LOW]** Horodatage de la dernière tentative de connexion par ligne.

### model-picker — Sélectionner un modèle
Filtre, colonne fournisseur, fenêtre de contexte, marqueur `● current`.
- **[HIGH]** Le `ctx` de la ligne sélectionnée est en **rouge** → lit comme une erreur.
  Passer à un accent neutre (le rouge casse le langage de couleur de l'app).
- **[MED]** Ajouter coût/token ou indice vitesse/qualité pour répondre à « quel modèle
  est le meilleur ? » (le ctx seul est un signal faible).
- **[LOW]** Grouper/labelliser par fournisseur si la liste grandit.

### permission-prompt — Permission requise
Très fort : en-tête, commande exacte `cargo test --all`, 3 choix codés couleur
(allow/allow-always/deny), « remember for this session » bien cadré.
- **[MED]** Après « Deny », montrer/confirmer si un champ « raison » apparaît (sinon
  retirer « and tell the agent why »).
- **[LOW]** Mettre en valeur le flag large `--all` (important pour une décision de perm).
- **[LOW]** Rappel de portée sur « Allow always » (par-commande vs par-session).

### session-picker — Reprendre une session
6 lignes (titre + temps relatif + nb messages), bascule bold/dim.
- **[MED]** Auto-sélectionner / badger la session correspondant à la requête
  (« yesterday ») au lieu de la plus récente par défaut.
- **[LOW]** Aligner temps et nb messages en colonnes fixes.
- **[LOW]** Actions de ligne légères (delete/rename/pin) avec hint footer.

### skills-picker — Sélectionner des skills
Bascules on/off explicites (vert/gris), `vision-opus` correctement sélectionné et « off ».
- **[MED]** Clarifier la persistance : pas de touche « confirmer » — esc annule-t-il ?
  Ajouter « changes apply immediately » ou `enter apply`.
- **[LOW]** Compteur vivant (« 5 of 6 enabled ») qui suit les bascules.
- **[LOW]** Distinguer « chargé cette session » vs « disponible mais off ».

### slash-menu — Menu slash
Le mieux documenté : chaque commande décrite, filtre `/`, footer clair.
- **[LOW]** Sélection par défaut sur le 1er item (`/help`) plutôt que `/models`.
- **[LOW]** Séparateurs de sections au-delà de ~10 items.
- **[LOW]** Raccourcis inline pour les commandes qui en ont.

### status — Statut de session
Dense et bien organisé : table 2 colonnes, barre de contexte visuelle `47% · 94k/200k`.
- **[MED]** Séparateur vertical entre les deux paires FIELD/VALUE (deux en-têtes « FIELD »
  prêtent à confusion).
- **[LOW]** Barre de contexte qui change de couleur (vert→ambre→rouge) vers 100 %.
- **[LOW]** Lier `c to compact` visuellement à la ligne de contexte concernée.

### timeline — Chronologie de session
Table WHEN/ACTOR/SUMMARY, points d'acteur codés couleur, footer `f fork`.
- **[MED]** Légende pour les couleurs de points (bleu « Build » = actif/courant ?
  vs vert = fait, orange = Review) — actuellement non documenté.
- **[MED]** Rendre les tours expansibles (`enter` → voir les 4 findings / le diff).
- **[LOW]** Clarifier le résultat de `f fork` (« → nouvelle session depuis ce tour »).

---

## orchestration/

### concurrency-metrics — Métriques de concurrence
Moniteur de conflit/quarantaine ; bandeau haut (`w3 hot 16.7%`, `quarantine 1`) = alerte.
- **[HIGH]** Donner un vrai raccourci à l'action de résolution (« reattach → re-post
  READY ») et la remonter au footer — la quarantaine est tout l'enjeu de l'écran.
- **[MED]** Lier visuellement le `feat-api-routes` en quarantaine à sa ligne de conflit
  wave-3 (couleur/soulignement partagés).
- **[LOW]** Convertir les 2 blocs de prose explicative en une légende compacte 2 lignes.

### intent-scorer-report — Rapport du scoreur d'intention
Fort : `total 8/10`, 3 tables (scores / verdict / evidence).
- **[MED]** Marquer les dimensions « gated » (D1/D3/D4) inline dans la table des scores
  (glyphe cadenas) pour rendre la logique pass/fail auto-évidente.
- **[MED]** Fusionner/replier les D1–D5 dupliqués entre table des scores et evidence.
- **[LOW]** Remplacer le `—` de la colonne GATE par `n/a` explicite.

### mode-switch — Changer de mode
Excellent : delta `+2 grants`, matrice de permissions, légende `Δ` pour outils débloqués.
- **[MED]** Renommer le marqueur `● active` de la ligne Agent en `● selected`/`requested`
  (« active » laisse croire que le switch a déjà eu lieu ; le courant est Edit).
- **[LOW]** Rapprocher l'explication `Δ = unlocked` de l'en-tête de matrice.
- **[LOW]** Teinter les 2 lignes de nouveaux grants (plan_note, spawn_subagent).

### reviewer-output — Sortie du reviewer
`CHANGES REQUESTED` rouge en haut, 2 sections reviewer avec pass/fail par check.
- **[HIGH]** Agréger les 4 findings en une checklist actionnable près de la ligne de gate
  (« re-post with: Q3, Q4, Q8, Q9 ») au lieu de notes `↳` éparpillées.
- **[MED]** Aligner les codes de référence (`server/util.rs:88`) en vraie colonne.
- **[LOW]** Confirmer / estomper `a accept` tant que des findings bloquants existent.

### team-board — Tableau d'équipe (Kanban)
Bandeau de flux `OPEN→READY→CLAIMED→ACTIVE→SUBMITTED→CLOSED / BLOCKED` avec compteurs.
- **[MED]** Trier la table des tâches par étape du pipeline (miroir du bandeau de flux).
- **[MED]** Mettre en valeur BLOCKED (barre/ligne rouge) — la dép échouée doit ressortir
  des lignes CLOSED.
- **[LOW]** Marquer les lignes READY éligibles à `c claim`.

### tool-envelope — Contrat d'enveloppe d'outil
Doc du contrat de réponse + journal d'appels ; 2 exemples JSON côte à côte (ok vs refused).
- **[MED]** Rendre les compteurs du haut (`ok 41 · error 2 · refused 3`) filtrants
  (afficher seulement error/refused).
- **[LOW]** Remonter le contraste « error = a tourné & échoué · refused = jamais lancé »
  en légende inline près de la colonne statut.
- **[LOW]** Équilibrer la hauteur des 2 blocs JSON (légendes désalignées).

### verification-ladder — Échelle de vérification
Très clair : 5 niveaux L0–L4, bandeau de progression, concept masqué-vs-visible bien expliqué.
- **[MED]** Replier la liste L0–L4 dupliquée entre la table `ladder` du haut et le bloc
  `submission` — une seule table faisant autorité.
- **[MED]** Utiliser une couleur non-rouge pour « masked » (se confond avec « fail »).
- **[LOW]** Rendre la continuation L3 explicite (« 3 questions restantes → enter »).

### workflow-launch — Lancement de workflow
Écran de confirmation ; `preflight OK — 11/11 artifacts present` (vert) rassurant ;
graphe de dépendances = point fort.
- **[MED]** Lier visuellement le calendrier de waves au graphe (colorier les nœuds par wave).
- **[LOW]** Estomper la note `abort form: MISSING…` (détail edge-case sur un écran de confirm).
- **[LOW]** Mettre en valeur `N=3 parallelism` et `budget $2.00` (leviers éditables).

### workflow-monitor — Moniteur de workflow
Le plus dense mais tient : bandeau de phases A–F, table de verdicts de features 6 colonnes.
- **[MED]** Escalader la feature en échec (`feat-audit-log`) : barre rouge / épinglée en
  haut, pour que `failed 1` ait un point d'ancrage visuel.
- **[MED]** Montrer plus fortement la position courante dans la boucle D1–D8 (highlight D4).
- **[LOW]** `f filter` par défaut sur non-vérifié pour trier partial/failed d'abord.

---

## session/

### chat-turn — Tour de conversation
Baseline solide : prompt `›` + timestamp, réponse « Cortex », TL;DR « In short » apprécié.
- **[MED]** Établir une convention claire de couleur inline (identifiant vs statut HTTP
  vs lien) et l'appliquer partout.
- **[LOW]** Relever légèrement le contraste du gris du corps de texte (confort de lecture).

### compact — Conversation compactée
Bon capital confiance : « nothing was lost », métriques `18 turns collapsed · 31,400
tokens reclaimed · context now 12%`.
- **[MED]** Clarifier la cible de `ctrl+o expand` (« expand summary » vs « full transcript »).
- **[LOW]** Séparateur subtil entre les puces de recap et la ligne de métriques.

### diff-edit — Édition proposée
Le plus fort écran de décision : chemin, `+2 −1 · 1 hunk`, diff rouge/vert, rationale.
- **[MED]** Désambiguïser la ligne d'input : montrer un prompt de touche `[y/n/edit]`
  plutôt qu'un champ texte qui accepte aussi la frappe.
- **[LOW]** Montrer old→new inline dans la rationale (`None → Some(1)`).

### home — Accueil
Splash (mascotte, version, « Welcome back, root », config) + premier échange en direct.
- **[MED]** Dédupliquer les deux lignes de hints (celle du panneau d'accueil et le footer
  se recoupent).
- **[LOW]** Séparateur/règle faible entre le bloc d'accueil et le premier tour.

### onboarding — Configuration guidée
Meilleure clarté d'onboarding : assistant 3 étapes, « step 2 of 3 », états done/current/pending.
- **[LOW]** Les labels de statut à droite peuvent sembler déconnectés en 120 de large
  (leader léger / colonne plus serrée).
- **[LOW]** Distinguer plus nettement l'étape courante des autres (toutes montrent des valeurs).

### reasoning — Raisonnement visible
« Thinking 8s » (chaîne de pensée) grisé puis réponse claire — bonne transparence.
- **[MED]** Ajouter une affordance « plus de contenu » (`▾ more`/fondu) : la liste de
  réponse est visiblement tronquée par la barre d'input, sans indicateur de scroll.
- **[LOW]** Remonter un peu le contraste du gris de la trace « Thinking » (proche du seuil).

### rich-response — Réponse riche
Impressionnant : table réelle, badges de verdict, bloc de code, reco en gras.
- **[MED]** Réduire la redondance : la colonne « Verdict » de la table et les puces
  `[best fit]/[caution]/[avoid]` disent la même chose.
- **[LOW]** Affordance de copie du bloc de code (`y copy`).
- **[LOW]** Ajouter un symbole (●/▲/■) aux verdicts rouge/jaune/vert (daltoniens).

### shell — Commandes shell (git status + cargo test)
Deux blocs d'outil empilés ; point ambre + `running · 4s`, curseur clignotant.
- **[MED]** Dédupliquer les hints d'interruption (ligne running + bande + footer = 3 fois).
- **[MED]** Ajouter un glyphe (✓/●) à côté des points (pas couleur seule).
- **[LOW]** Aligner `exit 0 · 0.2s` en colonne de statut cohérente.

### streaming — Réponse en cours + Working band
La bande chaude « ✳ Working · writing the summary · 6s · esc to interrupt » lit clairement
(meilleure implémentation du lot).
- **[HIGH]** Replier la ligne redondante « streaming… press esc to interrupt » dans la
  Working band — une seule ligne de statut.
- **[LOW]** Éviter les doublons esc/interrupt entre bande et footer.

### todo-list — Plan avec progression
Excellent : checklist ✓/▶/○, résumé `3 of 5 done · 1 in progress · 1 pending`, bande
nommant le fichier actif. Les glyphes distincts (pas couleur seule) = modèle à généraliser.
- **[MED]** Emphase visuelle sur la ligne ▶ courante (gras/fond) pour que l'œil y atterrisse.
- **[LOW]** Fusionner la bande générique « working through the plan… » dans la Working band.

### tool-failed — Échec d'outil
Échec clair : point rouge + `failed · exit 1`, explication + `r retry`.
- **[MED]** Ajouter un raccourci explicite « apply fix » (`y apply`) puisque Cortex propose
  une action concrète.
- **[LOW]** Bordure gauche rouge subtile sur le bloc échoué.

### tool-running — Outil en cours (grep_code)
`running · 128 files · 3 matches so far` + bande `grep_code · 128 files scanned · 4s`.
- **[MED]** Consolider les métriques : la ligne de tâche possède « matches », la bande
  possède elapsed/scanned — sans répéter le nombre de fichiers.
- **[LOW]** Indicateur de progression léger (fraction) si le total est connu.

### tool-success — Résultats d'outil (grep)
Point vert, résultats structurés `3 matches across 221 files · 0.4s`, hits file:line.
Meilleur écran de résultats.
- **[LOW]** Rendre les hits file:line visiblement actionnables (« ↵ open »).
- **[LOW]** Garder libellés/unités cohérents entre états (221 vs 128 fichiers).

### tool-task — Délégation à un subagent
Un subagent `done · 3m12s · 6 findings` (vert), un autre en cours `2 of 3 files · 1m04s`,
bande `subagent · adding rate-limit tests · 64s`.
- **[HIGH]** Incohérence de format de temps : `1m04s` sur la ligne de tâche vs `64s` dans
  la bande (même valeur) — unifier.
- **[MED]** Rendre `6 findings` découvrable/expansible (`ctrl+o`).
- **[LOW]** Réduire la redondance entre la ligne « 2 of 3 files » et le label de la bande.

---

## states/

### empty — Aucune session
État vide manuel : « No sessions yet », CTA « Press n to start » (`n` mis en valeur).
- **[LOW]** Fusionner les deux lignes de sous-texte (redondantes).
- **[LOW]** Relever un peu le contraste du gris secondaire.

### error-context — Limite de contexte atteinte
Best-in-class : cause immédiate, barre 99 %, ventilation « Where the tokens are »
(history/system/tools/turn), `/compact` et `/new` avec résultats concrets.
- **[MED]** Différencier la barre quasi pleine (dégradé rouge/ambre) pour que « 99 % »
  se lise urgent (l'écran est presque monochrome-orange).
- **[LOW]** Colorer le chiffre « ~150k tokens » récupéré (rendre `/compact` le choix évident).
- **[LOW]** La barre pleine libellée « full » double le « 99 % ».

### error-network — Réseau injoignable
Très fort et rassurant : « your work saved · nothing was lost », table diagnostique
(endpoint/model/attempts/reason), `r retry`, `c copy error`.
- **[LOW]** Teinter « attempts 3 of 3 » (retries épuisés) plutôt que métadonnée neutre.
- **[LOW]** Clarifier enter (continuer) vs r (renvoyer).

### interrupt — Réponse interrompue
Clair : réponse partielle préservée + « kept · partial answer saved », point ambre (≠ erreur).
- **[LOW]** « retype » → « rephrase » pour coller à la description (« ask a different way »).
- **[LOW]** Ajouter une couture visuelle entre la réponse partielle et le bloc d'interruption.

### rate-limit — Limite de débit atteinte
Excellent ton : « resumes on its own », point vert `● Waiting`, barres « Usage this minute »
(requests 50/50 « at the cap », tokens 58k/80k 72 %), tier « Pro ».
- **[MED]** Rendre « retry in 42s » un compte à rebours vivant (un « 42s » figé vieillit vite).
- **[LOW]** Démoter visuellement `retry now` (peut aggraver — l'écran se répare seul).

### toasts — Notifications empilées + checkpoint
Point faible du lot : attention divisée entre 3 toasts (OK/INFO/BAD) et un bloc checkpoint
en bas ; le toast rouge **BAD** (échec réel) a le même poids qu'un INFO bénin.
- **[HIGH]** Donner au toast BAD une affordance clavier de récupération (« press s to retry
  settings save ») — un échec avec seulement « try again » en prose laisse l'utilisateur bloqué.
- **[MED]** Élever/persister le toast d'erreur pendant que OK/INFO s'auto-dismiss.
- **[MED]** Réduire le whiplash succès/échec (« Checkpoint saved » à côté de « Could not
  save settings ») — remonter l'échec près de l'action qui l'a causé.
- **[LOW]** Ordonner les toasts par sévérité (BAD en haut) plutôt que chronologiquement.
