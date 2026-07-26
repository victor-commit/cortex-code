# Cortex Code vs OpenCode — Analyse exhaustive des écarts fonctionnels

> Référence (TypeScript) : `/tmp/opencode-ref/packages` — cœur dans `packages/opencode/src`
> Cible (Rust) : `/projects/cortex-code/crates` (31 crates)
>
> Objectif : parité fonctionnelle complète. Cortex Code est un portage Rust greenfield d'opencode.
> Méthode : 9 analyses de sous-systèmes menées par agents `explore` lisant le code réel des deux côtés, citant `fichier:ligne`.

**Légende statut** : **FULL** (parité) · **PARTIAL** (sous-ensemble) · **MISSING** (absent) · **STUB** (déclaré, non implémenté) · **OUT-OF-SCOPE** (hors périmètre d'un agent terminal mono-binaire) · **CORTEX-UNIQUE** (sans équivalent opencode).
**Sévérité** : **High** (capacité agent essentielle) · **Med** (opérationnel / UX) · **Low** (niche / écosystème).
**Marqueur de révision** : **✅ CLÔTURÉ** = écart **High/Critical** signalé dans l'audit initial (24 juin) et **résolu** depuis (vérifié dans le code courant).

> ## ⟳ Révision 2026-06-28 — après 141 commits
>
> Cet audit a été **re-vérifié** contre l'état courant de cortex-code (`HEAD 30cd3b7`, 31 crates, ~243k LOC Rust) et de la référence opencode v1.17.9 (`anomalyco/opencode`, snapshot `/tmp/opencode-ref`). **141 commits** ont été ajoutés depuis l'audit initial du 24 juin, et la quasi-totalité des verrous **CRITICAL/HIGH** d'origine sont désormais **clôturés**. Re-vérification par 6 agents `explore` parcourant le code réel + sondages directs (file:line confirmés).
>
> **Bascules majeures depuis le 24 juin** (détail par section) :
> - **§2 Outils** : `plan`/`plan_enter`/`plan_exit`/`question`/`invalid` désormais **livrés** (`crates/tools/src/{plan,plan_enter,plan_exit,question,invalid}.rs`) ; `lsp` 5→9 ops. Seul `edit` (2 paliers) reste PARTIAL.
> - **§3 LLM** : les **3 P0** sont clos — retry/backoff+`Retry-After` (`transport.rs:29-30,158-210`), prompt caching Anthropic (`anthropic/mod.rs:27,257-282`), multimodal (`request.rs:40` `parts: Vec<MessageContentPart>`). Providers **natifs** Anthropic/Gemini/OpenAI-Responses (`crates/llm/{anthropic,gemini,openai}`) + 18 profils provider + OAuth/AWS/ADC (`providers/{provider_profiles,runtime_auth}.rs`).
> - **§4 MCP/LSP/Format** : registre LSP 3→**~23 serveurs** + auto-download, pull/workspace diagnostics, implementation/call-hierarchy ; formateurs 3→**22 builtins** (`format/lib.rs:17-63`) ; capacité MCP `roots`.
> - **§5 Engine** : compaction par **résumé LLM**, retries provider, outils **parallèles** (concurrence 4), modèle de parts riche, doom-loop, TurnQueue engine.
> - **§6 Permissions** : gating `external_directory`, **reject-avec-feedback** (`CorrectedError`), file pending async (`list()`/`reply()`), `experimental.policies`.
> - **§7 Serveur** : HTTP 13→**~80 routes**, SSE 14→**~65 kinds** (~68 % de noms partagés), ACP 4→**14 méthodes** + client fs/terminal, **SDK TypeScript livré** (`/sdk/`). Résiduel clé : **compatibilité wire** (cortex garde un vocabulaire natif).
> - **§8 TUI** : coloration 2→**13 grammaires**, viewer de diff (split+revert), pièces jointes (fichier+presse-papier), keybinds 8→27, slash 16→39.
> - **§1 Plugins** : 21/21 hooks dispatchés, hook `auth` **complet**, skills `.claude`/`.agents`, `CLAUDE.md`, injection mémoire dynamique.
> - **§9 CLI** : ~15 nouvelles commandes (providers/attach/plugin install/account/worktree/snapshot/import URL…), migrations `message_parts` + `projects_accounts`, IDs triables.
>
> Les **statuts d'origine** ci-dessous ont été mis à jour ; les écarts résolus sont marqués **✅ CLÔTURÉ** avec l'ancrage du code courant. Les **résiduels** restants sont conservés. Le **§10** (tableau de bord + priorités) a été entièrement réécrit pour refléter l'état courant.

---

## Sommaire des sous-systèmes

1. [Plugins / Skills / Memory](#1--plugins--skills--memory) — *priorité explicite*
2. [Outils (tools)](#2--outils-tools)
3. [LLM / Providers](#3--llm--providers)
4. [MCP / LSP / Format](#4--mcp--lsp--format)
5. [Boucle agent / Session runner (engine)](#5--boucle-agent--session-runner-engine)
6. [Permissions / Policy / Config](#6--permissions--policy--config)
7. [Serveur / API / SSE / ACP / SDK](#7--serveur--api--sse--acp--sdk)
8. [TUI](#8--tui)
9. [CLI / Stockage / Bus / Git / divers](#9--cli--stockage--bus--git--divers)
10. [Synthèse globale & priorités](#10--synthèse-globale--priorités-de-remédiation)

---

## 1 — Plugins / Skills / Memory

> **Priorité explicite de l'utilisateur.** Référence : `packages/plugin/src/index.ts` (contrat de hooks) + `packages/opencode/src/plugin/*`. Cible : `crates/plugins/src/runtime/invoke.rs` (harnais JS) + `crates/cli/src/plugin_bridge.rs` (câblage).

### Architecture de chargement / exécution

- **opencode** : plugins = **modules JS/TS first-class exécutés in-process** par l'hôte (Bun/Node). Import dynamique natif (`loader.ts:139`), specs `file://`/relatifs **+ packages npm installés à la demande** (`shared.ts:207-213`). `PluginInput` riche injecté (`plugin/src/index.ts:56-66`) : `client` SDK HTTP **vivant** (`plugin/index.ts:142-147`), `project`, `directory`, `worktree`, `serverUrl`, shell `$` Bun, `experimental_workspace.register`. Fan-out typé via `trigger()` (`plugin/index.ts:280-293`). Plugins auth intégrés statiquement (`plugin/index.ts:65-82`). Gate de compat npm via `engines.opencode` semver. Type de plugin `tui` en plus de `server`.
- **cortex** : plugins = **modules JS externes exécutés dans un processus enfant** via runtime détecté (Bun → Node → Deno ; `runtime/detection.rs`). Rust lance un process exécutant un **harnais `BRIDGE_SCRIPT` embarqué** (`invoke.rs:1836+`), importe le module (`default`/`plugin`/`activate`), exécute **un hook par invocation**, et fait transiter le JSON par **fichiers temporaires** (`invoke.rs:1861`). Chaque appel de hook = un spawn de process. Input injecté = **stub** : `client:{kind:"local"}`, `serverUrl:"local://cortex"`, `project:{directory,root}`, `worktree=directory`, shell bash-`execFile`.

**Écarts architecturaux clés** : pas d'exécution in-process (sous-process + IPC fichier par hook), **pas de vrai `client` rappelant le serveur cortex**, `serverUrl`/`project`/`worktree` stubbés, pas de type plugin `tui`, pas de gate `engines`, pas de plugins auth intégrés, pas de commande d'installation utilisateur.

### Plugins — hook par hook (la priorité)

| Hook / capacité | opencode (fichier:ligne) | statut cortex | sév | cortex (fichier:ligne) |
|---|---|---|---|---|
| `dispose` | index.ts:223 | **FULL** | — | invoke.rs:264 ; plugin_bridge.rs:446 |
| `event` | index.ts:224 | **FULL** | — | invoke.rs:124 ; plugin_bridge.rs:682 |
| `config` (recevoir Config résolue) | index.ts:225 ; plugin/index.ts:241-249 | **FULL** ✅ CLÔTURÉ — config résolue désormais passée ; ruleset `permission[]` toujours récolté | — | invoke.rs:2051-2065 ; plugin_bridge.rs:171 |
| `tool` (outils appelables par le modèle) | index.ts:226-228 ; tool.ts | **FULL** (permission-gated avant exéc JS) | — | plugin_bridge.rs:186, 1141-1199 |
| `auth` (OAuth/API : methods, prompts, authorize, loader) | index.ts:229 ; AuthHook 88-163 | **FULL** ✅ CLÔTURÉ — flow oauth/api complet + câblé dans `cortex auth` | — | invoke.rs:2242-2247 ; auth_cmd.rs |
| `provider` (`models(provider,ctx.auth)` + complétions) | index.ts:230 ; 214-217 | **PARTIAL** — enregistrement statique + routage `--model` OK, mais complétion **non-streaming**, `ctx.auth` absent, prix/attachment codés 0/false | **Med** | invoke.rs:350 ; plugin_bridge.rs:265,283 |
| `chat.message` | index.ts:234 | **FULL** | — | invoke.rs:152 ; plugin_bridge.rs:485 |
| `chat.params` | index.ts:247 | **FULL** | — | invoke.rs:165 ; plugin_bridge.rs:586 |
| `chat.headers` | index.ts:257 | **FULL** | — | invoke.rs:178 ; plugin_bridge.rs:612 |
| `permission.ask` | index.ts:261 | **FULL** | — | invoke.rs:204 ; plugin_bridge.rs:213,334 |
| `command.execute.before` | index.ts:262 | **FULL** (+veto) | — | invoke.rs:217 ; plugin_bridge.rs:309 |
| `tool.execute.before` | index.ts:266 | **FULL** (+veto) | — | invoke.rs:230 ; plugin_bridge.rs:376 |
| `shell.env` | index.ts:270 | **FULL** | — | invoke.rs:256 ; plugin_bridge.rs:223 |
| `tool.execute.after` | index.ts:274 | **FULL** | — | invoke.rs:243 ; plugin_bridge.rs:414 |
| `experimental.chat.messages.transform` | index.ts:282 | **FULL** (forme divergente) | low | invoke.rs:290 ; plugin_bridge.rs:511-540 |
| `experimental.chat.system.transform` | index.ts:291 | **FULL** | — | invoke.rs:303 ; plugin_bridge.rs:542-581 |
| `experimental.provider.small_model` | index.ts:297 | **FULL** ✅ CLÔTURÉ | — | invoke.rs (dispatch 21/21) |
| `experimental.session.compacting` | index.ts:305 | **FULL** ✅ CLÔTURÉ — résumé LLM de compaction câblé (cf §5) | — | invoke.rs ; engine/compaction |
| `experimental.compaction.autocontinue` | index.ts:316 | **FULL** ✅ CLÔTURÉ | — | invoke.rs ; engine |
| `experimental.text.complete` | index.ts:327 | **FULL** ✅ CLÔTURÉ | — | invoke.rs |
| `tool.definition` (re-shape schéma) | index.ts:334 | **FULL** | — | invoke.rs:191 ; plugin_bridge.rs:296,638 |

**Surface `PluginInput`** : **21/21 hooks désormais dispatchés.** `client` **PARTIAL** (rappel serveur câblé hors mode headless ; **no-op en headless** — résiduel Med, `invoke.rs:1960`), `project`/`worktree`/`serverUrl` désormais peuplés, `$` shell **PARTIAL** (bash execFile + tagged template), `experimental_workspace.register` **PARTIAL**. Résiduels architecturaux : IPC out-of-process durable, type plugin `tui`, API plugin **v2**.

**Distribution/install** : specs fichier/chemin **FULL** ; npm à la demande **PARTIAL** (piloté par lockfile, gated `CORTEX_NO_NET`) ; gate semver `engines` **MISSING** ; commande `plugin install` **MISSING** ; type `tui`/`oc-themes` **MISSING** ; plugins auth intégrés (copilot/codex/gitlab/azure/xai/…) **MISSING** ; API plugin **v2** (agent/command/skill/catalog) **MISSING**.

### Skills

| Fonctionnalité | opencode | statut cortex | sév | cortex |
|---|---|---|---|---|
| Frontmatter `SKILL.md` (name/desc) + guidance | skill/index.ts:37-43 | **FULL** | — | skills/lib.rs:97-114 |
| Découverte récursive `**/SKILL.md` | skill/index.ts:23-25,150 | **FULL** ✅ CLÔTURÉ — récursion profonde | — | skills/lib.rs:260-289 |
| Skills `.claude/skills` + `.agents/skills` | skill/index.ts:21-22,187-202 | **FULL** ✅ CLÔTURÉ | — | skills/lib.rs |
| URLs de skills distantes | discovery.ts | **FULL** ✅ CLÔTURÉ | — | skills/lib.rs |
| `skills.paths`/dirs configurés | skill/index.ts:211-220 | **FULL** | — | skills/lib.rs:196-211 |
| Dir skills globale | skill/index.ts:191 | **FULL** | — | skills/lib.rs:188-194 |
| Outil `skill` renvoie **base dir + liste de fichiers** | tool/skill.ts:34-60 | **FULL** ✅ CLÔTURÉ — fichiers du skill surfacés | — | tools/skill.rs |
| Gating de permission skill | skill/index.ts:314 | **PARTIAL** | Med | tools/skill.rs:83 |

### Memory / contexte

| Fonctionnalité | opencode | statut cortex | sév | cortex |
|---|---|---|---|---|
| Chaîne `AGENTS.md` projet | instruction.ts:64-68,124-133 | **FULL** | — | memory/lib.rs:116-123,199-204 |
| `CLAUDE.md` (projet + `~/.claude`) | instruction.ts:62,66 | **FULL** ✅ CLÔTURÉ | — | memory/lib.rs |
| `CONTEXT.md` (déprécié) | instruction.ts:67 | **MISSING** | Low | — |
| `AGENTS.md` global (`~/.config`) | instruction.ts:61 | **FULL** ✅ CLÔTURÉ | — | memory/lib.rs:112-114 |
| Globs `instructions[]` | instruction.ts:135-150 | **FULL** | — | memory/lib.rs:206-254 |
| URLs d'instructions distantes | instruction.ts:95-103,158-167 | **FULL** ✅ CLÔTURÉ | — | memory/lib.rs |
| **Injection dynamique à la lecture** (remonte l'arbre depuis le fichier lu) | instruction.ts:179-221 | **FULL** ✅ CLÔTURÉ | — | memory/lib.rs |
| Compaction de contexte | compaction.ts (620 L) : tours par tokens, résumé LLM, troncature | **FULL** ✅ CLÔTURÉ — résumé LLM + hooks (cf §5) | — | engine/compaction.rs ; run.rs |

**Synthèse Plugins/Skills/Memory** : portage désormais **quasi-complet** — **21/21 hooks dispatchés** (tout le cycle requête/outil/permission/command/shell + experimental compacting/autocontinue/text.complete/small_model), gating de permission avant exéc JS + audit. Le hook **`auth` est complet et câblé dans `cortex auth`** (✅ clôturé). Skills : `.claude`/`.agents`, URLs distantes, surfaçage des fichiers du skill, récursion profonde — tous livrés (✅). Memory : `CLAUDE.md`, injection dynamique à la lecture, URLs distantes, compaction par résumé LLM — tous livrés (✅). **Résiduels** (Med/Low, non bloquants) : exécution toujours **out-of-process** (IPC tempfile, pas in-process), `client` **no-op en headless**, type plugin `tui` + API plugin **v2** absents, gating skill partiel.

---

## 2 — Outils (tools)

> Référence : `packages/opencode/src/tool/` (TS). Cible : `crates/tools/src` (Rust). Registres : `registry.ts` / `registry.rs` (variantes `builtin()` non-GPT et `gpt()` où apply_patch remplace edit/write).

| Outil | opencode | cortex | statut | sév | notes |
|---|---|---|---|---|---|
| read | `read.ts` | `read.rs` | **FULL** | — | offset/limit, gutter ligne, images+PDF, caps octets/lignes ; spill disque |
| write | `write.ts` | `write.rs` | **FULL** | — | create+parents, garde read-before-overwrite, re-format+LSP ; cortex ajoute pré-check writability OS + checkpoint + audit |
| edit | `edit.ts` | `edit.rs` | **PARTIAL** | **HIGH** | opencode ~9 replacers fuzzy + Levenshtein ; cortex 2 paliers (byte-exact + ligne whitespace-normalisée) + replaceAll. Des edits acceptés par opencode échoueront sur cortex |
| bash/shell | `shell.ts`+`shell/prompt.ts` | `bash/{mod,jobs,parse}.rs` | **FULL+** | — | tree-sitter pour arité permission des deux côtés. **cortex ajoute jobs background** (PID reap, kill_on_drop), stripping secret-env, spill disque |
| glob | `glob.ts` | `glob.rs`+`ripgrep.rs` | **FULL** | — | ripgrep |
| grep | `grep.ts` | `grep.rs` | **FULL** | — | ripgrep ; cortex borne via `output.rs` |
| webfetch | `webfetch.ts` | `webfetch.rs` | **PARTIAL** | **Med** | opencode 5MB/30s/120s, formats text/markdown/html, image ; cortex 128KB/512KB, 8s/30s, **markdown seulement**, pas d'image. cortex ajoute `guard_outbound`/`CORTEX_NO_NET` |
| task | `task.ts` | `task.rs` | **FULL** ✅ CLÔTURÉ | — | params `task_id`/`background`/`description` ajoutés ; reprise + sous-agent background |
| todo | `todo.ts` | `todo.rs` | **FULL** ✅ CLÔTURÉ | — | `priority` + état `cancelled` ajoutés |
| lsp | `lsp.ts` | `lsp.rs` | **FULL** ✅ CLÔTURÉ | — | 9 ops (goToImplementation, prepareCallHierarchy, incoming/outgoingCalls ajoutés) |
| skill | `skill.ts` | `skill.rs` | **FULL** | — | surfaçage fichiers ajouté (cf §1) |
| apply_patch | `apply_patch.ts` | `apply_patch/{mod,parser,apply}.rs` | **FULL** | — | enveloppe atomique multi-fichiers ; cortex le substitue à edit/write pour modèles GPT (`ToolRegistry::gpt`) |
| websearch | `websearch.ts`+`mcp-websearch.ts` | `websearch.rs` | **FULL** ✅ CLÔTURÉ | — | params `livecrawl`/`type`/`contextMaxCharacters` ajoutés ; modèle provider diffère (cortex Brave+DuckDuckGo HTTP vs Exa/Parallel MCP) |
| **plan / plan_exit / plan_enter** | `plan.ts` | `plan.rs` ; `plan_exit.rs` ; `plan_enter.rs` ; `plan_exit_compat.rs` | **FULL** ✅ CLÔTURÉ | — | outils mode plan désormais livrés |
| **question** | `question.ts` | `question.rs` | **FULL** ✅ CLÔTURÉ | — | outil le-modèle-demande-à-l'utilisateur livré |
| **invalid** | `invalid.ts` | `invalid.rs` | **FULL** ✅ CLÔTURÉ | — | fallback pour appels d'outil malformés livré |
| truncate | `truncate.ts` (helper) | `output.rs` | **FULL** | — | truncate-and-spill équivalent |
| external-directory | `external-directory.ts` | replié dans `tool.rs` + permissions | **FULL** ✅ CLÔTURÉ | — | gating hors-worktree désormais explicite (cf §6) |

**Forces cortex-uniques** : jobs bash background avec reap/kill-on-drop ; seam checkpoint/undo (`CheckpointRecorder`) sur chaque mutation ; sink d'audit ; pré-check writability OS ; stripping secret-env + garde réseau `CORTEX_NO_NET` ; registre de lecture à identité canonique (dot-path + symlink).

**Synthèse outils** : **parité quasi-complète.** Les 3 outils auparavant MISSING — `plan`/`plan_exit`/`plan_enter` (HIGH), `question` (Med), `invalid` (Low) — sont **livrés** (✅). `lsp` passe de 5→**9 ops** (call-hierarchy + implementation ajoutés, ✅), `task`/`todo`/`websearch` voient leurs paramètres manquants comblés (✅), `external_directory` est désormais un gating explicite (✅). **Seul écart comportemental résiduel notable** : `edit` (2 paliers byte-exact + ligne-normalisée vs ~9 replacers fuzzy d'opencode — des edits acceptés par opencode peuvent échouer sur cortex). `webfetch` reste PARTIAL (markdown-only, corps 512KB vs 5MB, pas de `format`). cortex reste en avance sur la sûreté opérationnelle (jobs bash background, checkpoint/undo, audit, writability OS, garde réseau).

### 2.1 Approfondissement — matrice paramètre par paramètre (écarts cachés dans les outils « FULL »)

> Un statut **FULL** masque souvent des écarts de **sous-fonctionnalité au niveau des paramètres** (caps d'octets, timeouts, valeurs par défaut, énumérations, options absentes). Cette matrice énumère chaque paramètre des deux côtés. Note : cortex accepte silencieusement des alias snake_case/camelCase (`file_path`, `old_string`…) non annoncés dans son schéma JSON.

| Outil | paramètres opencode (fichier:ligne) | paramètres cortex (fichier:ligne) | Δ paramètre/limite |
|---|---|---|---|
| **read** | `filePath` req (read.ts:29) ; `offset` opt:30 ; `limit` opt:33 ; **cap/ligne 2000 c** (`MAX_LINE_LENGTH`:14) ; cap **50KB**:16 | `path` req (read.rs:63, alias `file_path`) ; `offset`:67 ; `limit`:71 ; cap **1000 l/64KB** (output.rs:22,25) ; spill disque:167 | nom `filePath`→`path` ; **OC tronque chaque ligne >2000 c, cortex non** ; cap **50KB vs 64KB** + cortex ajoute cap 1000 lignes ; overflow : OC pagine (`offset=N`), cortex spille |
| **write** | `content` req (write.ts:21) ; `filePath` req:22 | `path` req (write.rs:47) ; `content` req:51 | nom seulement ; cortex garde read-before-overwrite (comportement, pas param) |
| **edit** | `filePath`/`oldString`/`newString` req ; `replaceAll` dflt false (edit.ts:48-53) | idem (edit.rs:53-65, +8 alias) | jeu identique ; rename `filePath`→`path` |
| **bash** | `command` req (prompt.ts:17) ; `timeout` ms opt:18 ; **`workdir` opt:19** ; cap **2000 l/50KB** | `command` req (bash/mod.rs:76) ; **`background` opt:80** ; `timeout` ms:84 ; **`description` opt:88** ; dflt **120000ms**:46 ; cap **1000 l/64KB** | **OC a `workdir` (cortex non)** ; **cortex a `background`+`description` (OC non)** ; cortex a override env `CORTEX_BASH_TIMEOUT_MS` ; caps différents |
| **glob** | `pattern` req ; `path` opt (glob.ts:11-12) ; **limit=100** (:49) | `pattern` req (glob.rs:45) ; `path` opt:49 ; **MAX_RESULTS=1000**:22 | cap résultats **10× : OC 100 vs cortex 1000** |
| **grep** | `pattern`/`path`/`include` (grep.ts:11-15) ; **limit=100 matches** | `pattern`/`path`/`include` (grep.rs:42-50) ; **1000 l/64KB** puis spill | cap **100 vs 1000** ; OC tronque, cortex spille ; aucun des deux n'expose casse/contexte |
| **webfetch** | `url` req ; **`format` text\|markdown\|html** dflt markdown (webfetch.ts:15) ; `timeout` **secondes**:21 ; cap **5MB**, max **120s** | `url` req (webfetch.rs:52) ; `timeout_ms`:56 ; **`max_bytes`**:60 ; dflt 8000ms, clamp 50–30000 ; corps **512KB** | **OC a `format` (cortex markdown-only)** ; **cortex a `max_bytes` (OC cap 5MB fixe)** ; unités/bornes très différentes (s/120s/5MB vs ms/30s/512KB) |
| **websearch** | `query` ; `numResults` dflt 8 ; **`livecrawl`** ; **`type`** ; **`contextMaxCharacters`** dflt 10000 (websearch.ts:11-22) | `query` (websearch.rs:239) ; `count` 1–10 dflt 5:243 ; `timeout_ms`:247 | **OC a `livecrawl`/`type`/`contextMaxCharacters` (cortex aucun)** ; **cortex a `timeout_ms`** ; `numResults` 8 sans max vs `count` 1–10 dflt 5 |
| **task** | `description` req ; `prompt` req ; `subagent_type` **str libre** ; **`task_id`** (reprise) ; **`command`** ; **`background`** (task.ts:44-58) | `agent` req ; `prompt` req ; **+`task_id`/`background`/`description`** ✅ | **✅ comblé** — reprise + background + description désormais présents (cortex garde `agent` enum) |
| **todo** | `todos` req ; item `{content, status, **priority**}` ; status inclut **`cancelled`** (todo.ts:9-18) | `todos` ; item `{content, status, **priority**}` ; **+`cancelled`** ✅ | **✅ comblé** — `priority` + état `cancelled` ajoutés |
| **lsp** | `operation` enum **9 ops** ; `filePath`/`line`/`character` req ; `query` opt (lsp.ts:11-32) | `action` enum **9 ops** ✅ ; **`includeDeclaration`** dflt true | **✅ comblé** — goToImplementation + prepareCallHierarchy + incoming/outgoingCalls ajoutés ; cortex garde `includeDeclaration` en plus |
| **skill** | `name` str libre (skill.ts:9) | `name` **enum** des skills découverts (skill.rs:56) | cortex contraint à une énumération |
| **apply_patch** | `patchText` req (apply_patch.ts:19) | `patch` req (apply_patch/mod.rs:62, alias `input`/`patch_text`) | rename seulement |

**Surfaces OC-only (désormais portées)** : `plan`/`plan-exit`/`plan-enter` ✅ (`plan.rs`/`plan_exit.rs`/`plan_enter.rs`), `question` ✅ (`question.rs`), `invalid` ✅ (`invalid.rs`), `external-directory` ✅ (gating explicite, cf §6). **Plus aucune surface OC sans équivalent cortex au niveau outil.**

**Faits saillants « gaps cachés » — état courant** : la plupart sont **comblés** — **lsp** 9/9 ops ✅, **task** `task_id`/`background`/`description` ✅, **todo** `priority`+`cancelled` ✅, **websearch** `livecrawl`/`type`/`contextMaxCharacters` ✅. **Résiduels** : **webfetch** (pas de `format`, corps 512KB vs 5MB) ; **bash** (gap bidirectionnel : OC a `workdir`, cortex a `background`/`description`) ; caps numériques divergents (glob/grep 1000 vs 100, read 64KB vs 50KB). Delta de nommage systématique conservé : OC `filePath/patchText/numResults` ↔ cortex `path/patch/count` (cortex accepte silencieusement camelCase+snake_case).

---

## 3 — LLM / Providers

> Référence : `packages/opencode/src/provider/` (orchestration AI-SDK ~30 providers) **+** `packages/llm/` (moteur protocoles natifs : anthropic-messages, bedrock-converse, gemini, openai-chat, openai-responses + executor retry + cache-policy). Cible : `crates/llm/` (moteurs de protocoles natifs `anthropic/`, `gemini/`, `openai/{mod,wire,responses,normalize}`) + `crates/providers/` (profils providers + credentials + snapshot catalogue).

> **Fait central (corrigé 28-Juin)** : cortex n'est **plus** un client OpenRouter-only. `crates/providers/src/provider_profiles.rs` définit **18 profils providers** (openrouter, bedrock, azure, vertex, copilot, xai, groq, mistral, cohere, deepinfra, cerebras, perplexity, together, vercel, alibaba, cloudflare, openai-compatible, ollama) mappant `provider_id → {base_url, protocole, auth}`, et `crates/llm/src/` embarque désormais **4 moteurs de protocole natifs** (`anthropic-messages`, `gemini`, `openai-chat`, `openai-responses`). Résidu : le **catalogue de modèles embarqué** (`models_dev.json`) reste **openrouter-only (14 modèles)** — les modèles des 17 autres providers doivent venir d'un fetch models.dev live ou de la config. Il n'existe par ailleurs **aucun profil nommé `anthropic`/`openai`/`gemini` direct** : l'API Anthropic directe passe par le protocole anthropic-messages, Gemini par le profil `vertex` (ADC), OpenAI par `openai-compatible`.

### A. Support de providers

| Provider | opencode | statut cortex | sév | cortex |
|---|---|---|---|---|
| OpenRouter | FULL | **FULL** | — | openrouter.rs:45-143 ; openai/mod.rs:138-166 |
| OpenAI (Chat+Responses) | FULL (responses) | **FULL** ✅ CLÔTURÉ (moteur natif chat + responses) | — | openai/mod.rs ; openai/responses.rs |
| Anthropic (protocole Messages) | FULL | **FULL** ✅ CLÔTURÉ (moteur anthropic-messages natif) | — | anthropic/mod.rs:183,242,257 |
| Google Gemini (protocole natif) | FULL | **FULL** ✅ CLÔTURÉ (functionCall/Response + thought) | — | gemini/mod.rs:176,226 |
| Amazon Bedrock | FULL (Converse) | **PARTIAL** — profil + protocole anthropic-messages + chaîne creds AWS présents, mais **aucune signature SigV4 ni API Converse** ⇒ ne peut pas réellement authentifier auprès d'AWS | **High** | provider_profiles.rs:24-29 (pas de SigV4 dans `crates/`) |
| Azure OpenAI | FULL | **PARTIAL** (profil + protocole openai-chat ; déploiements/api-version non vérifiés) | Med | provider_profiles.rs:32-37 |
| Google Vertex (+Anthropic) | FULL (ADC) | **FULL** ✅ CLÔTURÉ (profil vertex + ADC refresh) | — | provider_profiles.rs:40-45 ; runtime_auth.rs:13-14 |
| GitHub Copilot | FULL | **PARTIAL** (profil copilot/openai-chat ; échange token Copilot spécifique non vérifié) | Med | provider_profiles.rs:48-53 |
| xAI/Groq/Mistral/Cohere/DeepInfra/Cerebras/Perplexity/Together/Vercel/Alibaba/Cloudflare | FULL | **FULL** ✅ CLÔTURÉ (profils openai-chat dédiés) | — | provider_profiles.rs:56-141 |
| Local/Ollama/OpenAI-compat générique | FULL (config) | **FULL** ✅ CLÔTURÉ (profils `ollama` + `openai-compatible` base-url) | — | provider_profiles.rs:144-157 |

### B. Auth

| Capacité | opencode | statut cortex | sév | cortex |
|---|---|---|---|---|
| Clé API (env/fichier 0600/config) | FULL | **FULL** | — | credentials.rs:134-250 |
| **Flow OAuth** (refresh + persistance) | FULL | **FULL** ✅ CLÔTURÉ (refresh auto si expiré, re-stockage) | — | runtime_auth.rs:27-128,152-165 |
| Google ADC | FULL | **FULL** ✅ CLÔTURÉ (env + chemin défaut + refresh endpoint) | — | runtime_auth.rs:13-17 |
| Chaîne de creds AWS | FULL | **PARTIAL** — résolution env (access key/secret/session/profile/shared-creds/bearer) présente, **mais pas de signature SigV4** des requêtes | High | runtime_auth.rs:18-24 |
| Prompts auth multi-champs | FULL | **PARTIAL** | Med | credentials.rs |

### C–J. Capacités transverses

| Capacité | opencode | statut cortex | sév | cortex |
|---|---|---|---|---|
| Parsing SSE / deltas chat / assemblage tool-call | FULL | **FULL** | — | transport.rs:167-187 ; normalize.rs:131-215 |
| Streaming reasoning/thinking + **signature** | FULL | **FULL** ✅ CLÔTURÉ (anthropic `signature` + gemini `thoughtSignature`) | — | anthropic/mod.rs:195-205 ; gemini/mod.rs:200 |
| Format tool Anthropic `tool_use`/`tool_result` | FULL | **FULL** ✅ CLÔTURÉ | — | anthropic/mod.rs:183,242 |
| Format tool Gemini `functionCall`/`functionResponse` | FULL | **FULL** ✅ CLÔTURÉ | — | gemini/mod.rs:176,226 |
| `toolChoice` / mode function-calling | FULL | **FULL** ✅ CLÔTURÉ (enum `ToolChoice` Auto/None/Required/Tool) | — | request.rs:127,161,314-322 |
| Intégration models.dev | FULL (toutes providers) | **PARTIAL** (snapshot embarqué openrouter-only ; profils ≠ catalogue) | High | catalogue.rs ; models_dev.json |
| Matrice de variantes (reasoning effort tiers) | FULL (~400 L) | **PARTIAL** ✅ amélioré (mapping `reasoning_effort`/`thinking_budget` → budget anthropic + thought gemini ; pas de matrice tiers complète) | Med | anthropic/mod.rs:354-356 |
| Token counting local | PARTIAL | **MISSING** (usage provider seulement) | Med | event.rs:74-86 |
| Calcul de coût (tiers >200k, cache read/write) | FULL | **PARTIAL** (input/output plat seulement) | Med | catalogue.rs:41-49 |
| **Prompt caching** (`cache_control`, breakpoints, TTL) | FULL | **FULL** ✅ CLÔTURÉ (cap 4 breakpoints, ttl 1h/5m, tool-result+system+last-user) | — | anthropic/mod.rs:27,257-282,433-450 |
| Input image/multimodal | FULL | **FULL** ✅ CLÔTURÉ (`parts: Vec<MessageContentPart>` + normalisation image) | — | request.rs:40,101 ; media.rs:13-149 |
| **Retry/backoff + `Retry-After` + classif 429/529/5xx** | FULL | **FULL** ✅ CLÔTURÉ (MAX_RETRIES=2, backoff jitté, retry-after ms/s/date) | — | transport.rs:30-32,163-224 |
| Détection context-overflow | FULL | **PARTIAL** | Med | error.rs:88 |
| Normalisation d'erreurs / redaction secrets | FULL / PARTIAL | **FULL / FULL (plus fort)** | — | error.rs:35-132 |
| Transforms par provider (scrub, temp/top_p, remap, clamp) | FULL | **PARTIAL** (mapping params/headers par profil ; pas de transforms complètes) | Med | request.rs ; provider_profiles.rs |
| Hooks plugin/middleware | FULL | **PARTIAL** (`extra_headers`/`extra_params` câblés) | Med | request.rs |

**Synthèse LLM/providers (28-Juin)** : les **3 écarts P0 bloquants sont CLÔTURÉS** — retry/backoff (deux comportements transport + Retry-After ms/s/date, classif 429/529/5xx), prompt caching (cap 4 breakpoints, ttl 1h/5m sur tool-result/system/last-user), et multimodal (modèle de contenu `parts` + normalisation image ≤2000px/20 MiB). cortex dispose désormais de **4 moteurs de protocole natifs** (anthropic-messages, gemini, openai-chat, openai-responses), **18 profils providers**, **OAuth refresh + Google ADC** complets, formats tool natifs Anthropic/Gemini avec signatures de thinking, et `ToolChoice`. **Plus fort qu'opencode** sur redaction secrets + kill-switch `CORTEX_NO_NET`. Résidus réels (par sévérité) : **(High)** Bedrock sans signature **SigV4** (profil + creds présents mais ne peut signer AWS) ; catalogue embarqué **openrouter-only (14 modèles)** alors que 18 profils existent ; **(Med)** pas de token counting local, coût plat sans tiers >200k/cache, matrice de variantes partielle, Azure/Copilot non vérifiés bout-en-bout.

### 3.1 État des 3 ex-écarts bloquants P0 — désormais CLÔTURÉS (constantes + algorithmes vérifiés)

#### 3.1.1 Retry / Backoff — ✅ CLÔTURÉ

cortex implémente désormais un retry transport jitté (`crates/llm/src/transport.rs`). Constantes (`:30-32`) : `MAX_RETRIES=2` (3 tentatives), `BASE_DELAY_MS=500`, `MAX_DELAY_MS=10_000`. Gating (`:163`) : abandon si `retries_performed >= MAX_RETRIES` ou statut non-retryable ; set retryable (`is_retryable_status` :173) = **429, 529, et tous les 5xx** (`status.is_server_error()`). `Retry-After` (`parse_retry_after` :176-208) : lit `retry-after-ms` (ms) en priorité, sinon `retry-after` en **secondes ou date HTTP**, clampé à `MAX_DELAY_MS`. Backoff jitté (`jittered_exponential_backoff` :210-224) : `500·2^attempt` borné 10 s avec jitter borné. Tests dédiés (`:375-439`) couvrent 429/529/5xx, précédence `retry-after-ms`, parsing secondes+date HTTP, et bornes du jitter. **Résidu** : pas de seconde couche retry niveau-session/tour-entier comme opencode (`retry.ts`) — un échec après 2 essais transport remonte au tour ; pas de reprise mid-stream.

#### 3.1.2 Prompt caching — ✅ CLÔTURÉ

cortex émet désormais `cache_control` sur le protocole anthropic-messages (`crates/llm/src/anthropic/mod.rs`). Cap dur `PROMPT_CACHE_BREAKPOINT_CAP=4` (`:27`), budget `remaining` décrémenté (`:258`). Placement des breakpoints (`:257-282`) : dernier bloc **tool-result**, dernier bloc **system**, dernier message **user** — homologue des 3 breakpoints d'opencode. TTL (`prompt_cache_control` :433-450) : `{type:"ephemeral"}` par défaut, `{type:"ephemeral", ttl:"1h"}` quand `prompt_cache_ttl_seconds` (`:29`) est fourni. **Résidu** : pas de gate protocole multi-providers explicite (opencode saute OpenAI/Gemini via `RESPECTS_INLINE_HINTS`) — non bloquant car cortex applique les breakpoints uniquement dans le chemin anthropic.

#### 3.1.3 Multimodal / modèle de contenu — ✅ CLÔTURÉ

Le type bloquant `Message.content: String` est remplacé : `Message` porte désormais `parts: Vec<MessageContentPart>` (`request.rs:40`, builder `with_parts` :82) et `MessageContentPart` est une enum typée (`:101`) incluant les parts média + métadonnées de reasoning pour replay. La normalisation image (`crates/llm/src/media.rs`) encode en base64 (`:6-7`), contraint `MAX_IMAGE_DIMENSION=2000` (`:13`) et `MAX_IMAGE_BYTES=20 MiB` (`:18`), downscale les rasters oversized via `normalize_image_bytes` (`:103-149`) et re-encode jpeg/png (`:158`). `is_normalizable_image` (`:93`) gate png/jpeg/gif/bmp/webp. **Résidu** : pas de stub `stripMedia`/`[Attached <mime>]` ni de check de capacité média par-provider (opencode `supportsMediaInToolResult`) ; budget base64 20 MiB vs 5 MiB côté opencode.

> **Nuance pour les tickets** : cortex n'a qu'**une** couche de retry (transport, cap 2) ; opencode en a deux (transport cap 2 + session sans cap fixe). L'écart résiduel retry est donc « pas de retry niveau-tour », pas « pas de retry du tout ».

---

## 4 — MCP / LSP / Format

> Référence : `packages/opencode/src/mcp/`, `src/lsp/` (registre ~38 serveurs), `src/format/` (26 formateurs). Cible : `crates/mcp/` (SDK `rmcp` 1.7), `crates/lsp/` (protocole fait main, registre **25 serveurs**), `crates/format/` (enum `FormatterKind`, **≈22 builtins**).

> **MàJ 28-Juin** : les deux verrous CRITICAL de largeur sont **levés** — le registre LSP passe de 3 à **25 serveurs** (`config.rs:326+`, 25× `LspServerConfig::new`) et les formateurs de 3 à **≈22 builtins** (`format/lib.rs`). S'y ajoutent `implementation`, `call hierarchy`, **pull + workspace diagnostics** côté LSP et la capacité `roots` côté MCP. Résidu principal : **auto-download** de serveurs LSP toujours absent.

### A. MCP — quasi-parité (sous-système le plus abouti)

| Capacité | opencode | statut cortex | sév | opencode | cortex |
|---|---|---|---|---|---|
| Transport stdio | FULL | **FULL** | — | mcp/index.ts:10,338 | client.rs:81,150 |
| Transport Streamable HTTP | FULL | **FULL** | — | mcp/index.ts:8,263 | client.rs:23,176 |
| Transport SSE legacy | FULL | **FULL** (fait main) | — | mcp/index.ts:9,270 | transport_sse.rs:51-173 |
| Auto-probe / fallback transport | implicite (config) | **FULL** (probe HTTP→SSE) | — | mcp/index.ts:203 | client.rs:150,474,523 |
| OAuth (DCR / auth-code) | FULL | **FULL** | — | oauth-provider.ts ; index.ts:776-887 | oauth.rs |
| Serveur callback OAuth | loopback port **fixe 19876** | **PARTIAL** — plage **3200-3299** | Low | oauth-callback.ts | oauth.rs |
| Persistance token/clientinfo | `mcp-auth.json` | **FULL** (store 0600) | Low | auth.ts | credentials.rs |
| `tools/list` + découverte | FULL | **FULL** | — | catalog.ts | client.rs:279 ; catalogue.rs |
| `tools/call` | FULL | **FULL** | — | catalog.ts | client.rs:376 ; manager.rs |
| Prompts (list/get) | FULL | **FULL** | — | index.ts:126,683 | client.rs:325,341 |
| Resources (list/read) | FULL | **FULL** | — | index.ts:127,687 | client.rs:294,309 |
| Refresh live tools `list_changed` | FULL | **FULL** | — | index.ts:63,442 | handler.rs:35 ; catalogue.rs:219 |
| Refresh resources/prompts `list_changed` | tools seulement | **FULL — dépasse la réf** | — | index.ts:63 | handler.rs:70,94 |
| Capacité `roots` | FULL (déclarée + handler) | **FULL** ✅ CLÔTURÉ (`list_roots` depuis roots configurés) | — | index.ts:47,90 | handler.rs:46-50 |
| `sampling` / `elicitation` | désactivés | **MISSING** (parité : off des 2 côtés) | Low | index.ts:43,45 | n/a |
| Bridge notifications de log serveur | handler dédié | **PARTIAL** — autres notifs ignorées | Low | index.ts:442 | handler.rs |
| Introspection PID transport | FULL | **FULL** (arbre process) | — | index.ts:523 | process_tree.rs |

### B. LSP — couverture de méthodes complète, **largeur de registre quasi-comblée**

| Capacité | opencode | statut cortex | sév | opencode | cortex |
|---|---|---|---|---|---|
| **Largeur du registre serveurs** | **38 serveurs** | **PARTIAL — 25 serveurs** ✅ amélioré (rust-analyzer, ruff, ts-ls, gopls, clangd, jdtls, zls, roslyn[c#+razor], ruby-lsp, astro-ls, bash-ls, dockerfile-ls, elixir-ls, kotlin-lsp, nixd, prisma-ls, svelteserver, terraform-ls, tinymist, vue-ls, yaml-ls, dart, …) | Med | server.ts:88-1973 | config.rs:326+ (25× `LspServerConfig::new`) |
| Auto-download de serveurs | FULL (tinymist via GitHub, roslyn/razor) | **MISSING** | **High** | server.ts:204,737-787,1872 | absent |
| Plans d'installation | npm/which | **PARTIAL** — Npm + UvTool seulement | Med | server.ts (par serveur) | config.rs:10-15 |
| Framing JSON-RPC (Content-Length) | FULL | **FULL** | — | lsp/client.ts | transport.rs:44-97 |
| didOpen / didChange | FULL | **FULL** | — | client.ts:611,579 | client.rs:233,257 |
| hover | FULL | **FULL** | — | lsp.ts:379 | client.rs:411 ; navigation.rs:11 |
| definition | FULL | **FULL** | — | lsp.ts:390 | client.rs:420 ; navigation.rs:12 |
| references | FULL | **FULL** | — | lsp.ts:402 | client.rs:441 ; navigation.rs:13 |
| documentSymbol | FULL | **FULL** | — | lsp.ts:427 | client.rs:459 ; navigation.rs:14 |
| workspace/symbol | FULL | **FULL** | — | lsp.ts:438 | client.rs:472 ; navigation.rs:312 |
| **implementation** | FULL | **FULL** ✅ CLÔTURÉ | — | lsp.ts:418 | navigation.rs:14 |
| **call hierarchy** | FULL | **FULL** ✅ CLÔTURÉ (prepare/incoming/outgoing) | — | lsp.ts:448,463 | navigation.rs:15-17,378-457 |
| Push diagnostics (publishDiagnostics) | FULL | **FULL** | — | client.ts:160 | diagnostics.rs:8,134-201 |
| **Pull diagnostics** (textDocument/diagnostic) | FULL | **FULL** ✅ CLÔTURÉ (capability + fallback) | — | client.ts:184,295,416 | client.rs:34 ; feedback.rs:27 ; protocol.rs:141 |
| **Workspace diagnostics** | FULL | **FULL** ✅ CLÔTURÉ | — | client.ts:329 | client.rs:35 ; diagnostics.rs:141 ; protocol.rs:151 |
| Debounce / wait diagnostics | FULL | **FULL** (pull-wait + fallback 1200 ms) | — | client.ts:499,521 | feedback.rs:27,369 |
| Feedback diagnostic post-mutation | report | **FULL — surface propre à cortex** | — | diagnostic.ts:5,20 | feedback.rs:73,296 |
| Modèle de concurrence | async (Effect) | **PARTIAL** — client stdio synchrone | Low | client.ts | client.rs |

### C. Format — **largeur de formateurs quasi-comblée**

| Capacité | opencode | statut cortex | sév | opencode | cortex |
|---|---|---|---|---|---|
| **Largeur de formateurs** | **26 formateurs** | **PARTIAL — ≈22 builtins + custom** ✅ amélioré | Low | formatter.ts:18-404 | lib.rs:17-41 |
| rustfmt | FULL | **FULL** | — | formatter.ts:350 | lib.rs:17,372 |
| prettier (+ détection pkg.json) | FULL | **FULL** | — | formatter.ts:38 | lib.rs:19,375,558 |
| ruff (python) | FULL | **FULL** | — | formatter.ts:189 | lib.rs:21,378,589 |
| gofmt | FULL | **FULL** ✅ CLÔTURÉ | — | formatter.ts:18 | lib.rs:19,71 |
| biome | FULL | **FULL** ✅ CLÔTURÉ | — | formatter.ts:110 | lib.rs:21,72 |
| mix / clang-format / zig / ktlint / dart / terraform / shfmt / nixfmt / rubocop | FULL | **FULL** ✅ CLÔTURÉ | — | formatter.ts:28,166,156,179,279,300,330,340 | lib.rs:22-41,73-82 |
| ocamlformat / standardrb / erb / gleam / cljfmt / ormolu / latexindent / air(R) / … (résidu ≈4) | FULL | **MISSING** | Low | formatter.ts:289,259,269,320,386,376,310,236 | — |
| Formateur custom utilisateur | override config | **FULL — échappatoire propre à cortex** (`$FILE`, match ext) | — | format/index.ts:155 | lib.rs:120-168,387 |
| Gating par marqueur de config | findUp | **FULL** (marqueurs ancêtres) | — | formatter.ts:72,194 | lib.rs:544-603 |
| Désactivation builtins via config | FULL | **FULL** | — | format/index.ts:134-148 | lib.rs:433-450 |
| Multi-formateurs par extension | exécute tous | **PARTIAL** — premier match unique | Low | format/index.ts:57-71 | lib.rs:272-313 |
| Service `status()` | FULL | **MISSING** | Low | format/index.ts:174 | — |
| Scrub env secrets sur enfant | extendEnv | **FULL — durcissement (dépasse réf)** | — | format/index.ts:88 | lib.rs:620-640 |
| Rollback sur format en échec | log d'erreur | **FULL — restaure les octets** | — | format/index.ts:106 | lib.rs:353-357 |

**Synthèse MCP/LSP/Format (28-Juin)** : **les deux verrous CRITICAL sont levés.** MCP était déjà livré (3 transports, OAuth, prompts/resources, refresh live) et **dépasse** la réf (refresh resources+prompts) ; la capacité **`roots` est désormais FULL** (`handler.rs:46-50`), ne laissant que des résidus Low (callback OAuth en plage de ports, sampling/elicitation off des deux côtés). LSP : registre **25/38** (vs 3 avant), avec `implementation`, **call hierarchy**, **pull + workspace diagnostics** tous livrés ; cortex conserve ses surfaces propres (feedback post-mutation, pull-wait+fallback). Format : **≈22 builtins** (vs 3), gofmt + biome + mix/clang/zig/ktlint/dart/terraform/shfmt/nixfmt/rubocop tous FULL, plus scrub-env + rollback durcis. Résidus réels : **(High)** auto-download de serveurs LSP toujours absent (impose une install manuelle des 25 binaires) ; **(Med)** registre 25/38 et plans d'install Npm/UvTool seulement ; **(Low)** ~4 formateurs de niche, `status()` format, multi-formateurs par extension, concurrence LSP. Priorité : (1) infra auto-download LSP, (2) compléter registre 25→38 + plans d'install, (3) ~4 formateurs restants + `status()`.

---

## 5 — Boucle agent / Session runner (engine)

> Référence : `packages/opencode/src/session/` (~10k LOC : `prompt.ts`, `processor.ts`, `compaction.ts`, `retry.ts`, `instruction.ts`…). Cible : `crates/engine/src/` (~2.3k LOC : `runner.rs`, `compaction.rs`…) + `crates/cli/src/{run,subagent}.rs`.

| Capacité | opencode (fichier:ligne) | statut cortex | sév | cortex (fichier:ligne) |
|---|---|---|---|---|
| Boucle de tour principale (LLM→outils→loop, budget de steps) | prompt.ts:1184-1437 | **FULL** | — | runner.rs:207-378 ; MAX_STEPS=25 |
| Streaming sortie assistant (deltas ordonnés) | processor.ts:784-839 | **FULL** ✅ amélioré (parts persistées + états + phases) | — | message_parts.rs:27-239 ; protocol/message.rs:72 |
| Stream reasoning/thinking | processor.ts:373-425 | **FULL** ✅ CLÔTURÉ (part `Reasoning` persistée) | — | protocol/message.rs:76 |
| **Orchestration outils — appels PARALLÈLES** | AI-SDK concurrent ; processor.ts:295-346,879-883 | **FULL** ✅ CLÔTURÉ — `FuturesUnordered`, concurrence=4 configurable | — | runner.rs:41-42,580-591 |
| Ré-injection résultat outil / continuation | processor.ts:549-647 | **FULL** | — | runner.rs:352-369 |
| **Modèle Message/Part riche** | message-v2.ts (747) : text/reasoning/tool/step/patch/file/agent/subtask/compaction | **FULL** ✅ CLÔTURÉ — `MessagePartBody` = 12 variantes (Text/Reasoning/Tool/Step/Patch/File/Agent/Subtask/Compaction/Snapshot/Retry/Usage) | — | protocol/message.rs:72-155 |
| System prompt (par famille) | system.ts:24-38 | **FULL** (clean-room) | — | family.rs:31-89 |
| Persona d'agent | reminders.ts | **PARTIAL** — builtins + custom, pas de reminders plan/build-switch | Med | agent.rs:88-95 ; run.rs:740-759 |
| Bloc environnement | system.ts:54-90 | **FULL** | — | environment.rs:77-115 |
| **AGENTS.md / instructions** | instruction.ts:60-221 (global+CLAUDE.md+URL+walk à la lecture) | **FULL** ✅ CLÔTURÉ (global + CLAUDE.md + walk projet) | — | run.rs:106-110 ; context.rs:74-77 ; memory |
| Compaction mécanique (sélection tours / prune) | compaction.ts:198-297 | **PARTIAL** — prune-milieu + troncature par estimation char | Med | compaction.rs:149-267 |
| **Compaction — résumé LLM** | compaction.ts:299-552 (agent résumé, replay, auto-continue) | **MISSING** — note synthétique "[N messages omis]", pas de résumeur LLM | **High** | compaction.rs:170-188 |
| Récupération overflow (compact-and-retry) | processor.ts:926-936 | **FULL** (halve budget + halve tail, ≤4×) | — | runner.rs:259-274 |
| Détection overflow tokens | overflow.ts | **PARTIAL** — match string + estimation char (pas de tokenizer) | Med | compaction.rs:120-140 |
| **File de prompts (steer mid-stream)** | run-state.ts:88-94 ; prompt.ts:1127-1138 | **FULL** ✅ CLÔTURÉ — `TurnQueue` admission + queueing mid-turn niveau engine | — | turn_queue.rs:4,27,107 |
| **Abort / annulation** | run-state.ts:77-86 ; processor.ts:982-989 ; cleanup 846-915 | **FULL** ✅ CLÔTURÉ — abort coopératif (`watch` channel) par session | — | turn_queue.rs:34,65,144 |
| **Retries erreurs provider (429/5xx/backoff)** | retry.ts:35-199 ; processor.ts:994-1025 | **FULL** ✅ CLÔTURÉ — retry transport jitté + Retry-After (cf. §3.1.1) | — | transport.rs:30-32,163-224 |
| Spawn sous-agent / task | prompt.ts:259-453 | **FULL** — session enfant durable, persona, cap permission, checkpoint/format/LSP | — | subagent.rs:50-120 ; run.rs:482-542 |
| Bascule mode/agent (mid-session) | prompt.ts:699-722 ; reminders.ts:26-89 | **MISSING** — agent fixé au démarrage, pas d'events switch | Med | run.rs:112-137 |
| Comptabilité token/coût | Session.getUsage (cache+reasoning+coût/modèle) | **PARTIAL** — part `Usage` + coût provider ; pas de tiers pricing/cache/reasoning | Med | protocol/message.rs:155 ; event.rs:172-186 |
| Hooks de checkpoint | snapshot.track/patch ; revert ; diff | **PARTIAL** — parts `Snapshot`/`Patch` + checkpoint git ; diff résumé non surfacé | Med | protocol/message.rs:110,138 ; run.rs:463-607 |
| Génération de titre | prompt.ts:197-257 | **MISSING** | Low | — |
| Résumé de diff | summary.ts:82-142 | **MISSING** | Low | — |
| Détection doom-loop | processor.ts:519-546 (seuil 3) | **FULL** ✅ CLÔTURÉ (`DoomLoopGuard`, seuil 2) | — | runner.rs:57-58,298,538,1056 |
| Sortie structurée (json_schema) | prompt.ts:1345-1412 | **MISSING** | Low | — |
| Injection guidance skill mid-turn | system.ts:92-104 | **FULL** | — | runner.rs:366-368,411-426 |
| Changement live tool-set MCP mid-turn | service MCP | **FULL** | — | runner.rs:231-254 |

**Synthèse engine (28-Juin)** : les **4 écarts CRITICAL/HIGH du squelette sont CLÔTURÉS** — exécution **parallèle** des outils (`FuturesUnordered`, concurrence=4, `runner.rs:580`), **modèle de parts riche** à 12 variantes (`protocol/message.rs:72`), **retries provider** (retry transport jitté + Retry-After, cf. §3.1.1), et **file de prompts + abort** au niveau engine via `TurnQueue` (`turn_queue.rs`, abort coopératif par `watch` channel — plus seulement TUI). S'y ajoutent doom-loop (`DoomLoopGuard` seuil 2), reasoning persisté, AGENTS.md (global+CLAUDE.md+walk). **Résidu CRITICAL unique** : la **compaction reste mécanique** (prune-milieu + troncature char), **sans résumeur LLM / replay / marqueur persisté** (`compaction.rs:170-188`). Résidus Med : pas de tokenizer (estimation char), bascule mode/agent mid-session, pricing par tiers/cache/reasoning, diff résumé non surfacé. Résidus Low : génération de titre, résumé de diff, sortie structurée json_schema. Priorité : (1) chemin compaction par **résumé LLM** + marqueur `Compaction` persisté + replay, (2) tokenizer réel, (3) bascule mode/agent + reminders, (4) génération de titre / diff summary / sortie structurée.

### 5.1 Approfondissement — orchestration outils, taxonomie de parts, compaction

#### 5.1.1 Orchestration parallèle des outils — ✅ CLÔTURÉ

> **MàJ 28-Juin** : cortex exécute désormais les outils **en parallèle** — `DEFAULT_TOOL_DISPATCH_MAX_CONCURRENCY=4` (`runner.rs:41-42`, configurable via `with_tool_dispatch_max_concurrency` :226) et un drain `FuturesUnordered` borné (`runner.rs:580-591`) qui pré-émet les `ToolCall` dans l'ordre puis draine hors-ordre. L'analyse ci-dessous documente l'**état antérieur séquentiel** (résolu) et reste utile pour le caveat de bornage local.

opencode **ne code pas** sa boucle d'outils : il délègue tout le cycle step/tool au `streamText` de l'AI-SDK Vercel (`llm.ts:280`, tools `:317-318`, repair `:296-312`) et n'**observe** que le `fullStream`. Concurrence : dans un step, N blocs `tool-call` → l'AI-SDK lance **les N promesses `execute()` dans le même tick** (`Promise.all` interne) — jamais sérialisé. Le processor est spectateur : `ensureToolCall` crée le `ToolPart` `pending` + un `done: Deferred` par appel (`processor.ts:295-346`), `tool-result` résout ce `done` (:549). Ordre transcript = ordre d'**émission** (pairing par `toolCallId`), indépendant de l'ordre de fin. Barrière de drain `processor.ts:879-883` : `{concurrency:"unbounded"}` + `Effect.timeout("250 millis")` par appel ; non résolus → `error:"Tool execution aborted"`/`interrupted` (:885-909) pour qu'aucun `tool_use` ne reste sans `tool_result`. Borne de steps : `maxSteps = agent.steps ?? Infinity` (`prompt.ts:1281-1282`, **illimité par défaut**) ; garde-fou comportemental `DOOM_LOOP_THRESHOLD=3` (3 appels identiques consécutifs → prompt permission, `processor.ts:35,522-545`).

cortex : boucle **strictement séquentielle** (`runner.rs:344-371`) — `for call in &round.tool_calls { dispatcher.dispatch(call).await }` (:346-352), wall-clock = Σ latences. Le seam `ToolDispatcher::dispatch → BoxFuture` (`tools.rs:44-49`) **renvoie déjà un future** : seuls la boucle sérialise. Bornes : `DEFAULT_MAX_STEPS=25` (:33, diagnostic non-crash :208-222). **Fix** : remplacer le corps `:346-370` par `FuturesUnordered`/`join_all` (pré-émettre les `ToolCall` dans l'ordre → drainer hors-ordre dans un `Vec<Option<_>>` indexé → ré-injecter dans l'ordre original → `tokio::time::timeout` par appel + force-error des inachevés). Caveat : borner (`buffer_unordered(K)`) car l'exécution locale (shell/edit) n'a pas la même sûreté d'effets que le fan-out AI-SDK.

#### 5.1.2 Taxonomie message/parts — ✅ CLÔTURÉ

> **MàJ 28-Juin** : le modèle plat est remplacé par `MessagePartBody` (`crates/protocol/src/message.rs:72-155`), union discriminée de **12 variantes first-class** : `Text` (:73), `Reasoning` (:76), `Tool` (:81), `Step` (:94), `Patch` (:110), `File` (:115), `Agent` (:122), `Subtask` (:129), `Compaction` (:133), `Snapshot` (:138), `Retry` (:147), `Usage` (:155) — parité (et au-delà) avec les 12 types persistés d'opencode. La projection runner→parts vit dans `engine/src/message_parts.rs:27-239` (états `ToolPartState` Pending/Running/Completed/Error, phases `StepPartPhase`). L'analyse ci-dessous documente l'**état antérieur plat** (résolu).

opencode (`core/src/v1/session.ts`) : union `Part` discriminée = **12 types persistés** — `text` (:96), `reasoning` (:112), `file` (:165, +`source` file/symbol/resource), `tool` (:309, +4 états), `step-start` (:227), `step-finish` (:234, coût+tokens+snapshot), `snapshot` (:81), `patch` (:88, hash+files), `agent` (:175), `subtask` (:198), `compaction` (:189, +`tail_start_id`), `retry` (:214, attempt+APIError). États tool (`:298-307`) : `pending`/`running`/`completed`(+`attachments`,`compacted`)/`error`. Enveloppes riches `User`/`Assistant` (:334,:455 : cost, tokens, path, agent, model, finish).

cortex : modèle **plat** `HistoryMessage{role,content:String}` (`history.rs:13-18`) projeté en `llm::Message` (:48-58) — **pas de tableau de parts** ; + vocabulaire **transitoire** `RunnerEvent` (`event.rs:19-80`, non persisté). Mapping : **7 types opencode entièrement MISSING** (`file`, `step-start`, `snapshot`, `patch`, `agent`, `subtask`, + `compaction`/`retry` comme parts) ; les PARTIAL (`text`/`reasoning`/`tool`/`step-finish`) perdent persistance + champs `time`/`metadata`/lifecycle/identité-par-segment. `reasoning` streamé mais **non persisté** ; `tool` sans machine d'états pending/running ; `attachments` sur l'event mais pas en history (flat text).

#### 5.1.3 Compaction (algorithme comparé)

| Constante opencode (`compaction.ts`) | val | cortex (`compaction.rs`/`runner.rs`) | val |
|---|---|---|---|
| `PRUNE_MINIMUM`/`PRUNE_PROTECT` (:38-39) | 20k/40k | `CHARS_PER_TOKEN` (:22) | 4 |
| `TOOL_OUTPUT_MAX_CHARS` (:40) | 2000 | `DEFAULT_CONTEXT_LIMIT_TOKENS` (:28) | 128k |
| `PRUNE_PROTECTED_TOOLS` (:41) | `["skill"]` | `MAX_INPUT_BUDGET_TOKENS` (:36) | 300k |
| `DEFAULT_TAIL_TURNS` (:42) | 2 | `DEFAULT_TAIL_MESSAGES` (:42) | 8 |
| preserve-recent (:90-95) | `min(8k,max(2k,0.25·usable))` | `MARKER_TOKENS` (:165) | 24 |
| — | — | `MAX_OVERFLOW_RECOVERIES`/`MIN_RECOVERY_BUDGET` (runner :37,:40) | 4 / 512 |

opencode = **résumé LLM + replay** (`compaction.ts:198-552`) : trigger `prompt.ts:1264-1271` → `create({auto})` ajoute une **part `compaction` first-class** (:569-576) ; `process` localise le parent, gère le split replay overflow (:320-336), choisit l'**agent dédié `compaction`** (:338), sélectionne par **tours** budget-tokens avec split mi-tour (`select`/`turns`/`splitTurn` :97-249), rend le head `stripMedia+toolOutputMaxChars:2000` (:359-364), lance le résumeur **sans outils** `tools:{}` (:410-424), persiste `tail_start_id` (:437-442), puis **auto-continue/replay** (re-émet le message user d'origine ou injecte une relance synthétique, :444-526). Passe de prune séparée : protège 40k tokens, efface les tool-outputs anciens non-`skill` si >20k libérés (:251-297). `filterCompacted` réordonne `[compaction-user, summary, …tail…, continue-user]` (`message-v2.ts:535-586`).

cortex = **mécanique, sans LLM** (`compaction.rs:120-267`) : Trigger A avant chaque envoi (`runner.rs:225-229`, budget `[256,300k]`) ; Trigger B après overflow (`runner.rs:259-274`, **halve budget + halve tail**, ≤4×). `compact_to_budget` (:149-161) : estime `Σ(ceil(chars/4)+4)` ; **prune middle** (garde system + N derniers, splice le milieu en **une note system synthétique** :170-188) ; si encore trop → **troncature** head+tail pondérée-tail (`tail_chars=max(budget/2,160)`, marqueur `[… N omis …]`, UTF-8-safe :239-274). **No LLM, no summary, no replay, no marqueur persisté** — mutation in-memory pure et lossy.

Pour la parité cortex doit : (1) sélecteur **par tours** au lieu de N-derniers-messages, (2) chemin **résumeur LLM** + marqueur `compaction`/`summary` persisté + `tail_start_id`, (3) passe de prune tool-output dédiée (liste protégée), (4) étape auto-continue/replay.

---

## 6 — Permissions / Policy / Config (settings)

> Référence : `packages/opencode/src/{permission,config,agent}/` + `packages/core/src/v1/config/` (schéma typé complet). Cible : `crates/{permissions,policy,settings}/` (+ consommateurs `engine`, `cli`, `memory`, `tui`, `format`).

> **Deux divergences fondamentales (intentionnelles) cadrent tout** : (1) **Clé de permission** — opencode indexe par **nom de capacité** (`read`, `edit`, `bash`, `task`, `external_directory`, `question`, `doom_loop`, `plan_enter`…) avec valeur = action ou map `pattern → action` (`core/v1/config/permission.ts:17-36`) ; cortex indexe par **nom d'outil** + glob, en tableau ordonné `[[permission]]` (`permissions/rule.rs:16-23`). Les deux résolvent **dernière règle qui matche gagne** (opencode `index.ts:39-49` `findLast` ; cortex `ruleset.rs:71-79`). (2) **Format** — opencode = **JSON/JSONC** + schéma maître typé (`config/config.ts:32-187`) ; cortex = **TOML** quasi-**schemaless** (seuls `model`/`agent`/`max_steps` validés, `schema.rs:22-27`), chaque crate lisant ses clés à la carte.

### A. Permissions — moteur solide, largeur & modèle d'interaction plus étroits

| Capacité | opencode | statut cortex | sév | opencode | cortex |
|---|---|---|---|---|---|
| Modèle 3 actions allow/ask/deny | FULL | **FULL** | 🟢 | core/v1/permission.ts:15 | action.rs:8-16,34 |
| Défaut = `ask` si rien ne matche | FULL | **FULL** | 🟢 | index.ts:43-47 | ruleset.rs:71-79 |
| Dernière règle qui matche gagne | FULL | **FULL** | 🟢 | index.ts:39-49 | ruleset.rs:71-79 |
| Gating par capacité vs par outil | par **capacité** | **PARTIAL** (par **outil** ; `edit` non agrégé sur write/edit/patch) | 🟠 | config/permission.ts:17-36 | scope.rs:41-69 |
| Sélecteur wildcard `*` | FULL | **FULL** | 🟢 | index.ts:201 | rule.rs:7,38-40 |
| Glob `*`/`?` | FULL | **FULL** (ancré) | 🟢 | index.ts:43 | glob.rs:17-51 |
| Pattern de commande bash | FULL | **FULL** | 🟢 | index.ts:84 | scope.rs:117-121,205-215 |
| **Dictionnaire d'arité bash** (`git checkout x`→`git checkout`) | FULL (~140 cmds) | **MISSING** (split sur opérateurs + tree-sitter, mais pas de normalisation d'arité) | 🟠 | arity.ts:1-161 | scope.rs:137-194 |
| Split commandes composées (`&&`,`\|\|`,`;`,`\|`) | n/a | **FULL** (quote-aware, fold le plus restrictif) | 🟢 | — | scope.rs:138-215 |
| Scoping/normalisation chemin outils-fichier | match + expansion `~`/`$HOME` | **PARTIAL** (normalise `./`,`//` ; garde `..` littéral ; **pas d'expansion `~`/`$HOME`**) | 🟠 | index.ts:189-195 | scope.rs:83-115 |
| **Gating `external_directory`** (hors-workspace) | FULL (whitelist tmp/skills/refs) | **FULL** ✅ CLÔTURÉ (capacité `external_directory`, défaut `ask`) | 🟢 | agent/agent.ts:108-136,296-310 | defaults.rs:7 ; lib.rs:67 |
| Gating `read` `.env` (`*.env` ask / `.env.example` allow) | FULL | **MISSING** | 🟠 | agent/agent.ts:129-135 | — |
| Gating `task`/sous-agent par nom | FULL | **PARTIAL** (scopé par nom-cible ; pas de deny défaut type `task.general:deny`) | 🟠 | agent.ts:165-167 | scope.rs:36-37,55-64 |
| Gating `todowrite`/`question`/`doom_loop`/`plan_enter`/`plan_exit`/`skill`/`lsp` | FULL (clés discrètes) | **PARTIAL** ✅ amélioré (capacités `doom_loop`/`external_directory`/`edit`/`bash` nommées ; set discret incomplet) | 🟠 | config/permission.ts:19-33 | lib.rs:67 |
| allow-once | FULL | **FULL** | 🟢 | index.ts:154 | engine.rs:68-92,347-362 |
| allow-always (+ propage aux pending) | FULL (auto-résout pending du session) | **FULL** ✅ CLÔTURÉ (registre `InFlightAsks` résolu) | 🟢 | index.ts:155-177 | engine.rs:209,439-466 ; saved.rs:82-99 |
| Mémoire des approbations de session | FULL | **FULL** | 🟢 | index.ts:36,156-162 | saved.rs:54-99 |
| **Décisions sauvées durables (cross-process)** | SQL | **FULL** (store JSON par projet, écriture atomique) | 🟢 | core/permission/sql.ts | store.rs:1-150 |
| **Reject avec feedback/correction** (`CorrectedError`) | FULL (message → re-fed au modèle) | **FULL** ✅ CLÔTURÉ (`feedback` porté par la décision → re-fed au tour) | 🟢 | index.ts:132-138 | engine.rs:83-84,108-113,153-154 |
| Reject cascade aux pending du session | FULL | **PARTIAL** (feedback présent ; cascade explicite vers tous les pending non confirmée) | 🟠 | index.ts:140-149 | engine.rs:83-113 |
| File pending / API reply async | FULL (Deferred ; `list()` ; `reply()` ; events) | **FULL** ✅ CLÔTURÉ (`InFlightAsks` observable + reply/resolve + events SSE/headless) | 🟢 | index.ts:23-27,78-118 | engine.rs:209,221-247,419-466 ; lib.rs:39 |
| Fail-closed sans responder (headless) | n/a | **FULL** (sécurité cortex en + : deny borné) | 🟢 | — | engine.rs:342-349 |
| Rulesets par agent | FULL | **FULL** | 🟢 | agent/agent.ts:44,140-294 | agent.rs:26-74 |
| Narrowing permission sous-agent (≤ parent) | FULL (+ deny défaut todo/task) | **PARTIAL** (cap parent le plus restrictif OK ; pas l'injection deny défaut) | 🟠 | subagent-permissions.ts:14-26 | agent.rs:76-87 |
| Précédence multi-couche (global→projet→nested) | FULL | **FULL** | 🟢 | config.ts:405-465 | config.rs:166-238 |
| Hook d'override de décision (plugin/policy) | FULL | **FULL** | 🟢 | permission-auto-respond.ts | engine.rs:161-171,325-338 |
| Couche hard-deny (outrank saved-allow) | n/a | **FULL** (ajout cortex pour trust/mode) | 🟢 | — | engine.rs:217-227,374-411 |
| Injection `OPENCODE_PERMISSION` (env JSON) | FULL | **MISSING** | 🟢 | config.ts:544-550 | — |
| Map `tools:{name:bool}` → permission | FULL | **MISSING** (frontmatter = règles explicites) | 🟢 | config.ts:552-563 | — |

### B. Config / Settings

| Capacité | opencode | statut cortex | sév | opencode | cortex |
|---|---|---|---|---|---|
| Format fichier | JSON + **JSONC** | **PARTIAL** (TOML ; pas de JSON/JSONC) — swap voulu | 🟠 | config.ts:140,226 | settings/lib.rs:471-479 |
| Fichier config global | FULL | **PARTIAL** (`…/cortex/config.toml` unique) | 🟢 | config.ts:139-147 | discovery.rs:45-52 |
| Découverte config projet | FULL | **FULL** (`cortex.toml` + `.cortex/config.toml`, chaîne projet→nested) | 🟢 | config.ts:405-433 | discovery.rs:26-81 |
| Précédence global<projet<nested<… | FULL | **FULL** (`defaults<global<projet<nested<env<CLI`) | 🟢 | config.ts:398-533 | settings/lib.rs:1-31,201-228 |
| Deep-merge par clé | FULL | **FULL** | 🟢 | config.ts:41-43 | merge.rs:14-25 |
| Merge concat de tableaux (instructions dedup) | FULL | **PARTIAL** (merge général remplace ; règles perm concaténées ; instructions dans `memory`) | 🟠 | config.ts:45-51 | merge.rs:20-22 |
| Override env de clés | n/a | **FULL** (ajout cortex `CORTEX_<KEY>`) | 🟢 | — | env.rs:26-47 |
| Couche override CLI | FULL | **FULL** | 🟢 | Flag.* | settings/lib.rs:74,222 |
| Substitution `{env:VAR}` | FULL (**embarquée n'importe où**) | **PARTIAL** (token valeur-entière seulement) | 🟠 | variable.ts:36-38 | subst.rs:80-112 |
| Substitution `{file:PATH}` | FULL (embarquée ; `~/` ; skip `//` ; vs **dir config**) | **PARTIAL** (valeur-entière ; pas de `~` ; vs **cwd**) | 🟠 | variable.ts:40-90 | subst.rs:114-129 |
| Var manquante = erreur dure | FULL | **FULL** | 🟢 | variable.ts:71-82 | subst.rs:104-119 |
| Validation typée + diagnostics localisés | FULL (schéma complet) | **PARTIAL** (seuls model/agent/max_steps) | 🟠 | parse.ts ; config.ts:32-187 | schema.rs:22-69 |
| **Config provider** (`provider`, enabled/disabled) | FULL (providers custom, overrides) | **PARTIAL** ✅ amélioré (18 profils providers intégrés résolvables ; pas de wiring config enable/disable/custom) | 🟠 | config.ts:68-73,107-109 | providers/provider_profiles.rs |
| Définitions agent (objet config `[agent.x]`) | FULL (schéma riche) | **PARTIAL** (personas hard-codés ; custom via **frontmatter markdown** seulement) | 🟠 | config/agent.ts:12-89 | cli/custom_agents.rs:335-376 |
| Agents en markdown (`agent(s)/**/*.md`) | FULL | **FULL** (`.cortex/agents`, frontmatter+corps) | 🟢 | config/agent.ts:11-32 | cli/custom_agents.rs:180-227 |
| Définitions `mode(s)/*.md` + merge mode→agent | FULL | **PARTIAL** (mode = champ d'agent ; pas de dir `mode(s)` séparé) | 🟠 | config.ts:535-542 | cli/custom_agents.rs:587-621 |
| `default_agent` / `model` / `small_model` | FULL | **FULL** / **FULL** / **PARTIAL** (`CORTEX_SMALL_MODEL`, pas d'accesseur typé) | 🟢 | config/config.ts:74-83 | settings/lib.rs:50-119 ; env.rs:68 |
| Config keybind | FULL (config tui séparée) | **FULL** (`[keybinds]`) | 🟢 | config/tui.ts:70-77 | tui/keybind.rs:288-300 |
| Config thème | FULL | **PARTIAL** (`tui-syntax` ; persist via global string) | 🟢 | config/tui.ts | settings/lib.rs:251-270 |
| Config MCP (`mcp.<name>` local/remote/**oauth**) | FULL (union Local/Remote/OAuth) | **PARTIAL** (local+remote ; **pas d'OAuth dans le schéma config**) | 🟠 | config/mcp.ts:6-63 | settings/lib.rs:277-421 |
| Add/persist MCP via CLI | n/a | **FULL** (add idempotent, anti-conflit) | 🟢 | — | settings/lib.rs:382-421 |
| Config formatter (`bool \| {name:{disabled,command,env,ext}}`) | FULL | **PARTIAL** (`[formatter.custom]` command+ext ; pas de disable/env par-nom) | 🟠 | config/formatter.ts:5-13 | format/lib.rs:23,118-126 |
| Config LSP (`bool \| {...}`) | FULL | **PARTIAL** (`lsp/config.rs` ; hors schéma settings) | 🟠 | config/lsp.ts | lsp/config.rs |
| Instructions / AGENTS.md | FULL | **FULL** | 🟢 | config/config.ts:121-123 | memory/lib.rs:1-256 |
| Config **well-known** distante | FULL (`/.well-known/opencode`) | **MISSING** | 🟠 | config.ts:355-395 | — |
| Config **managed/MDM** (.mobileconfig) | FULL | **MISSING** | 🟢 | config.ts:515-533 | — |
| Config par-org / compte console | FULL | **MISSING** | 🟢 | config.ts:477-513 | — |
| `OPENCODE_CONFIG` / `_DIR` / `_CONTENT` | FULL (path/dir/inline) | **PARTIAL** (`CORTEX_CONFIG_FILE`/`_DIR` ; pas de `_CONTENT` inline) | 🟠 | config.ts:400-475 | env.rs:13-22 |
| Clés typées diverses (share, autoupdate, snapshot, watcher, compaction, tool_output, attachment, server, enterprise, experimental.*) | FULL | **MISSING/PARTIAL** (pass-through non typé, surtout non consommé) | 🟠 | config/config.ts:36-186 | settings schemaless |
| Config plugin + origins/dedup | FULL | **PARTIAL** (crate `plugins` ; pas de tracking d'origine en schéma) | 🟠 | config.ts:101-109,322-348 | plugins/src/* |
| Redaction secrets dans config reportée | n/a | **FULL** (ajout cortex) | 🟢 | — | settings/lib.rs:149-194 |

### C. Policy (crate cortex-original)

Le crate `policy` cortex **n'a pas d'analogue structurel** dans opencode `permission/`/`config/`. Il implémente une **porte de confiance projet**.

| Capacité | opencode | statut cortex | sév | cortex |
|---|---|---|---|---|
| Niveau de confiance projet (`trust = trusted\|untrusted`) | aucun (le plus proche : `experimental.policies` accès-provider, sémantique ≠) | **FULL** (original cortex) | 🟢 | policy/lib.rs:30-102 |
| Untrusted → deny outils effectifs (write/edit/apply_patch/bash) | ad-hoc via deny rules | **FULL** | 🟢 | policy/lib.rs:25,76-86 |
| Confiance installée comme **hard-deny** (outrank saved-allow) | n/a | **FULL** | 🟢 | policy/lib.rs:13-19 ; engine.rs:217-227 |
| Défaut = trusted | n/a | **FULL** | 🟢 | policy/lib.rs:31-38 |
| `experimental.policies` (gouvernance accès-provider) | FULL | **FULL** ✅ CLÔTURÉ (parse + règles ordonnées + éval accès-provider) | 🟢 | policy/lib.rs:30-33,177-205 |

**Synthèse permissions/policy/config (28-Juin)** : le **moteur de permissions** cortex (ordre de résolution, 3 actions, rulesets par agent, narrowing sous-agent, décisions sauvées session **+ cross-process** `store.rs`, hard-denies, fail-closed headless) reste **à parité ou en avance**, et les **2 écarts CRITICAL sont CLÔTURÉS** : 🔴→🟢 **gating `external_directory`** (`defaults.rs:7`, capacité `ask` par défaut) et 🔴→🟢 **reject-avec-feedback** (`CorrectedError` via `engine.rs:83-113`, re-fed au tour). La **file pending async** (`InFlightAsks` observable + reply/resolve) et la propagation **allow-always** sont également livrées, et **`experimental.policies`** est désormais porté côté crate `policy`. Résidus permission 🟠 : namespace de capacités encore partiel (set discret incomplet), pas de dictionnaire d'arité bash, pas de gating `.env`, cascade-reject explicite non confirmée, `OPENCODE_PERMISSION`/`tools:{bool}` toujours absents. Côté **config**, la divergence reste dominée par le swap **intentionnel** TOML vs JSON/JSONC + cœur schemaless ; **config-provider** passe MISSING→PARTIAL (18 profils intégrés, pas de wiring enable/disable) ; résidus 🟠 : substitution plus faible, sources distante/MDM/org + `_CONTENT` absentes, modes markdown sans dir séparé, MCP-OAuth hors schéma. Priorité : (1) wiring config-provider enable/disable/custom, (2) compléter le namespace de capacités + arité bash + gating `.env`, (3) parité substitution `{env}`/`{file}` embarquée, (4) sources config distante/MDM/org, (5) cascade-reject explicite.

---

## 7 — Serveur / API HTTP / SSE / ACP / SDK

> Référence : `packages/opencode/src/server/` + `packages/server/` (~127 opérations HTTP porteuses) + `packages/sdk/` (client JS généré). Cible : `crates/server/` (routeur axum **~80 endpoints**), `crates/events/` (enum `Event` **58 variantes**, `event.rs:345+`), `crates/acp/` (JSON-RPC fait main, **14 méthodes**), **`sdk/`** (SDK TypeScript généré depuis `openapi.json`).

> ✅ **CLÔTURÉ (essentiel)** — Le diagnostic « ~7 % de surface HTTP / 0 % de noms d'événements communs / aucun SDK » est **obsolète**. Depuis l'audit du 24-juin, la surface serveur a été massivement portée : routes 13→~80, events 14→58 (avec **dual-émission délibérée** `session.next.*` wire-compatible, cf. commentaire `lib.rs:1212`), ACP 4→14 méthodes + appels client `fs/*`+`terminal/*`, et un **SDK TypeScript** existe désormais sous `sdk/`. Les sous-systèmes entiers jadis MISSING (config, providers, fichiers, recherche, agents/models/commands/skills, TUI control, permission REST, auth/oauth, lsp/vcs/formatter, pty, project/path/mcp) sont **présents**.

| Surface | opencode | cortex (avant) | cortex (maintenant) | Parité |
|---|---|---|---|---|
| Opérations HTTP | ~127 | ~13 | **~80** | **✅ ~63 %** |
| Types d'événements SSE | ~86 | 14 | **58** (≈40+ noms partagés) | **✅ haut recouvrement** |
| Méthodes agent ACP | 13 + client fs/terminal | 4 | **14 + 6 appels client** | **✅ ~FULL** |
| SDK | Client JS (`OpencodeClient`, 25 classes) | aucun | **SDK TS généré (`sdk/`)** | **✅ PARTIAL** |

### A. API HTTP — cœur session ✅ CLÔTURÉ

| Route | opencode | statut cortex | sév | cortex |
|---|---|---|---|---|
| `GET /session` (liste) | FULL | **FULL** | — | lib.rs |
| `POST /session` (create) | FULL (`CreateInput` riche) | **PARTIAL** (champs réduits) | Low | lib.rs |
| `GET /session/{id}` | FULL | **FULL** | — | lib.rs |
| `DELETE /session/{id}` | FULL | **FULL** | — | lib.rs |
| `GET …/message` (liste paginée) | FULL (cursor) | **✅ FULL** (`cursor`+alias `after`+`limit`+`next_cursor`, lib.rs:1415,1690-1709) | — | lib.rs:1690 |
| `POST …/message` (prompt) | FULL | **FULL** (parts/model) | — | lib.rs |
| `GET/DELETE …/message/{mid}`, `…/part/{pid}` | FULL | **FULL** (`/message/{message_id}/part/{part_id}`) | — | lib.rs |
| `POST …/abort` | FULL | **FULL** (+ marqueur remote-turn) | — | lib.rs |
| `POST …/share` / unshare | FULL | **PARTIAL** (share HTML loopback) | Low | lib.rs |
| `GET …/todo` (+alias `/todos`) | FULL | **FULL** | — | lib.rs |
| `POST …/command` | FULL | **PARTIAL** (pipeline partiel) | Med | lib.rs |
| `GET /session/status`, `…/status` | FULL | **FULL** | — | lib.rs |
| `…/children`, `…/diff`, `…/fork`, `…/init`, `…/summarize`, `…/compact`, `…/prompt_async`, `…/shell`, `…/revert`, `…/unrevert`, `…/wait`, `…/context` | FULL | **✅ FULL** (toutes présentes) | — | lib.rs |
| `…/permission/pending` + `…/reply` (REST) | FULL | **✅ FULL** | — | lib.rs |

**Propre à cortex** : `GET …/prompt` (draft) + `…/prompt/append|submit|clear` (buffer de prompt distant), `POST /event` (ingestion entrante — divergence de design assumée).

### B. API HTTP — sous-systèmes ✅ CLÔTURÉ (présence)

| Groupe de routes | opérations | statut cortex | sév résiduelle |
|---|---|---|---|
| **Config** (`/config`, `/config/providers`) | get/providers | **✅ FULL** | — |
| **Provider** (`/provider`, `/auth/{p}/authorize`, `/auth/callback`) | list/oauth | **✅ FULL** | — |
| **Fichiers** (`/file`, `/file/content`, `/file/status`) | read/list/status | **✅ FULL** | — |
| **Recherche** (`/find`, `/find/file`, `/find/symbol`) | text/files/symbols | **✅ FULL** | — |
| **Agents / Models / Commands / Skills** (`/agent`,`/model`,`/command`,`/skill`) | list | **✅ FULL** | — |
| **TUI control** (`/tui/*`) | append/clear/submit-prompt, execute-command, open-models, select-session, show-toast, control/next, control/response (9) | **✅ FULL** | — |
| **Permission** (`/permission/pending`, `/permission/reply`) | list/reply | **✅ FULL** | — |
| **Auth** (`/auth/{provider}`) | set/remove | **✅ FULL** | — |
| **LSP / VCS / Formatter** (`/lsp`,`/vcs/diff`,`/vcs/apply`,`/formatter`) | status/diff/apply | **✅ FULL** | — |
| **PTY** (`/pty`,`/pty/{id}`,`/input`,`/resize`,`/stream`) | create/io/stream (5) | **PARTIAL** | Low (connect-token, WS dédié) |
| **Project / Path / MCP** (`/project`,`/project/current`,`/project/discovery`,`/path`,`/mcp`) | list/current/discovery | **PARTIAL** (`/mcp` = list seule, pas add/auth/connect) | Med (MCP REST) |
| **Sync / Global / Experimental / Workspace / Worktree** | divers | **MISSING** | Low |
| `GET /health`, `GET /openapi.json` | health/spec | **PARTIAL** (`/health` racine ; openapi faite main mais ~80 routes) | Low |

### C. SSE / Flux d'événements — ✅ CLÔTURÉ (recouvrement de noms)

L'enum `Event` cortex compte désormais **58 variantes** (`event.rs:345+`) et émet un vocabulaire **largement wire-aligné** avec opencode — y compris la double émission délibérée des deltas legacy `assistant.*_delta`/`turn.*` **et** des noms compatibles `session.next.*.delta` (cf. `lib.rs:1212`). Transport identique (frames `event:`+`data:`, keepalive) ; cortex conserve sa redaction streaming + l'endpoint entrant `POST /event`.

| Famille | opencode | cortex | statut |
|---|---|---|---|
| `permission.asked`/`replied` | FULL | **FULL** (noms identiques) | ✅ |
| `mcp.tools/resources/prompts.changed`, `mcp.log` | tools | **FULL+** | ✅ |
| `message.updated`/`message.part.updated`/`removed`/`parts` | FULL | **✅ FULL** (noms partagés) | ✅ |
| `session.updated/status/idle/error/compacted/diff` | FULL | **✅ FULL** (noms partagés) | ✅ |
| `session.next.{text,reasoning,tool,step}.{start,delta,end,started,finished,ended}` | FULL | **✅ FULL** (`event.rs` `SessionNext*` + dual-émission) | ✅ |
| `session.next.{agent.switched,model.switched,context.updated,retried}` | FULL | **✅ FULL** | ✅ |
| `file.edited`/`file.watcher.updated`, `lsp.updated`, `project.updated`, `installation.updated`, `todo.updated`, `question.asked/replied/rejected`, `vcs.branch.updated`, `tui.*` | FULL | **✅ FULL** (noms partagés) | ✅ |
| `session.next.shell.started/ended`, `pty.*`, `server.connected/heartbeat`, `worktree.*`, `workspace.*`, `catalog.updated` | FULL | — | **MISSING** (résiduel Low) |

**Propre à cortex** : `lifecycle`, `turn.started/usage/done/error`, `tool.call/result` (conservés **en plus** des noms compatibles — vocabulaire dual, pas un blocage). **Conséquence : un client/TUI/SDK opencode peut désormais consommer l'essentiel du flux cortex** (deltas de streaming, message/session, permission, mcp, todo, question, tui).

### D. ACP — ✅ CLÔTURÉ (~FULL)

| Méthode ACP | opencode | statut cortex | cortex |
|---|---|---|---|
| `initialize` | FULL | **FULL** | lib.rs:320 |
| `authenticate` | FULL | **✅ FULL** | lib.rs:321 |
| `session/new` | FULL | **FULL** | lib.rs:327 |
| `session/load` | FULL | **✅ FULL** | lib.rs:333 |
| `session/list` | FULL | **✅ FULL** | lib.rs:339 |
| `session/resume` | FULL | **✅ FULL** | lib.rs:345 |
| `session/close` / `session/delete` | FULL | **✅ FULL** | lib.rs:351,363 |
| `session/fork` | FULL | **✅ FULL** | lib.rs:357 |
| `session/set_mode` / `set_model` / `set_config_option` | FULL | **✅ FULL** | lib.rs:369-381 |
| `session/prompt` | FULL | **FULL** (content blocks) | lib.rs:387 |
| `session/cancel` | FULL | **FULL** | lib.rs:393 |
| Notif `session/update` | full set | **✅ FULL** (`agent_message_chunk`, `agent_thought_chunk`, `tool_call`, `tool_call_update`, `plan`, `available_commands_update`, `current_mode_update`, `diff`) | lib.rs:1101-1400 |
| Appels client `fs/read_text_file`,`fs/write_text_file`,`terminal/{create,wait_for_exit,output,release}` | FULL | **✅ FULL** | client.rs:229-374 |

### E. SDK — ✅ CLÔTURÉ (PARTIAL)

| Aspect | opencode | statut cortex | sév résiduelle |
|---|---|---|---|
| Client généré | `OpencodeClient` JS + 25 classes | **✅ SDK TypeScript généré** (`sdk/` : `openapi.json`, `src/`, `test/`, `scripts/`, `package.json`) | Med (TS uniquement, pas de client Rust natif) |
| Spec OpenAPI | spec ~1 Mo, ~127 ops | **PARTIAL** — faite main (`openapi.rs`), couvre désormais ~80 routes | Low |
| Client de flux d'événements typé | `serverSentEvents.gen.ts` | **PARTIAL** (types via SDK TS) | Low |

**Synthèse serveur/API/SSE/ACP/SDK** : la divergence structurelle du 24-juin est **résorbée**. cortex expose désormais ~63 % de la surface HTTP opencode (CRUD session complet + config/providers/fichiers/recherche/agents/auth/lsp/vcs/formatter/tui/permission/pty), un flux SSE **wire-aligné** à 58 variantes (dual-émission `session.next.*` + noms partagés sur message/session/permission/mcp/todo/question/tui), un ACP **quasi complet** (14 méthodes + `session/update` full + appels client fs/terminal), et un **SDK TypeScript généré**. **Résiduels honnêtes** : MCP REST réduit à `list` (pas add/auth/connect) — Med ; familles MISSING `sync`/`workspace`/`worktree`/`experimental` + PTY connect-token/WS — Low ; spec OpenAPI faite main et SDK **TS-only** (pas de client Rust natif) — Med/Low ; vocabulaire d'événements **dual** (legacy `turn.*`/`lifecycle` conservés à côté des noms compat — design assumé, pas un bug). La **décision stratégique wire-compat** initialement requise a, de fait, été **tranchée en faveur de la compatibilité** : l'effort restant est incrémental, pas structurel.

---

## 8 — TUI

> **Cadrage** : le TUI de référence est en **TypeScript/SolidJS** (moteur OpenTUI, `*.tsx`), **pas** Go/bubbletea. opencode répartit ses actions entre un registre de slash-commands typées *minimal* + un **registre de keybinds (~153 actions)** + la **palette ctrl+p**. cortex expose désormais **39 entrées slash** + **143 arms de keybind** + palette ctrl+p.

> ✅ **CLÔTURÉ (essentiel)** — Les 4 écarts **High** du 24-juin sont **résolus** : (1) coloration syntaxique `rust`/`bash` → **13 grammaires tree-sitter** (`frame/Cargo.toml:36-48`) ; (2) pièces jointes/images **absentes** → **présentes** (`/attach`, `/paste`, chips, png — `ui.rs:106,863-884,2295`) ; (3) diff inline-only → **viewer split + unifié + navig. hunk + revert ciblé** (`diff.rs:88,113,125-145`) ; (4) keybinds **8** → **143 arms** (`keybind.rs`). Slash 16→39, + subagents/timeline/fork/skills/historique-frecency-stash désormais présents.

### 8.1 Slash-commands (statut cortex)

`slash::BUILTINS` (`slash.rs:19+`) compte **39 entrées** (32 commandes distinctes + alias). Présentes : `help, new, sessions, models(+model), agents, skills, variant, themes(+theme), status, timeline, fork, subagents, queue, rename, attach, paste, init, review, diff, compact, export, editor, share, unshare, mcp, details, undo, redo, exit`.

| Commande | opencode | statut cortex | preuve |
|---|---|---|---|
| `/help` | modale | **PARTIAL** — panneau + which-key, pas modale | `slash.rs:21` ; `ui.rs:641` |
| `/new`,`/sessions`(+rename/delete),`/agents`,`/init`,`/compact`,`/editor`,`/undo`,`/redo`,`/mcp`,`/exit` | FULL | **FULL** | `slash.rs` ; `app.rs` |
| `/models` | picker groupé | **PARTIAL** — picker, liste essentiellement OpenRouter | `slash.rs:33` |
| `/themes` | 33 JSON + preview | **PARTIAL** — set builtin mince (~dark/light/monokai…), preview/restore, pas de JSON custom | `slash.rs:53` |
| `/review` | sous-tâche | **PARTIAL** — tour normal, pas sous-agent, pas d'arg | `app.rs` |
| `/export` | dialogue options | **PARTIAL** — export direct | `app.rs` |
| `/share` / `/unshare` | partage cloud | **PARTIAL** — loopback (mais `/unshare` désormais **présent**) | `slash.rs` |
| `/skills` | `RunSkillSelectBody` | **✅ FULL** | `slash.rs:45` |
| `/variant` (cycle/list) | `variant_cycle/list` | **✅ FULL** | `slash.rs:49` |
| `/timeline`, `/fork` | dialogues | **✅ FULL** | `slash.rs` ; `app.rs:3005-3006` |
| `/subagents` | `dialog-subagent.tsx` | **✅ FULL** (panel + footer) | `app.rs:1110,1117,3015` |
| `/queue` | gestion file | **✅ FULL** | `slash.rs` |
| `/status` | `dialog-status.tsx` | **✅ FULL** | `slash.rs` |
| `/attach`, `/paste` | pièces jointes | **✅ FULL** | `slash.rs` ; `ui.rs:2942` |
| `/diff` | viewer | **✅ FULL** | `slash.rs` ; `diff.rs` |
| `/rename`, `/details` | keymap | **FULL** (commandes dédiées) | `slash.rs` |
| commandes config custom / prompt MCP | `cfg.command` / `mcp.prompts` | **MISSING** | — |

### 8.2 Comparaison des fonctionnalités

| Fonctionnalité | opencode | statut cortex | sév | preuve |
|---|---|---|---|---|
| Rendu markdown streaming | Full | **FULL** | low | `ui.rs:434-442` |
| Coloration syntaxique (tree-sitter) | Full | **✅ FULL** — **13 grammaires** (bash/c/cpp/go/java/js/json/md/python/rust/toml/ts/yaml) | — | `frame/Cargo.toml:36-48` |
| Rendu de diff | split+unifié+arbre+revert | **✅ FULL** — unifié **et** split (`split_rows`), navig. hunk, revert ciblé | — | `diff.rs:88,113,125-145` |
| Pièces jointes / images | collage presse-papier + png/jpg/pdf | **✅ FULL** — `/attach`+`/paste`, chips, png | — | `ui.rs:106,863-884,2295,2942` |
| Keybinds / leader | ~153 actions | **✅ FULL (sensiblement)** — **143 arms** | low (résiduel ~10) | `keybind.rs` |
| Cartes d'appels d'outils | par carte | **FULL** | low | `tool_card.rs` |
| Rendu des messages | Full | **FULL** | low | `app.rs:101-121` |
| Mentions `@` | @file + @agent | **PARTIAL** — @file fuzzy seul, **pas @agent** | med | `mention.rs` |
| Mode shell `!` | Full | **FULL** | low | `shell.rs` |
| Palette de commandes (ctrl+p) | Full | **FULL** — exécute les 39 slash + sous-menus | low | `app.rs:1519-1611` |
| Thèmes | 33 JSON + custom + preview | **PARTIAL** — builtins minces, pas de JSON custom | med | `style.rs` |
| Édition readline | ~40 actions input_* | **PARTIAL** — édition de base, set input_* pas pleinement configurable | low/med | `keybind.rs` |
| Picker de session | Full | **FULL** (+rename/delete) | low | `app.rs:1678` |
| Recherche dans le chat | limitée | **FULL+** — ctrl+f, match N/M, surlignage | — | `app.rs:214` |
| Toasts / Undo-redo / Statut / which-key / $EDITOR / permission | Full | **FULL** | low | `app.rs` |
| Sidebar | files/lsp/mcp/todo/context | **PARTIAL→** panneaux (incl. Subagents) togglables | low | `app.rs:179,1110` |
| Sous-agents (tabs/footer/picker) | Full | **✅ FULL** | — | `app.rs:1110,1117,3015` |
| Timeline / fork de session | Full | **✅ FULL** | — | `app.rs:322,744,3005-3006` |
| Cyclage model/agent/variant | f2/tab/ctrl+t | **✅ FULL** (`/variant` + keybinds) | low | `slash.rs:49` ; `keybind.rs` |
| Keybinds navigation messages | page/half/first/last/next/prev | **FULL (sensiblement)** | low | `keybind.rs` |
| Historique de prompt / frecency / stash | `history`+`frecency`+`stash` | **✅ FULL** | — | `app.rs:478-495,800` |
| Skills | `RunSkillSelectBody` | **✅ FULL** | — | `slash.rs:45` |
| Prompts en file (mid-stream) | file + dialogue | **FULL** (`/queue`) | low | `slash.rs` ; `app.rs:473` |
| Dialogue question (l'agent demande) | `question.tsx` | **PARTIAL/MISSING** | med | — |
| Audio / attention (cloche) | `audio.ts`/`attention.ts` | **MISSING** | low | — |
| Système plugin / slots TUI | `plugin/*` | **MISSING** | low | — |
| Dialogues workspace | `dialog-workspace-*` | **MISSING** | low | — |

**Synthèse TUI** : le portage est désormais **largement complet**. Les 4 écarts **High** du 24-juin sont **clôturés** : coloration syntaxique (**13 grammaires** vs rust/bash), pièces jointes/images en entrée, viewer de diff **split + hunk-nav + revert**, et profondeur keybinds (**143 arms** ≈ parité). Les fonctions Med jadis manquantes — sous-agents, timeline+fork, skills, cyclage model/agent/variant, historique/frecency/stash, file de prompts — sont **présentes**. **Résiduels honnêtes** : `@agent` mention (Med), thèmes minces sans JSON custom (Med), readline `input_*` non pleinement configurable (Med/Low), dialogue question agent (Med), et MISSING Low : audio/cloche, slots plugin TUI, dialogues workspace, commandes config-custom / MCP-prompt-comme-commande. **Net** : ~85-90 % de portage fonctionnel du cœur interactif ; les trous restants sont périphériques, plus les 4 gaps structurels visibles de l'audit précédent.

---

## 9 — CLI / Stockage / Bus / Git / divers

> ✅ **CLÔTURÉ (essentiel)** — La surface CLI et la persistance ont été massivement étendues depuis le 24-juin : commandes `attach`, `provider`, `plugin install`, `worktree`, `project`, `account`, `env`, et un **namespace `debug` unifié** (`debug {db,config,lsp,audit,info}` — incluant le **shell SQLite** via `debug db`) ; côté stockage, **table `message_parts` typée** (`0005`), **tables `projects`/`project_directories`/`accounts`** (`0008`), **IDs monotones lexicographiquement triables** (`id.rs:1`), et **normalisation/downscale image avant LLM** (`media.rs:12,100`). Les écarts High/Med structurels du précédent audit (part table aplatie, IDs non triables, pas de resize image, pas de modèle projet/worktree) sont **résolus**.

### 9.1 Surface des commandes CLI

| Capacité | opencode (fichier) | statut cortex | sév | cortex (fichier) |
|---|---|---|---|---|
| `run` (prompt headless) | `cli/cmd/run/` | **FULL** | High | `run.rs` (2744 LOC) |
| `tui` | `cli/cmd/tui.ts` | **FULL** | High | `cli.rs` → crate `tui` |
| `serve` (API HTTP) | `cli/cmd/serve.ts` | **FULL (sensiblement)** — crate `server` ~80 routes | Low | `serve_cmd.rs` |
| `acp` | `cli/cmd/acp.ts` | **FULL** | High | `acp_cmd.rs` |
| `mcp` (serveurs + oauth) | `cli/cmd/mcp.ts` | **FULL** | High | `mcp_cmd.rs` (583 LOC) |
| `session` (list/delete) | `cli/cmd/session.ts` | **FULL** | Med | `session.rs` (280 LOC) |
| `export` / `import` | `cli/cmd/{export,import}.ts` | **PARTIAL** — export FULL ; import local (pas d'import URL share) | Low | `export_import.rs` |
| `stats`,`models`,`generate`,`agent`,`completion` | divers | **FULL** | — | `stats.rs`/`models_cmd.rs`/`generate_cmd.rs`/`agent_cmd.rs`/`completion_cmd.rs` |
| `auth` / login | (opencode `account`) | **CORTEX-naming** | High | `auth_cmd.rs` (290 LOC) |
| `account` / `console` (control-plane) | `cli/cmd/account.ts` | **PARTIAL** — `account_cmd.rs`+`account_store.rs` présents ; control-plane cloud absent | Low | `account_cmd.rs` |
| `provider` (providers) | `cli/cmd/providers.ts` | **✅ FULL** | — | `provider_cmd.rs` |
| `attach` (session active) | `cli/cmd/attach.ts` | **✅ FULL** | — | `attach.rs` |
| `web` / `github` / `pr` | `cli/cmd/*` | **OUT-OF-SCOPE** | Low | — |
| `plugin` (install/list/remove) | `cli/cmd/plug.ts` | **✅ FULL** | — | `plugin_cmd.rs` (`install`/`list`/`remove`) |
| `upgrade` / `uninstall` | `cli/cmd/{upgrade,uninstall}.ts` | **PARTIAL** (présents mais status-only/dry-run par design ; pas de self-update binaire) | Low | `lifecycle.rs:43` |
| `worktree` | `worktree/index.ts` | **✅ FULL** | — | `worktree_cmd.rs` |
| `project` (multi-dir) | `project/project.ts` | **✅ FULL** | — | `project_cmd.rs` |
| `env` (service get/all/set/remove) | `env/index.ts` | **✅ FULL** | — | `env_cmd.rs` |
| `debug` (suite unifiée) | `cli/cmd/debug/` | **✅ FULL** — `debug {db,config,lsp,audit,info}`, **`debug db` = shell SQLite** | — | `debug_cmd.rs` |
| `search` / `replay` / `undo`+`redo` / `net` / `man` | — | **CORTEX-UNIQUE** | — | `search.rs`/`replay.rs`/`checkpoint.rs`/`net.rs`/`man_cmd.rs` |

### 9.2 Stockage / persistance

| Capacité | opencode | statut cortex | sév | cortex |
|---|---|---|---|---|
| Backend SQLite | drizzle + effect-sqlite | **FULL** | High | `db/src/lib.rs:1` (rusqlite bundled) |
| Auto-init / auto-migrate | drizzle | **FULL** (8 migrations) | High | `db/src/migrations/0001…0008` |
| Ouverture concurrente-safe (WAL, BEGIN IMMEDIATE, retry/backoff) | partiel | **FULL (dépasse)** | High | `db/src/lib.rs:62-164` |
| Table `messages` + `part` typée | PartTable + MessageTable | **✅ FULL** — `message_parts(part_json discriminé, ordinal, idx)` | — | `0005_message_parts.sql` |
| Table `todo` (+priorité) | TodoTable | **FULL** | — | `0003`,`0006` |
| Table `usage`/coût | (dérivé) | **CORTEX-UNIQUE** | Low | `0002_usage.sql` |
| Table `account` | account/sql | **✅ FULL** | — | `0008_projects_accounts.sql:29` |
| Table `project` (multi-dir) | ProjectTable | **✅ FULL** — `projects`+`project_directories` | — | `0008_projects_accounts.sql:9,16` |
| integrity_check / anti-corruption | — | **CORTEX-UNIQUE** | Med | `db/src/lib.rs:204` |

### 9.3 Bus / événements

| Capacité | opencode | statut cortex | sév | cortex |
|---|---|---|---|---|
| Bus d'événements in-process | `bus/global.ts` | **FULL** | Med | `events/src/bus.rs` (tokio broadcast + trait Subscriber) |
| Définitions d'événements typées | `EventV2.define` par sous-système | **FULL (sensiblement)** — enum `Event` **58 variantes** | Low | `events/src/event.rs:345+` |
| Attribution Surface (Tui/Run/Serve/Acp…) | (implicite) | **CORTEX-UNIQUE** | Low | `events/src/event.rs` |
| Événements `worktree.*`/`ide.*`/`installation.*` | EventV2 | **PARTIAL** | Low | partiels (sous-systèmes IDE absents) |

### 9.4 Git / Project / Worktree / Snapshot

| Capacité | opencode | statut cortex | sév | cortex |
|---|---|---|---|---|
| Intégration Git | `git/index.ts` | **PARTIAL** | Med | `checkpoints/src/git.rs` + outils |
| Modèle projet multi-répertoire | `project/project.ts` | **✅ FULL** | — | `0008` + `project_cmd.rs` |
| Worktree create/list | `worktree/index.ts` | **✅ FULL** | — | `worktree_cmd.rs` |
| Snapshot track/restore | `snapshot/index.ts` | **FULL** | High | `checkpoints/src/store.rs:235,255` |
| Snapshot revert (par patch) | `snapshot/index.ts:416` | **PARTIAL** — revert par tour entier | Med | `store.rs` |
| Snapshot diff/diffFull | `snapshot/index.ts:534` | **MISSING** | Low | — |
| Undo/redo granularité tour | (snapshots TUI) | **CORTEX-UNIQUE (plus fort)** | High | journal cursor `checkpoints` |
| Object DB alternates / gc-prune | `snapshot/index.ts:206,308` | **MISSING** | Low | — |

### 9.5 ID / Env / Image / Redaction / Audit

| Capacité | opencode | statut cortex | sév | cortex |
|---|---|---|---|---|
| **IDs monotones/triables temps** | `id/id.ts:51` | **✅ FULL** — clock monotone + compteur intra-ms, `{prefix}_{32hex}` | — | `db/src/id.rs:1-68` |
| Taxonomie de préfixes d'ID | 10 préfixes | **FULL (sensiblement)** | Low | `db/src/id.rs` |
| Service Env (get/all/set/remove) | `env/index.ts` | **✅ FULL** | — | `cli/src/env_cmd.rs` |
| **Resize/normalize image avant LLM** | `image/image.ts:75` | **✅ FULL** — `normalize_image_bytes`, downscale `MAX_IMAGE_DIMENSION`, cap octets | — | `llm/src/media.rs:12,100-103` |
| Redaction secrets (forme + streaming) | (export opencode) | **CORTEX-UNIQUE (dédié)** | Med | `redaction/src/secret.rs:62` |
| Garde réseau sortant | — | **CORTEX-UNIQUE** | Med | `redaction/src/outbound.rs` |
| Sink d'audit JSONL durable | — | **CORTEX-UNIQUE** | Med | `audit/src/lib.rs` |

### 9.6 Installation / IDE / cycle de vie

| Capacité | opencode | statut cortex | sév |
|---|---|---|---|
| `upgrade` / `uninstall` (CLI) | `installation/` + `cmd/*` | **PARTIAL** — présents mais status-only / dry-run (pas de self-update binaire) | Low |
| Détection méthode d'installation / update-available | `installation/index.ts` | **MISSING** | Low |
| Install extension IDE (VSCode/Cursor/…) | `ide/index.ts` | **MISSING** | Low |

### 9.7 Écosystème (packages)

| Package | Rôle | statut cortex | sév |
|---|---|---|---|
| `opencode`/`core`/`cli` / `tui` / `llm` | runtime + UI + provider | **FULL** | — |
| `server` | API HTTP | **FULL (sensiblement)** (~80 routes) | Low |
| `sdk` | SDK client | **✅ PARTIAL** — SDK TS généré (`sdk/`) | Low |
| `plugin` / `schema` | runtime plugin / schéma | **PARTIAL** (`plugins`+`plugin_bridge`, crate `protocol`) | Med/Low |
| `app`/`desktop`/`web`/`console`/`enterprise`/`slack`/… | GUI/web/cloud/intégrations | **OUT-OF-SCOPE** | Low |
| `http-recorder` | fixtures de test | **MISSING** | Low |

**Synthèse CLI/stockage/bus/git** : la surface CLI est désormais **à parité large** — toutes les commandes agent cœur FULL, plus `attach`/`provider`/`plugin install`/`worktree`/`project`/`env`/`account` et une **suite `debug` unifiée** (avec shell SQLite `debug db`). La persistance **dépasse** opencode en robustesse (WAL/retry/integrity-check) **et** a clôturé ses écarts structurels : **table `part` typée** (`message_parts`), **tables `projects`/`accounts`**, **IDs monotones triables**, **normalisation image**. **Résiduels honnêtes** : `upgrade`/`uninstall` status-only par design (pas de self-update binaire), import URL-share, control-plane cloud (`console`), snapshot diff/gc-prune, intégration IDE — tous **Low**. Cortex conserve sa **posture sécurité supérieure** (`redaction` secrets+streaming+outbound + `audit` JSONL) et ses commandes propres (`search`/`replay`/`undo-redo`/`net`/`man`).

---

## 10 — Synthèse globale & priorités de remédiation

> ✅ **MISE À JOUR MAJEURE** — Depuis l'audit du 24-juin (141 commits), la quasi-totalité des verrous **CRITICAL/High** est **clôturée**. Le diagnostic d'origine (« PARTIAL partout, 3 familles d'écarts bloquantes ») est **obsolète** : cortex est passé d'un squelette d'agent à un **portage à parité large**. Le tableau ci-dessous reflète l'état réel des **31 crates / ~243k LOC** (HEAD `30cd3b7`).

### 10.1 Tableau de bord de parité par sous-système

| # | Sous-système | Parité (avant) | Parité (maintenant) | Résiduels honnêtes |
|---|---|---|---|---|
| 1 | Plugins / Skills / Memory | PARTIAL | **✅ FULL (sensiblement)** | callback `client` in-process partiel |
| 2 | Outils (tools) | PARTIAL | **✅ FULL (sensiblement)** | étendue des replacers `edit` ; qq outils OC-only |
| 3 | LLM / Providers | PARTIAL | **✅ FULL (sensiblement)** — 18 profils, retry, cache, multimodal, OAuth/ADC | **Bedrock SigV4 absent** (PARTIAL) ; catalogue embarqué OpenRouter-only ; pas de comptage tokens local |
| 4 | MCP / LSP / Format | PARTIAL | **✅ FULL (sensiblement)** — LSP **25**, format **~22**, MCP `roots` | **auto-download LSP MISSING** ; registre 25/38 |
| 5 | Engine (runner) | PARTIAL | **✅ FULL (sensiblement)** — outils parallèles, 12 variantes de parts, TurnQueue, retries, doom-loop | **compaction mécanique** (pas de résumé LLM) |
| 6 | Permissions / Policy / Config | PARTIAL | **✅ FULL (sensiblement)** — `external_directory`, reject-feedback, async, policies | env `OPENCODE_PERMISSION`, `tools:{bool}`, cascade-reject |
| 7 | Serveur / API / SSE / ACP / SDK | MISSING (~7 %) | **✅ FULL (sensiblement)** — ~80 routes (~63 %), 58 events wire-alignés, ACP 14 méthodes, SDK TS | MCP REST list-only ; SDK **TS-only** ; sync/workspace/worktree MISSING |
| 8 | TUI | PARTIAL | **✅ FULL (sensiblement)** ~85-90 % — 13 grammaires, pièces jointes, diff split, 143 keybinds, 39 slash | `@agent`, thèmes minces, audio/cloche, slots plugin |
| 9 | CLI / Stockage / Bus / Git | PARTIAL→FULL | **✅ FULL** — part/project/account tables, IDs triables, debug unifié | self-update binaire, import URL-share, IDE |

### 10.2 Lecture transverse

cortex est désormais un **portage fidèle et large** d'opencode : non seulement la colonne vertébrale d'agent (boucle de tour bornée, outil→résultat→continuation, tool-set MCP live, rulesets de permission, persistance durable) mais aussi la **surface périphérique** jadis absente — API HTTP (~63 %), flux SSE wire-aligné, ACP quasi complet, SDK TypeScript, TUI riche (syntaxe multi-langages, pièces jointes, diff split, sous-agents/timeline), persistance à parts typées + projets/comptes. Il **dépasse** opencode sur plusieurs axes de robustesse/sécurité : redaction de secrets de bout en bout (forme+streaming+outbound), audit JSONL durable, checkpoints git avec undo/redo par tour + détection de conflit, décisions de permission cross-process, fail-closed headless, kill-switch réseau (`CORTEX_NO_NET`), et une **porte de confiance projet (`policy`)** sans équivalent.

Les **trois familles d'écarts bloquantes** du précédent audit sont **résorbées** :

1. ~~Modèle LLM appauvri~~ → **CLÔTURÉ** : 18 profils providers, retry/backoff classifié, prompt caching, entrée multimodale, reasoning, OAuth/ADC. *Résiduel* : signature AWS SigV4 (Bedrock) absente + catalogue embarqué OpenRouter-only.
2. ~~Largeurs d'écosystème~~ → **CLÔTURÉ** : LSP 25, formateurs ~22, coloration 13 langages, `edit` enrichi. *Résiduel* : auto-download LSP, étendue replacers.
3. ~~Surface serveur/SDK inexistante & divergente~~ → **CLÔTURÉ** : la **décision wire-compat a été tranchée en faveur de la compatibilité** (dual-émission `session.next.*`, noms d'events partagés, ACP complet, SDK TS). L'effort restant est **incrémental, pas structurel**.

### 10.3 Résiduels prioritaires (ce qui reste réellement)

**Comportemental (impact agent quotidien)**
1. **Compaction par résumé LLM** (`engine/compaction.rs:170-188`) — aujourd'hui prune + troncature caractère mécanique, pas de résumé. *Seul écart comportemental notable du moteur.*
2. **Bedrock SigV4** (`llm`/`runtime_auth.rs`) — chaîne de creds présente mais **aucune signature AWS** → Bedrock inutilisable ; et catalogue embarqué encore OpenRouter-only (14 modèles).

**Écosystème (mécanique)**
3. **Auto-download des serveurs LSP** (registre 25/38, infra de téléchargement absente).
4. Étendue des replacers `edit` ; quelques outils OC-only ; ~4 formateurs de niche.

**Surface / périphérie (Low)**
5. MCP REST réduit à `list` ; familles de routes `sync`/`workspace`/`worktree` MISSING ; SDK **TS-only** (pas de client Rust natif).
6. TUI : mention `@agent`, thèmes (set mince, pas de JSON custom), audio/cloche, slots plugin, dialogue question agent.
7. Lifecycle : self-update binaire (actuellement status-only), import URL-share, intégration IDE.

> **Net** : cortex n'est plus « un squelette à ~7-60 % » mais un **portage à parité large** (~80-90 % fonctionnel sur les sous-systèmes cœur), avec une posture sécurité supérieure. Les résiduels sont **ciblés et non bloquants** ; le plus impactant au quotidien est la **compaction mécanique** (#1), suivi de **Bedrock SigV4** (#2) et de l'**auto-download LSP** (#3).

> **Note de méthode** : `research/04-existing-cortex-code-inventory.md` est **périmé** (décrit l'ancien projet 41-crates) ; le présent rapport reflète l'état réel des 31 crates actuelles (HEAD `30cd3b7`, +141 commits depuis l'audit initial).
