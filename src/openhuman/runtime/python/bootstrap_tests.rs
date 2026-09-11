//! Tests for the Python interpreter client.
//!
//! The module owns discovery, selection, download, and install, and tests all of
//! it in its own repository. These cover the seam: how a module answer becomes a
//! [`ResolvedPython`], and what happens when the host has Python turned off.

use std::sync::Arc;

use tinyruntime_bus::{Language, ResolvedRuntime, RuntimeLayout, RuntimeSource};

use super::{PythonBootstrap, PythonSource, ResolvedPython};
use crate::openhuman::config::Config;

/// A module resolution carrying `executables`.
fn resolution(executables: &[(&str, &str)]) -> ResolvedRuntime {
    let mut layout = RuntimeLayout::new("3.12.4", "/cache/cpython-3.12.4/python/bin");
    for (name, path) in executables {
        layout = layout.with_executable(*name, *path);
    }
    ResolvedRuntime::from_layout(Language::python(), RuntimeSource::Managed, layout)
}

#[test]
fn a_resolution_becomes_the_paths_callers_name() {
    let adapted = ResolvedPython::from_module(&resolution(&[
        ("python", "/cache/cpython-3.12.4/python/bin/python3"),
        ("pip", "/cache/cpython-3.12.4/python/bin/pip3"),
    ]))
    .expect("a toolchain reporting an interpreter adapts");

    assert_eq!(
        adapted.python_bin,
        std::path::Path::new("/cache/cpython-3.12.4/python/bin/python3")
    );
    assert_eq!(adapted.version, "3.12.4");
    assert_eq!(adapted.source, PythonSource::Managed);
}

#[test]
fn an_install_without_an_interpreter_is_refused() {
    // Unlike npm for Node, there is nothing to fall back to: the interpreter is
    // the toolchain.
    let error = ResolvedPython::from_module(&resolution(&[("pip", "/usr/bin/pip3")]))
        .expect_err("an install with no interpreter is not one");
    assert!(error.to_string().contains("interpreter"), "got `{error}`");
}

#[test]
fn an_install_without_pip_still_resolves() {
    let adapted = ResolvedPython::from_module(&resolution(&[("python", "/usr/bin/python3")]))
        .expect("an interpreter without pip is still an interpreter");
    assert_eq!(adapted.python_bin, std::path::Path::new("/usr/bin/python3"));
}

#[test]
fn a_system_interpreter_is_reported_as_one() {
    let mut resolved = resolution(&[("python", "/usr/bin/python3")]);
    resolved.source = RuntimeSource::System;
    assert_eq!(
        ResolvedPython::from_module(&resolved)
            .expect("adapts")
            .source,
        PythonSource::System
    );
}

#[tokio::test]
async fn a_disabled_runtime_refuses_before_reaching_the_bus() {
    let mut config = Config::default();
    config.runtime_python.enabled = false;
    let bootstrap = PythonBootstrap::new(Arc::new(config));

    let error = bootstrap.resolve().await.expect_err("python is off");
    assert!(error.to_string().contains("disabled"), "got `{error}`");
    assert!(bootstrap.probe_installed().await.is_none());
}

#[test]
fn nothing_is_cached_before_the_first_resolution() {
    let bootstrap = PythonBootstrap::new(Arc::new(Config::default()));
    assert!(bootstrap.try_cached().is_none());
}
