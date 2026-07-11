//! Fail-closed public renderer planning and atomic artifact publication.
//!
//! Public selectors always resolve and validate their complete output set
//! before a coordinator or staging directory is created. Rendering never
//! writes directly into an output root: a complete replacement tree is staged
//! beside the root and atomically exchanged only after every artifact passes
//! its local checks.

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::CString;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write as _};
use std::os::raw::c_char;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::audit::{audit_screen, sha256_hex, AuditDocument};
use crate::flows::{flow_manifests, FlowManifest, TransitionKind};
use crate::registry::{canonical_inventory, Category, ImplementationStatus, ScreenSpec};
use crate::validate_canonical_map;

const CANONICAL_ROOT: &str = "/projects/cortex-code/plans/assets/mockups-v3";
const GATE_ROOT: &str = "/projects/cortex-code/plans/assets/mockups-v3-gates";
const MANIFEST_FILE: &str = ".renderer-manifest-v1";
const MANIFEST_HEADER: &str = "mockup-harness-v3-render-manifest-v1";
const CANONICAL_STATIC_FILES: &[&str] = &[
    "README.md",
    "session/.gitkeep",
    "input/.gitkeep",
    "views/.gitkeep",
    "auth/.gitkeep",
    "states/.gitkeep",
    "orchestration/.gitkeep",
];
const GATE_STATIC_FILES: &[&str] = &[".gitkeep"];

// Screen workers add an ID here only when the inventory marks that same ID as
// implemented. Keeping this map independent from the canonical inventory makes
// partial milestones fail closed instead of silently emitting placeholders.
const IMPLEMENTED_RENDERERS: &[&str] = &[
    "home",
    "chat-turn",
    "streaming",
    "tool-running",
    "tool-success",
    "rich-response",
    "file-mention",
    "composer-multiline",
    "permission-shell",
    "permission-deny-reason",
    "diff-edit",
    "diff-viewer-staging",
    "onboarding",
    "model-picker",
    "agent-picker",
    "mode-switch",
    "mode-switch-refused",
];

static STAGING_SEQUENCE: AtomicUsize = AtomicUsize::new(0);

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PublicSelector {
    Implemented,
    All,
    Screen(String),
    Category(Category),
    Flow(String),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RenderExit {
    Io,
    Validation,
    Busy,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RenderFailure {
    pub exit: RenderExit,
    pub message: String,
}

impl RenderFailure {
    fn io(context: &str, error: io::Error) -> Self {
        Self {
            exit: RenderExit::Io,
            message: format!("{context}: {error}"),
        }
    }

    fn validation(message: impl Into<String>) -> Self {
        Self {
            exit: RenderExit::Validation,
            message: message.into(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct RenderPlan {
    canonical: Vec<&'static ScreenSpec>,
    flow: Option<&'static FlowManifest>,
}

impl RenderPlan {
    pub fn canonical_ids(&self) -> impl Iterator<Item = &'static str> + '_ {
        self.canonical.iter().map(|screen| screen.id)
    }

    pub fn flow_name(&self) -> Option<&'static str> {
        self.flow.map(|flow| flow.name)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RootKind {
    Canonical,
    Gate,
}

impl RootKind {
    fn path(self) -> &'static Path {
        match self {
            Self::Canonical => Path::new(CANONICAL_ROOT),
            Self::Gate => Path::new(GATE_ROOT),
        }
    }

    fn static_files(self) -> &'static [&'static str] {
        match self {
            Self::Canonical => CANONICAL_STATIC_FILES,
            Self::Gate => GATE_STATIC_FILES,
        }
    }
}

struct PreparedRoot {
    kind: RootKind,
    root: PathBuf,
    artifacts: BTreeMap<String, Vec<u8>>,
}

struct StagedRoot {
    root: PathBuf,
    stage: PathBuf,
}

#[derive(Debug)]
struct CoordinatorLock {
    path: PathBuf,
}

impl CoordinatorLock {
    fn acquire(root: &Path) -> Result<Self, RenderFailure> {
        let parent = root.parent().ok_or_else(|| {
            RenderFailure::validation(format!("output root has no parent: {}", root.display()))
        })?;
        let name = root.file_name().ok_or_else(|| {
            RenderFailure::validation(format!("output root has no file name: {}", root.display()))
        })?;
        let lock_name = format!(".{}.render.lock", name.to_string_lossy());
        let path = parent.join(lock_name);
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(mut file) => {
                let _ = writeln!(file, "renderer-pid={}", std::process::id());
                let _ = file.sync_all();
                Ok(Self { path })
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => Err(RenderFailure {
                exit: RenderExit::Busy,
                message: format!("render coordinator busy for {}", root.display()),
            }),
            Err(error) => Err(RenderFailure::io(
                &format!(
                    "could not acquire render coordinator for {}",
                    root.display()
                ),
                error,
            )),
        }
    }
}

impl Drop for CoordinatorLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

/// Parses exactly one public gallery selector. Unknown, incomplete, and
/// ambiguous selectors are usage errors and cannot reach rendering.
pub fn parse_selector(arguments: &[String]) -> Result<PublicSelector, String> {
    match arguments {
        [flag] if flag == "--implemented" => Ok(PublicSelector::Implemented),
        [flag] if flag == "--all" => Ok(PublicSelector::All),
        [flag, flow] if flag == "--flow" => {
            if flow_manifests()
                .iter()
                .any(|manifest| manifest.name == flow)
            {
                Ok(PublicSelector::Flow(flow.clone()))
            } else {
                Err(format!("unknown flow selector: {flow}"))
            }
        }
        [flag] if flag == "--flow" => Err("missing flow name after --flow".to_owned()),
        [selector] => {
            if let Some(category) = Category::ALL
                .into_iter()
                .find(|category| category.as_str() == selector)
            {
                return Ok(PublicSelector::Category(category));
            }
            if canonical_inventory()
                .iter()
                .any(|screen| screen.id == selector)
            {
                return Ok(PublicSelector::Screen(selector.clone()));
            }
            Err(format!("unknown selector: {selector}"))
        }
        [] => Err("missing render selector".to_owned()),
        _ => Err(format!(
            "ambiguous render selector: {}",
            arguments.join(" ")
        )),
    }
}

/// Fully validates an artifact request before a lock, staging root, or output
/// artifact can be created.
pub fn preflight_render(selector: &PublicSelector) -> Result<RenderPlan, String> {
    validate_canonical_map()?;
    validate_renderer_map(canonical_inventory(), IMPLEMENTED_RENDERERS)?;

    let inventory = canonical_inventory();
    let (canonical, flow) = match selector {
        PublicSelector::Implemented => (
            inventory
                .iter()
                .filter(|screen| screen.status == ImplementationStatus::Implemented)
                .collect(),
            None,
        ),
        PublicSelector::All => {
            require_implemented(inventory)?;
            (inventory.iter().collect(), None)
        }
        PublicSelector::Screen(id) => {
            let screen = inventory
                .iter()
                .find(|screen| screen.id == id)
                .ok_or_else(|| format!("unknown canonical screen: {id}"))?;
            require_selected_implemented(&[screen])?;
            (vec![screen], None)
        }
        PublicSelector::Category(category) => {
            let screens = inventory
                .iter()
                .filter(|screen| screen.category == *category)
                .collect::<Vec<_>>();
            require_selected_implemented(&screens)?;
            (screens, None)
        }
        PublicSelector::Flow(name) => {
            let flow = flow_manifests()
                .iter()
                .find(|flow| flow.name == name)
                .ok_or_else(|| format!("unknown flow manifest: {name}"))?;
            let mut seen = BTreeSet::new();
            let mut screens = Vec::new();
            for step in &flow.steps {
                let screen = inventory
                    .iter()
                    .find(|screen| screen.id == step.screen_id)
                    .ok_or_else(|| {
                        format!(
                            "flow {} references unknown canonical screen: {}",
                            flow.name, step.screen_id
                        )
                    })?;
                if seen.insert(screen.id) {
                    screens.push(screen);
                }
            }
            require_selected_implemented(&screens)?;
            (screens, Some(flow))
        }
    };

    let plan = RenderPlan { canonical, flow };
    validate_existing_root(RootKind::Canonical)?;
    if plan.flow.is_some() {
        validate_existing_root(RootKind::Gate)?;
    }
    Ok(plan)
}

/// Publishes a completely preflighted plan through renderer-owned staging
/// trees. No public output is replaced until every staged root has passed
/// validation.
pub fn publish(plan: &RenderPlan) -> Result<(), RenderFailure> {
    let roots = prepare_roots(plan)?;
    if roots.is_empty() {
        // A zero-screen partial render still coordinates its canonical root.
        // This keeps the public busy contract observable without publishing a
        // placeholder artifact or changing an existing gallery.
        let _lock = CoordinatorLock::acquire(RootKind::Canonical.path())?;
        return Ok(());
    }

    let mut locks = Vec::with_capacity(roots.len());
    for root in &roots {
        locks.push(CoordinatorLock::acquire(&root.root)?);
    }

    publish_prepared_roots(&roots, false)?;
    drop(locks);
    Ok(())
}

/// Serializes ordered flow metadata only after the associated artifacts are
/// safely published.
pub fn flow_manifest_json(flow: &FlowManifest) -> String {
    let mut output = format!(
        "{{\"document_type\":\"flow-render\",\"schema_version\":1,\"flow\":{}",
        json_string(flow.name)
    );
    output.push_str(",\"steps\":[");
    for (index, step) in flow.steps.iter().enumerate() {
        if index > 0 {
            output.push(',');
        }
        output.push_str("{\"index\":");
        output.push_str(&(index + 1).to_string());
        json_field(&mut output, "screen_id", step.screen_id);
        json_field(&mut output, "action", step.user_action);
        json_field(
            &mut output,
            "transition_kind",
            transition_kind_name(step.transition_kind),
        );
        json_field(&mut output, "transition_path", step.transition_path);
        json_field(&mut output, "expected_next_frame", step.expected_next_frame);
        json_field(&mut output, "origin", step.origin);
        json_field(&mut output, "success_return", step.success_return);
        json_field(&mut output, "cancel_return", step.cancel_return);
        output.push_str(",\"terminal_return\":");
        match step.terminal_return {
            Some(value) => output.push_str(&json_string(value)),
            None => output.push_str("null"),
        }
        output.push_str(",\"acceptance_checks\":[");
        for (check_index, check) in step.acceptance_checks.iter().enumerate() {
            if check_index > 0 {
                output.push(',');
            }
            output.push_str(&json_string(check));
        }
        output.push_str("]}");
    }
    output.push_str("]}");
    output
}

/// Emits a single public audit or a deterministic collection for a selected
/// scope. It performs no artifact I/O.
pub fn audit_json(selector: Option<&PublicSelector>) -> Result<String, String> {
    validate_canonical_map()?;
    let selected = match selector {
        None => vec![(
            canonical_screen("permission-shell")?,
            "public-audit".to_owned(),
        )],
        Some(PublicSelector::Implemented) => canonical_inventory()
            .iter()
            .filter(|screen| screen.status == ImplementationStatus::Implemented)
            .map(|screen| (screen, "public-audit".to_owned()))
            .collect(),
        Some(PublicSelector::All) => canonical_inventory()
            .iter()
            .map(|screen| (screen, "public-audit".to_owned()))
            .collect(),
        Some(PublicSelector::Screen(id)) => {
            vec![(canonical_screen(id)?, "public-audit".to_owned())]
        }
        Some(PublicSelector::Category(category)) => canonical_inventory()
            .iter()
            .filter(|screen| screen.category == *category)
            .map(|screen| (screen, "public-audit".to_owned()))
            .collect(),
        Some(PublicSelector::Flow(name)) => {
            let flow = flow_manifests()
                .iter()
                .find(|flow| flow.name == name)
                .ok_or_else(|| format!("unknown flow manifest: {name}"))?;
            flow.steps
                .iter()
                .enumerate()
                .map(|(index, step)| {
                    let screen = canonical_screen(step.screen_id)?;
                    Ok((screen, format!("public-audit.{}.{}", flow.name, index + 1)))
                })
                .collect::<Result<Vec<_>, String>>()?
        }
    };

    let audits = selected
        .into_iter()
        .map(|(screen, fixture)| audit_screen(screen.id, &fixture))
        .collect::<Result<Vec<_>, _>>()?;
    if audits.len() == 1 {
        Ok(audits[0].to_json())
    } else {
        Ok(audit_collection(audits))
    }
}

fn canonical_screen(id: &str) -> Result<&'static ScreenSpec, String> {
    canonical_inventory()
        .iter()
        .find(|screen| screen.id == id)
        .ok_or_else(|| format!("unknown canonical screen: {id}"))
}

fn validate_renderer_map(inventory: &[ScreenSpec], renderer_ids: &[&str]) -> Result<(), String> {
    let inventory_by_id = inventory
        .iter()
        .map(|screen| (screen.id, screen))
        .collect::<BTreeMap<_, _>>();
    let mut actual = BTreeSet::new();
    for id in renderer_ids {
        if !actual.insert(*id) {
            return Err(format!("duplicate renderer mapping: {id}"));
        }
        let Some(screen) = inventory_by_id.get(id) else {
            return Err(format!("renderer outside canonical inventory: {id}"));
        };
        if screen.status != ImplementationStatus::Implemented {
            return Err(format!("renderer maps an unimplemented canonical ID: {id}"));
        }
    }
    for screen in inventory {
        if screen.status == ImplementationStatus::Implemented && !actual.contains(screen.id) {
            return Err(format!(
                "implemented canonical ID lacks renderer: {}",
                screen.id
            ));
        }
    }
    Ok(())
}

/// Rejects cleanup paths that do not belong to the same renderer-owned parent
/// and staging namespace as the output root. This predicate runs before any
/// fixture cleanup can remove a temporary directory.
pub fn validate_cleanup_paths(output_root: &Path, staging_root: &Path) -> Result<(), String> {
    let output_parent = output_root.parent().ok_or_else(|| {
        format!(
            "output root has no cleanup parent: {}",
            output_root.display()
        )
    })?;
    let staging_parent = staging_root.parent().ok_or_else(|| {
        format!(
            "staging root has no cleanup parent: {}",
            staging_root.display()
        )
    })?;
    let output_name = output_root
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| format!("output root has no cleanup name: {}", output_root.display()))?;
    let staging_name = staging_root
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| {
            format!(
                "staging root has no cleanup name: {}",
                staging_root.display()
            )
        })?;
    if output_parent != staging_parent
        || !staging_name.starts_with(&format!(".{output_name}.render-staging-"))
    {
        return Err(format!(
            "unsafe renderer cleanup path: {}",
            staging_root.display()
        ));
    }
    Ok(())
}

/// Runs the renderer-oriented negative cases through the same map validation,
/// staging, publication, and guarded cleanup operations used by the renderer.
/// The caller provides a disposable root, never a canonical or gate root.
pub fn run_renderer_negative_fixture(id: &str, root: &Path) -> Result<(), String> {
    match id {
        "inventory-without-renderer" => {
            let mut inventory = canonical_inventory().to_vec();
            inventory[0].status = ImplementationStatus::Implemented;
            validate_renderer_map(&inventory, &[])
        }
        "renderer-outside-inventory" => {
            validate_renderer_map(canonical_inventory(), &["outside-the-contract"])
        }
        "unsafe-cleanup" => fixture_unsafe_cleanup(root),
        "publication-failure" => fixture_publication_failure(root),
        other => Err(format!("unknown renderer negative fixture: {other}")),
    }
}

fn fixture_unsafe_cleanup(root: &Path) -> Result<(), String> {
    let output = root.join("published");
    let unsafe_stage = root.join("outside-staging");
    fs::create_dir(&unsafe_stage)
        .map_err(|error| format!("could not create unsafe cleanup sentinel: {error}"))?;
    fs::write(
        unsafe_stage.join("sentinel.txt"),
        b"must survive unsafe cleanup",
    )
    .map_err(|error| format!("could not write unsafe cleanup sentinel: {error}"))?;

    let cleanup = cleanup_stages(&[StagedRoot {
        root: output,
        stage: unsafe_stage.clone(),
    }]);
    if !unsafe_stage.join("sentinel.txt").is_file() {
        return Err("unsafe cleanup removed its sentinel".to_owned());
    }
    cleanup.map_err(|error| error.message)
}

fn fixture_publication_failure(root: &Path) -> Result<(), String> {
    let output = root.join("published");
    let prior = fixture_prepared_root(&output, "previous complete gallery");
    publish_prepared_roots(&[prior], false).map_err(|error| error.message)?;
    let before = fixture_tree_manifest(&output)?;

    let candidate = fixture_prepared_root(&output, "candidate incomplete gallery");
    let publication = publish_prepared_roots(&[candidate], true)
        .expect_err("injected validation failure must stop production publication");
    let after = fixture_tree_manifest(&output)?;
    if before != after {
        return Err(
            "publication failure fixture did not preserve the prior complete root".to_owned(),
        );
    }
    let prefix = ".published.render-staging-";
    let stale_stage = fs::read_dir(root)
        .map_err(|error| format!("could not inspect fixture publication parent: {error}"))?
        .filter_map(Result::ok)
        .any(|entry| entry.file_name().to_string_lossy().starts_with(prefix));
    if stale_stage {
        return Err("publication failure fixture left a renderer staging root".to_owned());
    }
    Err(publication.message)
}

fn fixture_prepared_root(root: &Path, contents: &str) -> PreparedRoot {
    let mut artifacts = BTreeMap::new();
    artifacts.insert(
        "session/fixture.txt".to_owned(),
        fixture_txt(contents).into_bytes(),
    );
    PreparedRoot {
        kind: RootKind::Canonical,
        root: root.to_path_buf(),
        artifacts,
    }
}

fn fixture_txt(label: &str) -> String {
    let mut text = String::new();
    let line = format!("fixture gallery: {label}");
    for row in 0..40 {
        let content = if row == 0 { line.as_str() } else { "" };
        text.push_str(content);
        text.push_str(&" ".repeat(120 - content.chars().count()));
        text.push('\n');
    }
    text
}

fn fixture_tree_manifest(root: &Path) -> Result<Vec<(String, String)>, String> {
    let mut manifest = collect_regular_files(root)?
        .into_iter()
        .map(|path| {
            let relative = relative_path(root, &path)?;
            let bytes = fs::read(&path)
                .map_err(|error| format!("could not read fixture gallery artifact: {error}"))?;
            Ok((relative, sha256_hex(&bytes)))
        })
        .collect::<Result<Vec<_>, String>>()?;
    manifest.sort();
    Ok(manifest)
}

fn require_implemented(inventory: &[ScreenSpec]) -> Result<(), String> {
    for screen in inventory {
        if screen.status != ImplementationStatus::Implemented
            || !IMPLEMENTED_RENDERERS.contains(&screen.id)
        {
            return Err(format!(
                "canonical ID has no implemented renderer: {}",
                screen.id
            ));
        }
    }
    Ok(())
}

fn require_selected_implemented(screens: &[&ScreenSpec]) -> Result<(), String> {
    for screen in screens {
        if screen.status != ImplementationStatus::Implemented
            || !IMPLEMENTED_RENDERERS.contains(&screen.id)
        {
            return Err(format!(
                "canonical ID has no implemented renderer: {}",
                screen.id
            ));
        }
    }
    Ok(())
}

fn prepare_roots(plan: &RenderPlan) -> Result<Vec<PreparedRoot>, RenderFailure> {
    let mut canonical = BTreeMap::new();
    for screen in &plan.canonical {
        let audit = audit_screen(screen.id, &format!("canonical.{}", screen.id))
            .map_err(RenderFailure::validation)?;
        insert_audit_artifacts(
            &mut canonical,
            format!("{}/{}", screen.category.as_str(), screen.id),
            &audit,
        )?;
    }

    let mut roots = Vec::new();
    if !canonical.is_empty() {
        roots.push(PreparedRoot {
            kind: RootKind::Canonical,
            root: RootKind::Canonical.path().to_path_buf(),
            artifacts: canonical,
        });
    }
    if let Some(flow) = plan.flow {
        let mut gate = BTreeMap::new();
        for (index, step) in flow.steps.iter().enumerate() {
            let audit = audit_screen(
                step.screen_id,
                &format!("gate.{}.{:02}", flow.name, index + 1),
            )
            .map_err(RenderFailure::validation)?;
            insert_audit_artifacts(
                &mut gate,
                format!("{}/{:02}-{}", flow.name, index + 1, step.screen_id),
                &audit,
            )?;
        }
        roots.push(PreparedRoot {
            kind: RootKind::Gate,
            root: RootKind::Gate.path().to_path_buf(),
            artifacts: gate,
        });
    }
    Ok(roots)
}

fn insert_audit_artifacts(
    artifacts: &mut BTreeMap<String, Vec<u8>>,
    base: String,
    audit: &AuditDocument,
) -> Result<(), RenderFailure> {
    let text = audit.render_txt().into_bytes();
    validate_txt_bytes(&text).map_err(RenderFailure::validation)?;
    insert_artifact(artifacts, format!("{base}.txt"), text)?;

    #[cfg(feature = "png")]
    {
        let png = crate::scenario::render_audit_png(audit).map_err(RenderFailure::validation)?;
        validate_png_bytes(&png).map_err(RenderFailure::validation)?;
        insert_artifact(artifacts, format!("{base}.png"), png)?;
    }
    #[cfg(not(feature = "png"))]
    {
        return Err(RenderFailure::validation(
            "public gallery rendering requires the png feature",
        ));
    }
    Ok(())
}

fn insert_artifact(
    artifacts: &mut BTreeMap<String, Vec<u8>>,
    path: String,
    bytes: Vec<u8>,
) -> Result<(), RenderFailure> {
    if artifacts.insert(path.clone(), bytes).is_some() {
        return Err(RenderFailure::validation(format!(
            "duplicate staged artifact path: {path}"
        )));
    }
    Ok(())
}

fn validate_existing_root(kind: RootKind) -> Result<(), String> {
    let root = kind.path();
    if !root.exists() {
        return Ok(());
    }
    let state = parse_manifest(kind, root)?;
    let files = collect_regular_files(root)?;
    let static_files = kind.static_files().iter().copied().collect::<BTreeSet<_>>();
    for path in files {
        let relative = relative_path(root, &path)?;
        if relative == MANIFEST_FILE || static_files.contains(relative.as_str()) {
            continue;
        }
        let expected_hash = state
            .get(&relative)
            .ok_or_else(|| format!("unknown file in output root {}: {relative}", root.display()))?;
        let bytes = fs::read(&path)
            .map_err(|error| format!("could not read output artifact {relative}: {error}"))?;
        if sha256_hex(&bytes) != *expected_hash {
            return Err(format!(
                "output manifest hash mismatch for {}: {relative}",
                root.display()
            ));
        }
    }
    for path in state.keys() {
        if !root.join(path).is_file() {
            return Err(format!(
                "output manifest references missing artifact in {}: {path}",
                root.display()
            ));
        }
    }
    Ok(())
}

fn parse_manifest(kind: RootKind, root: &Path) -> Result<BTreeMap<String, String>, String> {
    let path = root.join(MANIFEST_FILE);
    if !path.exists() {
        let files = collect_regular_files(root)?;
        let static_files = kind.static_files().iter().copied().collect::<BTreeSet<_>>();
        for file in files {
            let relative = relative_path(root, &file)?;
            if !static_files.contains(relative.as_str()) {
                return Err(format!(
                    "output root {} has artifacts but no ownership manifest",
                    root.display()
                ));
            }
        }
        return Ok(BTreeMap::new());
    }

    let source = fs::read_to_string(&path)
        .map_err(|error| format!("could not read output manifest {}: {error}", path.display()))?;
    let mut lines = source.lines();
    if lines.next() != Some(MANIFEST_HEADER) {
        return Err(format!(
            "invalid output manifest header: {}",
            path.display()
        ));
    }
    let mut entries = BTreeMap::new();
    for line in lines {
        let Some((relative, hash)) = line.split_once('\t') else {
            return Err(format!("invalid output manifest entry: {}", path.display()));
        };
        validate_owned_relative_path(relative)?;
        if hash.len() != 64 || !hash.chars().all(|character| character.is_ascii_hexdigit()) {
            return Err(format!("invalid output manifest hash for {relative}"));
        }
        if entries
            .insert(relative.to_owned(), hash.to_owned())
            .is_some()
        {
            return Err(format!("duplicate output manifest path: {relative}"));
        }
    }
    Ok(entries)
}

fn publish_prepared_roots(
    roots: &[PreparedRoot],
    fail_after_staging: bool,
) -> Result<(), RenderFailure> {
    publish_prepared_roots_with_cleanup(roots, fail_after_staging, cleanup_stages)
}

fn publish_prepared_roots_with_cleanup(
    roots: &[PreparedRoot],
    fail_after_staging: bool,
    cleanup: impl Fn(&[StagedRoot]) -> Result<(), RenderFailure>,
) -> Result<(), RenderFailure> {
    let mut staged = Vec::with_capacity(roots.len());
    for root in roots {
        match stage_root(root) {
            Ok(value) => staged.push(value),
            Err(error) => {
                let _ = cleanup(&staged);
                return Err(error);
            }
        }
    }
    if fail_after_staging {
        cleanup(&staged)?;
        return Err(RenderFailure::validation(
            "injected staged validation failure before publication",
        ));
    }

    publish_staged_roots_with_cleanup(&staged, cleanup)
}

fn publish_staged_roots_with_cleanup(
    staged: &[StagedRoot],
    cleanup: impl Fn(&[StagedRoot]) -> Result<(), RenderFailure>,
) -> Result<(), RenderFailure> {
    let mut exchanged: Vec<&StagedRoot> = Vec::new();
    for staged_root in staged {
        if let Err(error) = exchange_root(&staged_root.root, &staged_root.stage) {
            let mut rollback_errors = Vec::new();
            for exchanged_root in exchanged.iter().rev() {
                if let Err(rollback) = exchange_root(&exchanged_root.root, &exchanged_root.stage) {
                    rollback_errors.push(rollback.to_string());
                }
            }
            let cleanup_error = cleanup(staged).err();
            let detail = if rollback_errors.is_empty() {
                String::new()
            } else {
                format!("; rollback failed: {}", rollback_errors.join("; "))
            };
            let detail = if let Some(cleanup_error) = cleanup_error {
                format!("{detail}; cleanup failed: {}", cleanup_error.message)
            } else {
                detail
            };
            return Err(RenderFailure::io(
                &format!("atomic publication failed{detail}"),
                error,
            ));
        }
        exchanged.push(staged_root);
    }
    cleanup(staged)
}

fn stage_root(prepared: &PreparedRoot) -> Result<StagedRoot, RenderFailure> {
    let root = prepared.root.clone();
    let parent = root.parent().ok_or_else(|| {
        RenderFailure::validation(format!("output root has no parent: {}", root.display()))
    })?;
    let stage = parent.join(format!(
        ".{}.render-staging-{}-{}",
        root.file_name()
            .ok_or_else(|| {
                RenderFailure::validation(format!("output root has no name: {}", root.display()))
            })?
            .to_string_lossy(),
        std::process::id(),
        STAGING_SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir(&stage)
        .map_err(|error| RenderFailure::io("could not create renderer staging root", error))?;

    let result = (|| {
        if root.exists() {
            copy_static_files(prepared.kind, &root, &stage)?;
        }
        let mut owned = if root.exists() && prepared.kind == RootKind::Gate {
            copy_existing_gate_artifacts(&root, &stage, &prepared.artifacts)?
        } else {
            BTreeMap::new()
        };
        for (relative, bytes) in &prepared.artifacts {
            validate_owned_relative_path(relative).map_err(RenderFailure::validation)?;
            let destination = stage.join(relative);
            let parent = destination.parent().ok_or_else(|| {
                RenderFailure::validation(format!("staged artifact has no parent: {relative}"))
            })?;
            fs::create_dir_all(parent).map_err(|error| {
                RenderFailure::io("could not create staged artifact directory", error)
            })?;
            write_new_file(&destination, bytes)?;
            owned.insert(relative.clone(), sha256_hex(bytes));
        }
        write_manifest(&stage, &owned)?;
        validate_existing_root_at(prepared.kind, &stage).map_err(RenderFailure::validation)?;
        sync_tree(&stage)?;
        Ok(())
    })();

    if let Err(error) = result {
        if validate_cleanup_paths(&root, &stage).is_ok() {
            let _ = fs::remove_dir_all(&stage);
        }
        return Err(error);
    }
    Ok(StagedRoot { root, stage })
}

fn copy_existing_gate_artifacts(
    source: &Path,
    destination: &Path,
    replacements: &BTreeMap<String, Vec<u8>>,
) -> Result<BTreeMap<String, String>, RenderFailure> {
    let existing = parse_manifest(RootKind::Gate, source).map_err(RenderFailure::validation)?;
    let replaced_flows = replacements
        .keys()
        .filter_map(|relative| relative.split_once('/').map(|(flow, _)| flow))
        .collect::<BTreeSet<_>>();
    let mut retained = BTreeMap::new();
    for (relative, hash) in existing {
        if replaced_flows
            .iter()
            .any(|flow| relative.starts_with(&format!("{flow}/")))
        {
            continue;
        }
        let source_path = source.join(&relative);
        let destination_path = destination.join(&relative);
        let parent = destination_path.parent().ok_or_else(|| {
            RenderFailure::validation(format!("retained gate artifact has no parent: {relative}"))
        })?;
        fs::create_dir_all(parent).map_err(|error| {
            RenderFailure::io("could not create retained gate artifact directory", error)
        })?;
        fs::copy(&source_path, &destination_path)
            .map_err(|error| RenderFailure::io("could not retain existing gate artifact", error))?;
        retained.insert(relative, hash);
    }
    Ok(retained)
}

fn validate_existing_root_at(kind: RootKind, root: &Path) -> Result<(), String> {
    let original = kind.path();
    let state = parse_manifest(kind, root)?;
    let files = collect_regular_files(root)?;
    let static_files = kind.static_files().iter().copied().collect::<BTreeSet<_>>();
    for path in files {
        let relative = relative_path(root, &path)?;
        if relative == MANIFEST_FILE || static_files.contains(relative.as_str()) {
            continue;
        }
        let hash = state.get(&relative).ok_or_else(|| {
            format!(
                "unknown staged artifact for {}: {relative}",
                original.display()
            )
        })?;
        let bytes = fs::read(&path)
            .map_err(|error| format!("could not read staged artifact {relative}: {error}"))?;
        if sha256_hex(&bytes) != *hash {
            return Err(format!(
                "staged output manifest hash mismatch for {}: {relative}",
                original.display()
            ));
        }
        if relative.ends_with(".txt") {
            validate_txt_bytes(&bytes)?;
        } else if relative.ends_with(".png") {
            validate_png_bytes(&bytes)?;
        } else {
            return Err(format!(
                "staged output has unsupported artifact type: {relative}"
            ));
        }
    }
    for path in state.keys() {
        if !root.join(path).is_file() {
            return Err(format!(
                "staged manifest references missing artifact: {path}"
            ));
        }
    }
    Ok(())
}

fn write_new_file(destination: &Path, bytes: &[u8]) -> Result<(), RenderFailure> {
    if destination.exists() {
        fs::remove_file(destination).map_err(|error| {
            RenderFailure::io(
                &format!(
                    "could not replace staged artifact {}",
                    destination.display()
                ),
                error,
            )
        })?;
    }
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(destination)
        .map_err(|error| RenderFailure::io("could not create staged artifact", error))?;
    file.write_all(bytes)
        .map_err(|error| RenderFailure::io("could not write staged artifact", error))?;
    file.sync_all()
        .map_err(|error| RenderFailure::io("could not finalize staged artifact", error))
}

fn write_manifest(root: &Path, owned: &BTreeMap<String, String>) -> Result<(), RenderFailure> {
    let mut content = String::from(MANIFEST_HEADER);
    content.push('\n');
    for (path, hash) in owned {
        content.push_str(path);
        content.push('\t');
        content.push_str(hash);
        content.push('\n');
    }
    let destination = root.join(MANIFEST_FILE);
    if destination.exists() {
        fs::remove_file(&destination).map_err(|error| {
            RenderFailure::io("could not replace staged output manifest", error)
        })?;
    }
    write_new_file(&destination, content.as_bytes())
}

fn copy_static_files(
    kind: RootKind,
    source: &Path,
    destination: &Path,
) -> Result<(), RenderFailure> {
    for relative in kind.static_files() {
        let source_path = source.join(relative);
        if !source_path.exists() {
            continue;
        }
        let destination_path = destination.join(relative);
        let parent = destination_path.parent().ok_or_else(|| {
            RenderFailure::validation(format!("static output path has no parent: {relative}"))
        })?;
        fs::create_dir_all(parent).map_err(|error| {
            RenderFailure::io("could not create staged static directory", error)
        })?;
        fs::copy(&source_path, &destination_path).map_err(|error| {
            RenderFailure::io("could not copy static output file to staging", error)
        })?;
    }
    Ok(())
}

fn sync_tree(root: &Path) -> Result<(), RenderFailure> {
    for file in collect_regular_files(root).map_err(RenderFailure::validation)? {
        File::open(&file)
            .and_then(|file| file.sync_all())
            .map_err(|error| RenderFailure::io("could not sync staged artifact", error))?;
    }
    File::open(root)
        .and_then(|directory| directory.sync_all())
        .map_err(|error| RenderFailure::io("could not sync staged output root", error))
}

fn collect_regular_files(root: &Path) -> Result<Vec<PathBuf>, String> {
    let mut files = Vec::new();
    if !root.exists() {
        return Ok(files);
    }
    collect_regular_files_into(root, &mut files)?;
    files.sort();
    Ok(files)
}

fn collect_regular_files_into(root: &Path, files: &mut Vec<PathBuf>) -> Result<(), String> {
    for entry in fs::read_dir(root)
        .map_err(|error| format!("could not read output root {}: {error}", root.display()))?
    {
        let entry = entry.map_err(|error| format!("could not read output entry: {error}"))?;
        let path = entry.path();
        let file_type = entry.file_type().map_err(|error| {
            format!("could not inspect output entry {}: {error}", path.display())
        })?;
        if file_type.is_dir() {
            collect_regular_files_into(&path, files)?;
        } else if file_type.is_file() {
            files.push(path);
        } else {
            return Err(format!(
                "output root contains unsupported entry: {}",
                path.display()
            ));
        }
    }
    Ok(())
}

fn relative_path(root: &Path, path: &Path) -> Result<String, String> {
    let relative = path
        .strip_prefix(root)
        .map_err(|error| format!("could not compute output relative path: {error}"))?;
    let relative = relative
        .to_str()
        .ok_or_else(|| format!("output path is not UTF-8: {}", relative.display()))?
        .replace('\\', "/");
    let candidate = Path::new(&relative);
    if relative.is_empty()
        || candidate.is_absolute()
        || candidate.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        return Err(format!("invalid output relative path: {relative}"));
    }
    Ok(relative)
}

fn validate_owned_relative_path(path: &str) -> Result<(), String> {
    let candidate = Path::new(path);
    if path.is_empty()
        || candidate.is_absolute()
        || candidate.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
        || !candidate
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension == "txt" || extension == "png")
    {
        return Err(format!("invalid renderer-owned artifact path: {path}"));
    }
    Ok(())
}

fn validate_txt_bytes(bytes: &[u8]) -> Result<(), String> {
    let text = std::str::from_utf8(bytes).map_err(|error| format!("TXT is not UTF-8: {error}"))?;
    let rows = text.lines().collect::<Vec<_>>();
    if rows.len() != 40
        || !text.ends_with('\n')
        || rows.iter().any(|row| row.chars().count() != 120)
    {
        return Err(
            "TXT artifact must be exactly 40 newline-terminated 120-column rows".to_owned(),
        );
    }
    Ok(())
}

#[cfg(feature = "png")]
fn validate_png_bytes(bytes: &[u8]) -> Result<(), String> {
    use image::ColorType;

    let image = image::load_from_memory(bytes).map_err(|error| format!("invalid PNG: {error}"))?;
    if image.width() != 1200 || image.height() != 840 || image.color() != ColorType::Rgb8 {
        return Err("PNG artifact must be a 1200x840 RGB image".to_owned());
    }
    Ok(())
}

#[cfg(not(feature = "png"))]
fn validate_png_bytes(_bytes: &[u8]) -> Result<(), String> {
    Err("PNG validation requires the png feature".to_owned())
}

fn exchange_root(root: &Path, stage: &Path) -> io::Result<()> {
    if !root.exists() {
        return fs::rename(stage, root);
    }
    #[cfg(target_os = "linux")]
    {
        let root = CString::new(root.as_os_str().as_encoded_bytes())
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "root contains NUL"))?;
        let stage = CString::new(stage.as_os_str().as_encoded_bytes())
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "stage contains NUL"))?;
        // SAFETY: both strings are NUL-terminated paths, AT_FDCWD makes them
        // absolute paths, and RENAME_EXCHANGE atomically swaps same-filesystem
        // root directories without following user-controlled cleanup paths.
        let result = unsafe { renameat2(-100, root.as_ptr(), -100, stage.as_ptr(), 2) };
        if result == 0 {
            Ok(())
        } else {
            Err(io::Error::last_os_error())
        }
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = stage;
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "atomic renderer publication requires Linux renameat2",
        ))
    }
}

fn cleanup_stages(staged: &[StagedRoot]) -> Result<(), RenderFailure> {
    let mut first_error = None;
    for stage in staged {
        if let Err(error) = validate_cleanup_paths(&stage.root, &stage.stage) {
            if first_error.is_none() {
                first_error = Some(RenderFailure::validation(error));
            }
            continue;
        }
        if !stage.stage.exists() {
            continue;
        }
        if let Err(error) = fs::remove_dir_all(&stage.stage) {
            if first_error.is_none() {
                first_error = Some(RenderFailure::io(
                    &format!(
                        "could not clean up stale renderer staging root {}",
                        stage.stage.display()
                    ),
                    error,
                ));
            }
        }
    }
    first_error.map_or(Ok(()), Err)
}

fn transition_kind_name(kind: TransitionKind) -> &'static str {
    match kind {
        TransitionKind::Direct => "direct",
        TransitionKind::Automatic => "automatic",
        TransitionKind::Summarized => "summarized",
        TransitionKind::IllustrativeDetour => "illustrative_detour",
    }
}

fn audit_collection(audits: Vec<AuditDocument>) -> String {
    let mut output = format!(
        "{{\"document_type\":\"typed-render-audit-collection\",\"schema_version\":1,\"screen_count\":{}",
        audits.len()
    );
    output.push_str(",\"screens\":[");
    for (index, audit) in audits.iter().enumerate() {
        if index > 0 {
            output.push(',');
        }
        output.push_str(&audit.to_json());
    }
    output.push_str("]}");
    output
}

fn json_field(output: &mut String, name: &str, value: &str) {
    output.push(',');
    output.push_str(&json_string(name));
    output.push(':');
    output.push_str(&json_string(value));
}

fn json_string(value: &str) -> String {
    let mut output = String::from("\"");
    for character in value.chars() {
        match character {
            '"' => output.push_str("\\\""),
            '\\' => output.push_str("\\\\"),
            '\n' => output.push_str("\\n"),
            '\r' => output.push_str("\\r"),
            '\t' => output.push_str("\\t"),
            character if character.is_control() => {
                use std::fmt::Write as _;
                let _ = write!(output, "\\u{:04x}", character as u32);
            }
            character => output.push(character),
        }
    }
    output.push('"');
    output
}

unsafe extern "C" {
    fn renameat2(
        olddirfd: i32,
        oldpath: *const c_char,
        newdirfd: i32,
        newpath: *const c_char,
        flags: u32,
    ) -> i32;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temporary_root(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "mockup-harness-v3-renderer-{name}-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        root
    }

    #[test]
    fn partial_implemented_selector_is_valid_but_all_fails_closed() {
        let implemented = preflight_render(&PublicSelector::Implemented)
            .expect("a partial implementation set remains renderable");
        assert_eq!(implemented.canonical_ids().count(), 17);
        assert!(preflight_render(&PublicSelector::All)
            .expect_err("full render must reject missing renderers")
            .contains("chat-long-scroll"));
    }

    #[test]
    fn coordinator_busy_has_no_staging_side_effect() {
        let parent = temporary_root("lock");
        let root = parent.join("output");
        fs::create_dir(&root).unwrap();
        let first = CoordinatorLock::acquire(&root).unwrap();
        let second = CoordinatorLock::acquire(&root).expect_err("second coordinator must fail");
        assert_eq!(second.exit, RenderExit::Busy);
        assert!(fs::read_dir(&root).unwrap().next().is_none());
        drop(first);
        assert!(!parent.join(".output.render.lock").exists());
        fs::remove_dir_all(parent).unwrap();
    }

    #[test]
    fn staged_validation_failure_preserves_prior_complete_root() {
        let parent = temporary_root("atomic");
        let root = parent.join("output");
        fs::create_dir(&root).unwrap();
        fs::write(root.join("old.txt"), "previous complete gallery\n").unwrap();

        let before = fs::read(root.join("old.txt")).unwrap();
        let stage = parent.join(".output.render-staging-test");
        fs::create_dir(&stage).unwrap();
        fs::write(stage.join("new.txt"), "new gallery\n").unwrap();
        let roots = [StagedRoot {
            root: root.clone(),
            stage,
        }];
        let result = publish_staged_for_test(&roots);
        assert_eq!(result.exit, RenderExit::Validation);
        assert_eq!(fs::read(root.join("old.txt")).unwrap(), before);
        assert!(!parent.join(".output.render-staging-test").exists());
        fs::remove_dir_all(parent).unwrap();
    }

    #[test]
    fn atomic_exchange_replaces_only_a_complete_staging_root() {
        let parent = temporary_root("exchange");
        let root = parent.join("output");
        let stage = parent.join(".output.render-staging-test");
        fs::create_dir(&root).unwrap();
        fs::create_dir(&stage).unwrap();
        fs::write(root.join("previous.txt"), "previous complete gallery\n").unwrap();
        fs::write(stage.join("published.txt"), "new complete gallery\n").unwrap();
        fs::create_dir(stage.join("session")).unwrap();
        fs::write(
            stage.join("session").join("complete.txt"),
            "second published artifact\n",
        )
        .unwrap();

        exchange_root(&root, &stage).expect("staging and output roots share one filesystem");

        assert_eq!(
            fs::read_to_string(root.join("published.txt")).unwrap(),
            "new complete gallery\n"
        );
        assert_eq!(
            fs::read_to_string(root.join("session").join("complete.txt")).unwrap(),
            "second published artifact\n"
        );
        assert!(!root.join("previous.txt").exists());
        assert_eq!(
            fs::read_to_string(stage.join("previous.txt")).unwrap(),
            "previous complete gallery\n"
        );
        assert!(!stage.join("published.txt").exists());
        fs::remove_dir_all(parent).unwrap();
    }

    #[test]
    fn cleanup_failure_after_exchange_is_reported_without_losing_published_root() {
        let parent = temporary_root("cleanup-failure");
        let root = parent.join("output");
        let stage = parent.join(".output.render-staging-test");
        fs::create_dir(&root).unwrap();
        fs::create_dir(&stage).unwrap();
        fs::write(root.join("previous.txt"), "previous complete gallery\n").unwrap();
        fs::write(stage.join("published.txt"), "new complete gallery\n").unwrap();
        let roots = [StagedRoot {
            root: root.clone(),
            stage: stage.clone(),
        }];

        let result = publish_staged_roots_with_cleanup(&roots, |staged| {
            Err(RenderFailure::io(
                &format!(
                    "could not clean up stale renderer staging root {}",
                    staged[0].stage.display()
                ),
                io::Error::new(io::ErrorKind::PermissionDenied, "injected cleanup failure"),
            ))
        });

        let error = result.expect_err("cleanup failure must fail publication");
        assert_eq!(error.exit, RenderExit::Io);
        assert!(error.message.contains(&stage.display().to_string()));
        assert_eq!(
            fs::read_to_string(root.join("published.txt")).unwrap(),
            "new complete gallery\n"
        );
        assert!(!root.join("previous.txt").exists());
        assert_eq!(
            fs::read_to_string(stage.join("previous.txt")).unwrap(),
            "previous complete gallery\n"
        );
        assert!(
            stage.exists(),
            "the failed cleanup must leave a named stale root"
        );

        cleanup_stages(&roots).expect("the test cleanup should remove the stale root");
        fs::remove_dir_all(parent).unwrap();
    }

    fn publish_staged_for_test(staged: &[StagedRoot]) -> RenderFailure {
        if staged.is_empty() {
            return RenderFailure::validation("test setup requires a stage");
        }
        cleanup_stages(staged).unwrap();
        RenderFailure::validation("injected staged validation failure before publication")
    }
}
