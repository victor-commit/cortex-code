# Cortex Code — Analyse complète & Plan de mission

> **But de ce document.** Brief autoporteur à charger au démarrage d'une **nouvelle session
> de mission** dédiée à Cortex Code. Il contient l'analyse, l'architecture cible, le périmètre,
> la stratégie de test/validation, les jalons, le design du système de workers, l'infra et les
> décisions déjà prises. Rédigé par l'orchestrateur d'une autre session (mission « Cortex IDE »
> active, **non modifiée**).
>
> **Langue produit :** code/identifiants en anglais, docs utilisateur possibles en français
> (comme l'existant). Ce brief est en français.

---

## 0. Décisions verrouillées (par l'utilisateur)

1. **Greenfield** : reconstruire de zéro dans **`/projects/cortex-code`** (ce dossier).
2. **Objectif** : un logiciel **complet et entièrement fonctionnel, SANS AUCUN MOCK** côté
   utilisateur. Tout ce qui est exposé à l'utilisateur doit fonctionner réellement (vrais appels
   LLM, vrais outils, vrai MCP/LSP/plugins, vrai serveur). Les fixtures/cassettes déterministes
   restent autorisées **uniquement pour les tests**, jamais comme comportement par défaut livré.
3. **Références** : s'appuyer sur `/droid/cortex-code` (l'implémentation Rust existante, ~340k
   LOC, 41 crates) **et** sur opencode (sources dans `/tmp/opencode-sst`) + codex
   (`/tmp/codex/codex-rs`). `/droid/cortex-code` reste **lecture seule** (référence).
4. **Prompts/UX** : se **baser directement sur les prompts et l'UX d'opencode**, adaptés et
   **rebrandés** — **ne jamais** faire apparaître « opencode » (ni ses URLs/noms) dans le code,
   les prompts, ou les messages de commit. (Override explicite de la politique clean-room de
   l'ancien projet.)
5. **LLM** : **OpenRouter** comme fournisseur de référence pour les tests live (clé vérifiée, voir
   §7). L'architecture reste multi-providers.
6. **Session** : NE PAS lancer la mission dans la session courante (réservée à « Cortex IDE »).
   La mission Cortex Code démarrera dans une **nouvelle session**, en repartant de ce brief.

---

## 1. Vision produit (ce qu'attend l'utilisateur)

Cortex Code = un **agent de code pour le terminal**, type opencode/codex, **complet et utilisable
en production**. Un binaire Rust, trois surfaces :

- **TUI interactif** (`cortex`) : conversation streamée avec rendu markdown + coloration
  syntaxique, diffs lisibles, cartes d'appels d'outils (expand/collapse), commandes slash, mentions
  `@fichier` (fuzzy), mode shell `!`, palette de commandes, thèmes, keybinds, picker de sessions,
  recherche dans le chat, toasts, undo/redo.
- **Headless** (`cortex run`) : one-shot/continue, `--model/--agent/--session/--continue`, stdin,
  sortie `--json`/`--jsonl` stricte, pour scripts/CI.
- **Serveur local** (`cortex serve`) : API HTTP loopback + flux SSE `/event`, CRUD sessions,
  OpenAPI, contrôle TUI distant, partage local de session.

Extensibilité **réelle** : **MCP** (stdio + HTTP/SSE), **LSP** (diagnostics + navigation),
**formatters** post-édition, **agents/sous-agents** (`task`), **plugins/hooks**, **skills**.

Tout cela existe déjà partiellement dans `/droid/cortex-code` mais avec des zones
mock/orphelin/différées (voir §3 et `research/04`). **La mission consiste à reconstruire proprement
en rendant TOUT réel.**

---

## 2. Matériel de référence (déjà produit dans ce dépôt)

Tout est sous `/projects/cortex-code/research/` :

| Fichier | Contenu |
|---|---|
| `research/01-opencode-architecture.md` | Inventaire architecture+features d'opencode (V1 AI-SDK + V2 natif). **Blueprint principal.** Table « capability checklist ». |
| `research/02-opencode-prompts.md` | Système de prompts d'opencode : sélection par modèle, injections dynamiques, descriptions d'outils, agents, compaction/title. Notes de rebranding. |
| `research/opencode-prompts-raw/` | **Tous les prompts bruts d'opencode** copiés verbatim (session-prompt/, agent-prompt/, tool/, command/). Base à adapter+rebrander. |
| `research/03-codex-architecture.md` | Architecture Rust de codex : transport en couches, boucle agent tokio, apply_patch, MCP (rmcp), sandbox (seatbelt/landlock), crates à réutiliser. |
| `research/04-existing-cortex-code-inventory.md` | Inventaire crate-par-crate de `/droid/cortex-code` : ce qui est RÉEL vs MOCK/PARTIAL/ORPHAN/STUB/ABSENT, + liste des écarts « à rendre réel », + forces à conserver, + toolchain, + autorisation de port TUI. |

**À lire en premier dans la nouvelle session :** `04` (état de l'art interne), puis `01` (blueprint
opencode), puis `02`+raw (prompts), puis `03` (idées Rust).

---

## 3. Écarts « mock/orphelin/différé » à éliminer (exigence « tout réel »)

D'après `research/04`, voici ce qui n'est PAS pleinement réel dans l'existant et **doit l'être**
dans le greenfield :

1. **Transport live par défaut.** Les adaptateurs providers + webfetch sont du vrai code reqwest,
   mais le défaut livré est replay/cassette offline (`CORTEX_TRANSPORT=live` en opt-in). → **Le
   live doit être le chemin par défaut** dès qu'une clé est présente (kill-switch `CORTEX_NO_NET`
   conservé). Finir le câblage base-URL vertex/bedrock.
2. **Web search/research réel.** `WebResearchPolicy::default` désactivé, transport live « DEFERRED ».
   → brancher un vrai endpoint de recherche, activable par config.
3. **Exécution des plugins réellement fonctionnelle.** Aujourd'hui délégation à un runtime JS
   externe détecté (Bun/Node/Deno). → soit embarquer un moteur JS, soit formaliser et **garantir**
   le contrat runtime externe (détecté, testé, échec clair). Décision à prendre (§13).
4. **Replay réel et câblé** (la commande `cortex replay` est un stub d'affichage ; crate orpheline).
5. **Audit logging câblé** (types existent, aucun émetteur).
6. **Un seul runtime** : supprimer le squelette `cortex-runtime`/`-types` orphelin, ne garder que la
   vraie pipeline (équivalent `cortex-core`).
7. (Optionnel/parité) auth providers étendue (Azure AD, Vertex JWT, Bedrock SSO), MCP remote OAuth.
8. **Exclusions assumées** (à reconfirmer §13) : share hébergé, GitHub App, télémétrie, Windows,
   sandbox OS, browser/computer-use.

> Note « no mock » : la suite de tests peut garder cassettes/fixtures **pour le déterminisme**,
> mais la **validation de mission exige des preuves end-to-end via la vraie API OpenRouter** (§9).

---

## 4. Architecture cible (Rust greenfield)

Reprendre le **modèle V2 d'opencode** (protocoles natifs + runner durable + serveur HTTP/SSE +
SDK généré), en s'inspirant du **transport en couches de codex**, et en conservant les **forces de
`/droid/cortex-code`** (frontières de crates propres, redaction-first, permissions/trust,
checkpoints shadow-git, persistance SQLite, binaire unique 3 surfaces).

### 4.1 Couches

```
CLI / TUI / Server (surfaces)
        │
   Session Runner (boucle agent durable : MAX_STEPS, steer/queue, tool fibers, compaction)
        │
   ┌────┴───────────────────────────────────────────────┐
   │ Tools registry   Permissions/Trust   Agents/Subagents │
   │ MCP client       LSP client          Formatters       │
   └────┬───────────────────────────────────────────────┘
        │
   LLM layer (trait Provider) ── protocoles natifs : OpenAI Chat, OpenAI Responses,
        │                         Anthropic Messages, Gemini, OpenAI-compatible (→ OpenRouter)
   Transport (reqwest+rustls, SSE eventsource, WS optionnel) ── normalise un flux `LlmEvent`
        │
   Persistence (SQLite/sqlx ou rusqlite) · Config (TOML layered) · Event bus (typed) · Redaction
```

### 4.2 Découpage crates proposé (greenfield)

S'inspirer de l'existant mais **sans les orphelins**. Proposition (à affiner par l'orchestrateur de
la mission) :

- `cortex-protocol` (DTO/wire), `cortex-events` (bus typé), `cortex-config` (TOML layered + `{env}`/`{file}`),
  `cortex-redaction`, `cortex-errors`, `cortex-persistence` (SQLite + migrations).
- `cortex-llm` (trait Provider + protocoles natifs + transport SSE/WS + normalisation `LlmEvent`),
  `cortex-providers` (catalogue/models.dev + résolution + auth/credential store).
- `cortex-core` (session runner durable, prompts, compaction, history, context epoch).
- `cortex-tools` (read/write/edit/apply_patch/glob/grep/bash/todo/webfetch/websearch/task/lsp/skill/question),
  `cortex-permissions` (allow/ask/deny), `cortex-policy` (trust), `cortex-checkpoints` (shadow-git).
- `cortex-mcp` (stdio + HTTP/SSE + OAuth), `cortex-lsp` (JSON-RPC + auto-install + diagnostics→agent),
  `cortex-format` (formatters), `cortex-agents` (agents + sous-agents `task` + agents markdown frontmatter),
  `cortex-skills`, `cortex-memory` (AGENTS.md/rules), `cortex-search`, `cortex-plugins` (runtime + hooks),
  `cortex-audit` (sink réel câblé).
- TUI : réutiliser le **port TUI autorisé** (voir §10) — framework + widgets + capture, sous discipline
  de provenance/NOTICE.
- `cortex-server` (axum loopback : sessions, SSE, OpenAPI, contrôle TUI, share local), `cortex-acp`.
- `cortex-cli` (binaire `cortex` : run/tui/serve/auth/models/agent/mcp/...).

### 4.3 Boucle agent (blueprint = opencode V2 `core/src/session/runner/llm.ts`)

`MAX_STEPS=25` ; admission durable des inputs (steer/queue) ; un `llm.stream()` par tour ; appels
d'outils exécutés en **concurrence** (fiber set) puis résultats réinjectés ; **compaction** avant
envoi + récupération sur overflow (rebuild du tour). Voir `research/01` §2.

---

## 5. Périmètre fonctionnel (tout doit être RÉEL)

Liste maître (cocher = livrable réel et validé end-to-end) :

- [ ] **Providers** : OpenAI Chat, OpenAI Responses, Anthropic, Gemini, OpenAI-compatible
  (**OpenRouter** prioritaire), + locaux (ollama/lmstudio) optionnels. Streaming SSE réel.
- [ ] **Auth/credential** : `auth login/list/logout`, clé via env/`{env:}`/store 0600.
- [ ] **Catalogue modèles** : snapshot embarqué + `models --refresh` (models.dev).
- [ ] **Outils** : read, write, edit, apply_patch (swap selon modèle GPT), glob, grep (ripgrep),
  bash (gating + jobs background), todo, webfetch (réel), websearch (réel), task (sous-agents),
  lsp, skill, question. Truncation des grosses sorties.
- [ ] **Permissions** : allow/ask/deny, patterns glob + bash, dernière règle gagnante, par-agent,
  décisions sauvegardées, override par hook plugin.
- [ ] **Checkpoints** : snapshots shadow-git avant mutation, `/undo` `/redo`.
- [ ] **TUI** : streaming md + tree-sitter, diffs, cartes d'outils, **toutes** les slash
  (`/help /new /sessions /models /agents /themes /init /review /compact /export /editor /undo /redo
  /share /mcp /exit` …), `@` mentions fuzzy, mode `!`, palette `ctrl+p`, thèmes, keybinds leader,
  pickers, find-in-chat, toasts.
- [ ] **MCP** : client stdio + remote HTTP/SSE, handshake, tools/list, **tools/call réel**, OAuth,
  mises à jour live de la liste d'outils.
- [ ] **LSP** : auto-install, diagnostics réinjectés après edit/write, outil `lsp`.
- [ ] **Formatters** : exécutés après write/edit (rustfmt/prettier/ruff/gofmt/biome/...).
- [ ] **Agents** : build/plan/general/explore + agents markdown custom (frontmatter) + génération
  d'agent ; **sous-agents `task` réels**.
- [ ] **Skills / references** : injection de guidance dans le system-context.
- [ ] **Plugins/hooks** : ~20 hooks de cycle de vie, enregistrement d'outils/auth/provider,
  exécution **réelle** (cf. §3.3 décision runtime).
- [ ] **Serveur** : API HTTP + OpenAPI, SSE `/event`, CRUD sessions, contrôle TUI, share local.
- [ ] **Config** : TOML layered (defaults<user<project<env<CLI), `AGENTS.md`/instructions,
  substitution `{env}`/`{file}`.
- [ ] **Compaction** auto + overflow. **Persistence** SQLite (sessions/messages/todos/usage).
- [ ] **Redaction** + kill-switch `CORTEX_NO_NET`. **Audit** réel câblé.
- [ ] **Replay** réel (si conservé). **ACP** (sous-ensemble) pour éditeurs.

---

## 6. Technologies & dépendances (vérifiées dispo dans l'env)

Toolchain présent : **cargo 1.96** (edition 2024), node 24, bun 1.3.14, ripgrep 13.

Crates clés (alignées sur l'existant + recommandations codex) :
- Async : `tokio` (multi-thread). HTTP : `reqwest` (**rustls-only**, pas d'openssl). SSE :
  `eventsource-stream` (idle timeout par frame). TLS : `rustls`.
- CLI : `clap` 4 (+`clap_complete`, `clap_mangen`). TUI : `ratatui` 0.29 + `crossterm` 0.28
  (+ le port TUI autorisé, §10). Coloration : `tree-sitter` (+grammars). Markdown : `pulldown-cmark`.
- Persistance : `rusqlite` (bundled) **ou** `sqlx` (à trancher). Sérialisation : `serde`/`serde_json`,
  TOML : `toml`. MCP : envisager `rmcp` (utilisé par codex) ou client maison stdio/SSE. JSON-RPC LSP :
  `vscode`-style maison ou crate. SigV4 (si Bedrock) : `sha2`+`hmac`.
- Hygiène : `unsafe_code = forbid`, `deny.toml` (licences), `[bans] multiple-versions = deny`.

> À **éviter** (leçons codex/existant) : crate sprawl excessif, forks de ratatui/crossterm,
> protocole unique Responses-only (garder un **trait wire** avec adaptateurs Chat **et** Responses,
> car OpenRouter passe par Chat-completions).

---

## 7. Environnement & credentials (vérifié)

- **Clé OpenRouter VALIDE** : `OPENROUTER_API_KEY` dans `/droid/IDE-Cortex/.env.local`
  (préfixe `sk-or-v1-…`, len 73, non free-tier). Appel réel testé OK :
  `POST https://openrouter.ai/api/v1/chat/completions` avec `google/gemini-2.5-flash` → `pong`
  (coût ~0.000004$). `GET /api/v1/key` confirme la clé active.
- **Sourcing** (chaque shell est neuf) : `set -a; . /droid/IDE-Cortex/.env.local; set +a`.
- **Modèles conseillés pour tests** : chat `google/gemini-2.5-flash` (rapide/peu cher) et
  `anthropic/claude-3.5-sonnet` (qualité). ⚠️ NE PAS utiliser `gemini-3.5-flash` (n'existe pas sur
  OpenRouter ; piège relevé dans la mission Cortex IDE).
- **Sécurité** : ne JAMAIS committer la clé ni la logger. `.env.local` gitignored. Redaction active.
- **Nouveau dépôt** : `/projects/cortex-code` n'est pas encore un repo git → `git init` au démarrage
  de la mission ; copier/symlinker la clé via `.env.local` local (gitignored) plutôt que pointer
  vers `/droid/IDE-Cortex`.

---

## 8. Stratégie de prompts (base opencode, rebrandée)

- Adapter les prompts bruts de `research/opencode-prompts-raw/` : structure **identique**
  (system prompt par famille de modèle + bloc environnement dynamique + injection AGENTS.md/rules
  + fragments de mode + descriptions d'outils + compaction/title/summary).
- **Rebranding obligatoire** : remplacer toute mention `opencode` / `opencode.ai` /
  `anomalyco/opencode` par l'identité Cortex Code (cf. `research/02` §5). Notamment dans
  `default.txt` (`/help`, URL d'issues, « when the user asks about opencode … docs at opencode.ai »).
- **Garde CI** : un check qui échoue si `rg -i "opencode"` trouve quoi que ce soit dans
  `crates/**`, les prompts, ou les messages de commit. (À mettre dans services.yaml comme gate.)
- Conserver les règles de ton génériques (concis, <4 lignes, pas de préambule, pas d'emoji).

---

## 9. Stratégie de test & validation

### 9.1 Niveaux de test
- **Unit/intégration** par crate (offline, déterministe — cassettes/fixtures autorisées ICI).
- **End-to-end LIVE** (exigence « no mock ») : exécuter de vrais tours d'agent via OpenRouter et
  prouver les effets réels (fichiers écrits, commandes exécutées, diagnostics LSP, appels MCP réels,
  hooks plugins déclenchés). Skippent proprement si pas de clé, mais la **validation de mission DOIT
  les exécuter avec la vraie clé**.

### 9.2 Surfaces & outils de validation
- **CLI / headless (`cortex run`)** : valider via shell + parsing JSON/JSONL (déterministe, scriptable)
  — c'est la **surface de validation principale** (effets réels observables, peu coûteuse).
- **Serveur (`cortex serve`)** : valider via `curl` (API + SSE `/event`) sur un port dans la plage
  dédiée.
- **TUI** : valider via **`tuistory`** (skill disponible) — lancer le binaire, envoyer des entrées,
  capturer des snapshots, vérifier rendu (md/syntax/diff/slash/@/!). agent-browser ne s'applique pas
  (ce n'est ni web ni Electron).
- Chaque assertion du contrat doit préciser l'outil et les preuves attendues (stdout, fichiers,
  table de process, snapshots TUI, réponses HTTP/SSE, logs d'appels OpenRouter).

### 9.3 Gate programmatique (scrutiny, `services.yaml`)
- `build` : `cargo build -p cortex-cli` (features par défaut). `typecheck` : `cargo check --workspace`.
- `lint` : `cargo clippy --workspace` (définir le niveau exact ; éviter `-D warnings` global si
  baseline bruyante). `test` : suites offline du workspace (les suites live tournent en validation).
- Scoping worker : `cargo check -p <crate>`, `cargo clippy -p <crate>`, `cargo test -p <crate>`.
- **Format on-touch uniquement** (`rustfmt --edition 2024 --skip-children <fichiers>`), jamais
  `cargo fmt` global.

### 9.4 Hygiène processus (CRITIQUE)
- Aucun process orphelin : tout test qui lance le binaire/serveur/LSP/MCP doit l'arrêter **par PID
  exact** (jamais pkill/kill par nom). Ports de test dans une plage dédiée (ex. **3100-3199**).
- Pas de mode watch/interactif dans les tests.

---

## 10. Réutilisation du port TUI (autorisé)

L'existant a **une** exception de port autorisée par le propriétaire (clean-room sinon) :
- Source `CortexLM/cortex-code@7954d0218faf6f26aa84df0f6d8b09ea9f2e5ac9`, roots TUI uniquement
  (`src/cortex-core`→`cortex-frame`, `src/cortex-tui*`, `cortex-tui-framework`).
- **Header de provenance obligatoire** sur chaque fichier porté + entrée NOTICE.
- Réutilisable pour le greenfield **sous la même discipline**. Tout le reste = clean-room original.
- ⚠️ Cette autorisation concerne le **framework TUI**, pas opencode. Le rebranding « pas d'opencode »
  (§8) s'applique séparément aux prompts/UX.

À confirmer avec l'utilisateur (§13) : réutilise-t-on ce port TUI, ou on réimplémente le TUI from
scratch sur ratatui pur ?

---

## 11. Jalons proposés (à confirmer en début de mission)

Chaque jalon = tranche verticale testable. Proposition (ordre = dépendances) :

1. **M0 — Socle & squelette marchant** : workspace cargo, config TOML layered, event bus,
   persistence SQLite, redaction, CLI minimal, binaire qui démarre (TUI vide + `run` echo). Gate
   build/lint/test.
2. **M1 — LLM réel + boucle agent (headless)** : couche `cortex-llm` (trait + OpenAI-compatible →
   **OpenRouter live**), transport SSE, `LlmEvent`, runner durable (MAX_STEPS, streaming),
   `cortex run` qui répond réellement via OpenRouter. **1ère preuve end-to-end live.**
3. **M2 — Outils fichiers/shell + permissions + checkpoints** : read/write/edit/apply_patch/glob/
   grep/bash/todo, allow/ask/deny, shadow-git undo/redo. L'agent **code réellement** en headless.
4. **M3 — TUI complet** : rendu streaming md/syntax/diff, cartes outils, slash, `@`, `!`, palette,
   thèmes, keybinds, pickers, undo/redo, toasts. Validation `tuistory`.
5. **M4 — Extensibilité réelle** : MCP (stdio+HTTP/SSE, tools/call réel), LSP (diagnostics+nav),
   formatters, sous-agents `task`, skills, memory/AGENTS.md.
6. **M5 — Plugins/hooks réels** + webfetch/websearch réels + audit câblé.
7. **M6 — Serveur/API/SSE/OpenAPI** + contrôle TUI distant + share local + ACP.
8. **M7 — Finitions** : models.dev refresh, stats/coûts, export/import, completions/man, replay réel,
   doc utilisateur, garde CI anti-« opencode », durcissement no-mock end-to-end.

> Compromis à exposer à l'utilisateur : 8 jalons = granularité fine/qualité mais coût plus élevé.
> On peut fusionner (ex. M4+M5) si on veut accélérer.

---

## 12. Design du système de workers (skills à créer)

Types de workers proposés (un skill `SKILL.md` par type dans `{missionDir}/skills/`) :

- **`core-worker`** : crates moteur (config, events, persistence, core/runner, llm, providers,
  transport). TDD Rust strict, scoping `-p <crate>`, preuves live OpenRouter quand pertinent.
- **`tools-worker`** : outils agent + permissions + checkpoints + formatters + LSP + MCP. Vérif
  black-box des effets réels (fichiers, process, diagnostics).
- **`tui-worker`** : surface TUI (rendu, slash, mentions, palette, thèmes, keybinds). Vérif via
  `tuistory` (snapshots). Respect du port TUI autorisé + provenance.
- **`server-worker`** : `cortex serve` (axum, SSE, OpenAPI, contrôle TUI, share, ACP). Vérif `curl`.
- **`agents-plugins-worker`** : agents/sous-agents, skills, memory, plugins/hooks, audit. Vérif
  exécution réelle.

Chaque skill : frontmatter, Required Skills/Tools, Work Procedure (TDD red→green→vérif black-box,
scoping cargo, format on-touch, hygiène process PID-exact), Example Handoff riche, When to Return.

Validation auto (injectée par le système) : `scrutiny-validator` + `user-testing-validator` par jalon.

---

## 13. Questions ouvertes à reconfirmer au démarrage de la mission

1. **Port TUI** : réutiliser le port autorisé (`CortexLM/cortex-code@7954d02`, framework TUI) sous
   provenance/NOTICE, ou réimplémenter le TUI from scratch en ratatui pur ?
2. **Runtime plugins** : embarquer un moteur JS (deno_core/boa/quickjs) pour un produit
   self-contained, OU formaliser le contrat « runtime JS externe détecté » (Bun/Node/Deno) ? (Impacte
   « no mock / tout réel ».)
3. **Périmètre des exclusions** de l'existant : share hébergé, GitHub App, télémétrie, Windows,
   sandbox OS, browser/computer-use — in ou out pour ce build « complet » ?
4. **Auth providers étendue** (Azure AD, Vertex JWT, Bedrock SSO, OAuth providers) : nécessaire, ou
   API-key/OpenRouter suffit pour V1 ?
5. **Persistence** : `rusqlite` (comme l'existant) ou `sqlx` ?
6. **Licence/identité** : nom de produit, repo/URL officiels Cortex Code (pour remplacer les URLs
   d'opencode dans les prompts et le `/help`).
7. **Modèle par défaut** livré (ex. `google/gemini-2.5-flash` via OpenRouter) ?

---

## 14. Checklist de démarrage de la nouvelle session

1. Charger ce brief + lire `research/04`, `01`, `02`(+raw), `03`.
2. Lancer `mission-planning` + `define-mission-skills`. Reconfirmer §13 avec l'utilisateur.
3. `git init` dans `/projects/cortex-code` ; créer `.gitignore` (cible `target/`, `.env.local`,
   secrets) ; créer `.env.local` local avec `OPENROUTER_API_KEY` (gitignored).
4. Phase readiness : revérifier la clé OpenRouter (appel live), cargo build d'un hello-world
   workspace, `tuistory` opérationnel, ports 3100-3199 libres.
5. `propose_mission` → puis dans le missionDir : `architecture.md`, `validation-contract.md`
   (sous-agents par feature + passes de revue), `validation-state.json`, `features.json`,
   `AGENTS.md` (boundaries + directives : no-mock, no-opencode, PID-exact, format on-touch),
   `services.yaml` (build/check/clippy/test + garde anti-opencode + services serveur), `init.sh`,
   `library/` (environment.md, user-testing.md avec surfaces CLI/serveur/TUI + concurrence).
6. Décomposer en features par jalon (§11), `fulfills` ↔ assertions du contrat.
7. `start_mission_run`.

---

## 15. Garde-fous transverses (à inscrire dans AGENTS.md de la mission)

- **NO MOCK livré** : le défaut utilisateur est toujours le chemin réel. Fixtures/cassettes =
  tests uniquement. Toute feature « terminée » doit avoir une preuve end-to-end réelle.
- **NO « opencode »** nulle part (code, prompts, commits, docs). Garde CI active.
- **Sécurité** : jamais de clé en clair commitée/loggée ; redaction ; `CORTEX_NO_NET` respecté.
- **Process** : kill par **PID exact** uniquement ; pas d'orphelins ; ports 3100-3199.
- **Format** : on-touch only (`rustfmt --edition 2024 --skip-children`), jamais `cargo fmt` global.
- **Référence lecture seule** : ne jamais modifier `/droid/cortex-code` ni `/tmp/opencode-sst` ni
  `/tmp/codex`.
- **Provenance** : tout fichier porté (TUI autorisé) porte son header + entrée NOTICE.
```
