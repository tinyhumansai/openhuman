//! A callback-bound module served from its pinned release.
//!
//! TinyMCP, TinyEventQueues, TinyChannels, TinySecurity and TinyComputer take
//! host callbacks when they load. On a host without `OPENHUMAN_MODULE_PATH`
//! (every shipped build and embedder) their source is the pinned release, so
//! the callback load has to come from the installer bundle or the release
//! cache, digest-checked, and never require a local-file override.
//!
//! The module here is `tests/fixtures/storage_probe_module`, built by the
//! storage host's cdylib tests and staged as a release: the archive and its
//! extracted library side by side under `<root>/<id>/<version>/<host key>/`,
//! the layout TinyBus fills after a verified download. Its record pins the
//! archive and library digests, so a wrong digest is a refusal and not a load.
//! It is loaded through [`ensure_loaded_with_storage_callback`], the path the
//! storage-backed modules take; TinySecurity and TinyComputer share its
//! `callback_load` helpers.
//!
//! Each test runs in its own process: the module runtime, the resolution table
//! and the probe's bus name are process-wide, and TinyBus never unloads a
//! library.

use std::collections::HashMap;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use super::{ensure_loaded_with_storage_callback, report, runtime};
use crate::config::Config;
use tinybus::module::{
    artifact_dir, platform::host_candidates, LinkedRegistry, ModuleSource as ResolverSource,
    ReleaseAsset, ReleaseRecord, Resolver, ResolverInputs,
};

const ID: &str = "callbackprobe";
const VERSION: &str = "0.0.1";
const ARCHIVE: &str = "callbackprobe-module-0.0.1.tar.gz";
const BUS_NAME: &str = "ai.tinyhumans.test.StorageProbe";
/// The module SDK derives the object path from the bus name.
const OBJECT_PATH: &str = "/ai/tinyhumans/test/StorageProbe";
const WRONG_DIGEST: &str = "0000000000000000000000000000000000000000000000000000000000000000";

/// Re-run `test` alone in a fresh process; `true` means this is that process.
fn isolated(test: &str) -> bool {
    const ISOLATED: &str = "OPENHUMAN_TEST_ISOLATED_CALLBACK_RELEASE";
    if std::env::var_os(ISOLATED).is_some() {
        return true;
    }
    // Build the fixture here so the child does not race its own cargo build.
    let _ = probe_library();
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            &format!("modules::ops::callback_release_tests::{test}"),
            "--test-threads=1",
        ])
        .env(ISOLATED, "1")
        .output()
        .expect("run the callback release test in a fresh process");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success() && stdout.contains("1 passed"),
        "isolated callback release test failed\nstdout:\n{stdout}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    false
}

fn probe_library() -> &'static Path {
    crate::modules::storage_host::tests::cdylib::probe_artifact()
}

fn library_name() -> String {
    probe_library()
        .file_name()
        .and_then(|name| name.to_str())
        .expect("probe library name")
        .to_owned()
}

/// A private, canonical directory: TinyBus refuses module roots that others
/// can write to or that sit beneath a symlinked ancestor.
fn private_root() -> tempfile::TempDir {
    let root = tempfile::tempdir_in(std::env::temp_dir().canonicalize().unwrap()).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    root
}

/// The probe library beside a `modules.toml` pinning it, in `directory`.
/// Returns the library's path and SHA-256.
fn stage_library(directory: &Path) -> (PathBuf, String) {
    std::fs::create_dir_all(directory).unwrap();
    let library = directory.join(library_name());
    std::fs::copy(probe_library(), &library).unwrap();
    let digest = tinybus::module::sha256_file(&library).unwrap();
    std::fs::write(
        directory.join("modules.toml"),
        format!("{:?} = {digest:?}\n", library_name()),
    )
    .unwrap();
    (library, digest)
}

/// A verified release under `root`: the archive TinyBus checked and the
/// library it extracted. Returns the archive's and the library's SHA-256.
fn stage_release(root: &Path) -> (String, String) {
    let directory = artifact_dir(root, ID, VERSION, &host_candidates()[0]).unwrap();
    let (library, library_digest) = stage_library(&directory);
    let archive = directory.join(ARCHIVE);
    let mut builder = tar::Builder::new(flate2::write::GzEncoder::new(
        std::fs::File::create(&archive).unwrap(),
        flate2::Compression::default(),
    ));
    builder
        .append_path_with_name(&library, library_name())
        .unwrap();
    builder
        .append_path_with_name(directory.join("modules.toml"), "modules.toml")
        .unwrap();
    builder
        .into_inner()
        .unwrap()
        .finish()
        .unwrap()
        .flush()
        .unwrap();
    (
        tinybus::module::sha256_file(&archive).unwrap(),
        library_digest,
    )
}

/// The registry record pinning `archive_sha256` and `library_sha256`.
fn record(archive_sha256: &str, library_sha256: &str) -> &'static ReleaseRecord {
    let host_key: &'static str = Box::leak(host_candidates()[0].clone().into_boxed_str());
    let sha256: &'static str = Box::leak(archive_sha256.to_owned().into_boxed_str());
    let library_sha256: &'static str = Box::leak(library_sha256.to_owned().into_boxed_str());
    let assets: &'static [ReleaseAsset<'static>] = Box::leak(Box::new([ReleaseAsset {
        host_key,
        archive: ARCHIVE,
        sha256,
        library_sha256: Some(library_sha256),
    }]));
    Box::leak(Box::new(ReleaseRecord {
        id: ID,
        bus_name: BUS_NAME,
        object_path: OBJECT_PATH,
        version: VERSION,
        release_url: "https://github.com/tinyhumansai/openhuman/releases/tag/v0.0.1",
        assets,
    }))
}

/// A resolver over `record` alone, offline, shaped like [`super::resolver`]'s.
fn resolver(
    record: &'static ReleaseRecord,
    install_dir: &Path,
    bundled_dir: Option<&Path>,
    sources: HashMap<String, ResolverSource>,
) -> Resolver {
    Resolver::new(ResolverInputs {
        enabled: true,
        install_dir: Some(install_dir.to_path_buf()),
        allow_download: false,
        bundled_dir: bundled_dir.map(Path::to_path_buf),
        sources,
        linked: Arc::new(LinkedRegistry::new()),
        search_paths: None,
        search_skip: Vec::new(),
        gate: None,
        on_resolved: None,
        records: vec![record],
        terminal_marker: tinytools::status::MODULE_FAULT_MARKER.to_string(),
        runtime: Arc::new(|| Box::pin(runtime::runtime())),
        module_config: Arc::new(|_| serde_json::Value::Null),
        async_module_config: None,
        reporter: report::reporter(),
    })
}

/// Whether the probe is admitted on the module bus and retains the storage
/// callback binding its load prepared.
async fn serving_with_callbacks(resolver: &Resolver) -> bool {
    let Some(prepared) = super::STORAGE_CALLBACK_LOADS
        .lock()
        .await
        .get(ID)
        .map(|(binding, _served)| Arc::clone(binding))
    else {
        return false;
    };
    let host = runtime::runtime().await.unwrap().host();
    if !host
        .list()
        .iter()
        .any(|info| info.manifest.bus_name.as_str() == BUS_NAME)
    {
        return false;
    }
    // A loaded module answers with its retained binding and prepares nothing.
    resolver
        .ensure_loaded_with_host_callback_factory(ID, |_attempt| async {
            Err::<(Arc<tinybus::module::HostCallbackBinding>, serde_json::Value), _>(
                "nothing to prepare for a loaded module".to_owned(),
            )
        })
        .await
        .is_ok_and(|retained| retained.unique_name() == prepared.unique_name())
}

#[tokio::test(flavor = "multi_thread")]
async fn a_callback_module_in_the_installer_bundle_loads_with_callbacks() {
    if !isolated("a_callback_module_in_the_installer_bundle_loads_with_callbacks") {
        return;
    }
    let bundled = private_root();
    let cache = private_root();
    let (archive, library) = stage_release(bundled.path());
    let resolver = resolver(
        record(&archive, &library),
        cache.path(),
        Some(bundled.path()),
        HashMap::new(),
    );

    ensure_loaded_with_storage_callback(&resolver, &Config::default(), ID)
        .await
        .expect("a bundled release loads with its storage callback");
    assert!(serving_with_callbacks(&resolver).await);
    // A later load confirms the retained binding instead of re-presenting it.
    ensure_loaded_with_storage_callback(&resolver, &Config::default(), ID)
        .await
        .expect("a later load reuses the admitted binding");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_callback_module_in_the_release_cache_loads_with_callbacks() {
    if !isolated("a_callback_module_in_the_release_cache_loads_with_callbacks") {
        return;
    }
    let cache = private_root();
    let (archive, library) = stage_release(cache.path());
    let resolver = resolver(
        record(&archive, &library),
        cache.path(),
        None,
        HashMap::new(),
    );

    ensure_loaded_with_storage_callback(&resolver, &Config::default(), ID)
        .await
        .expect("a cached release loads with its storage callback");
    assert!(serving_with_callbacks(&resolver).await);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_release_served_callback_module_is_confirmed_by_its_admitted_binding() {
    if !isolated("a_release_served_callback_module_is_confirmed_by_its_admitted_binding") {
        return;
    }
    // Every later TinySecurity and TinyComputer load re-checks the binding its
    // first load admitted. On a release-served module that check must answer
    // from the retained binding, never ask for a local-file source.
    let cache = private_root();
    let (archive, library) = stage_release(cache.path());
    let resolver = resolver(
        record(&archive, &library),
        cache.path(),
        None,
        HashMap::new(),
    );
    ensure_loaded_with_storage_callback(&resolver, &Config::default(), ID)
        .await
        .expect("a cached release loads with its storage callback");
    let admitted = super::STORAGE_CALLBACK_LOADS
        .lock()
        .await
        .get(ID)
        .map(|(binding, _served)| Arc::clone(binding))
        .expect("the admitted binding is kept");

    super::callback_load::confirm(&resolver, ID, &admitted)
        .await
        .expect("the admitted binding is confirmed");
    let runtime = runtime::runtime().await.unwrap();
    let (other, _served) = crate::modules::storage_host::prepare_module_callbacks(runtime)
        .await
        .unwrap();
    let error = super::callback_load::confirm(&resolver, ID, &other)
        .await
        .unwrap_err();
    assert!(error.contains("different callback binding"), "{error}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_callback_load_whose_caller_gave_up_is_found_by_the_next_caller() {
    if !isolated("a_callback_load_whose_caller_gave_up_is_found_by_the_next_caller") {
        return;
    }
    // `ensure_loaded_within` bounds only the wait. A caller that gives up
    // must not strand the admitted binding's host state: TinyBus keeps
    // resolving, and the next caller has to find the module serving.
    let cache = private_root();
    let (archive, library) = stage_release(cache.path());
    let resolver = resolver(
        record(&archive, &library),
        cache.path(),
        None,
        HashMap::new(),
    );
    // The bus is not started yet: the caller gives up before its host
    // callbacks are installed, which must not cancel the load either.

    let gave_up = tokio::time::timeout(
        std::time::Duration::ZERO,
        ensure_loaded_with_storage_callback(&resolver, &Config::default(), ID),
    )
    .await;
    assert!(gave_up.is_err(), "the first caller stops waiting");

    ensure_loaded_with_storage_callback(&resolver, &Config::default(), ID)
        .await
        .expect("the next caller finds the load the first one started");
    assert!(serving_with_callbacks(&resolver).await);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_bundled_archive_that_does_not_match_its_pin_is_refused() {
    if !isolated("a_bundled_archive_that_does_not_match_its_pin_is_refused") {
        return;
    }
    let bundled = private_root();
    let cache = private_root();
    let (_archive, library) = stage_release(bundled.path());
    let resolver = resolver(
        record(WRONG_DIGEST, &library),
        cache.path(),
        Some(bundled.path()),
        HashMap::new(),
    );

    let error = ensure_loaded_with_storage_callback(&resolver, &Config::default(), ID)
        .await
        .unwrap_err();
    // A bundled archive that fails its pin is a broken installation: refused,
    // never replaced by the release cache or a download.
    assert!(error.contains("installer bundle"), "{error}");
    assert!(error.contains("not cached"), "{error}");
    assert!(!serving_with_callbacks(&resolver).await);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_cached_library_that_does_not_match_its_pin_is_refused() {
    if !isolated("a_cached_library_that_does_not_match_its_pin_is_refused") {
        return;
    }
    let cache = private_root();
    let (archive, _library) = stage_release(cache.path());
    let resolver = resolver(
        record(&archive, WRONG_DIGEST),
        cache.path(),
        None,
        HashMap::new(),
    );

    let error = ensure_loaded_with_storage_callback(&resolver, &Config::default(), ID)
        .await
        .unwrap_err();
    assert!(
        error.contains("extracted library hash does not match its release checksum"),
        "{error}"
    );
    assert!(!serving_with_callbacks(&resolver).await);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_local_file_override_wins_over_the_release() {
    if !isolated("a_local_file_override_wins_over_the_release") {
        return;
    }
    // The cached release does not match its pin, so a load can only succeed
    // from the override. `ops::resolver` turns a `path` override, a source
    // build and an `OPENHUMAN_MODULE_PATH` artifact into this source.
    let cache = private_root();
    let (_archive, library) = stage_release(cache.path());
    let local = private_root();
    let (path, _) = stage_library(local.path());
    let sources = HashMap::from([(
        ID.to_owned(),
        ResolverSource::LocalFile {
            path: path.to_string_lossy().into_owned(),
        },
    )]);
    let resolver = resolver(record(WRONG_DIGEST, &library), cache.path(), None, sources);

    ensure_loaded_with_storage_callback(&resolver, &Config::default(), ID)
        .await
        .expect("the override loads with its storage callback");
    assert!(serving_with_callbacks(&resolver).await);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_detached_callback_load_runs_under_the_callers_context() {
    use crate::core::runtime::{ContextOverlay, CoreContext, DomainSet};

    // TinySecurity's first load prepares its secret scope inside the detached
    // work, and that reads the ambient context (profile, agent, storage
    // scope). The task on the module runtime must act for its caller, not
    // for the process default.
    let caller = CoreContext::for_test(DomainSet::full(), None).derive_with(
        ContextOverlay::new(Config::default(), DomainSet::full(), Default::default())
            .session_agent("callback-load-caller"),
    );
    let cache = private_root();
    let resolver = resolver(
        record(WRONG_DIGEST, WRONG_DIGEST),
        cache.path(),
        None,
        HashMap::new(),
    );

    let seen = CoreContext::scope(
        caller,
        super::callback_load::detached(&resolver, async {
            Ok(CoreContext::current().and_then(|ctx| ctx.session_agent().map(str::to_owned)))
        }),
    )
    .await
    .expect("the detached work runs");
    assert_eq!(seen.as_deref(), Some("callback-load-caller"));
}
