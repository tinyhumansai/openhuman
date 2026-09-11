//! Tests for the Node toolchain client.
//!
//! What is worth testing here is the adaptation, not the resolution: the module
//! owns probing, downloading, verifying, and installing, and it has its own
//! suite for all of it. These cover the seam — how a module answer becomes a
//! [`ResolvedNode`], and what happens when the host has Node turned off.

use std::sync::Arc;

use tinyruntime_bus::{Language, ResolvedRuntime, RuntimeLayout, RuntimeSource};

use super::{NodeBootstrap, NodeSource, ResolvedNode};
use crate::openhuman::config::Config;

/// A module resolution carrying `executables`.
fn resolution(executables: &[(&str, &str)]) -> ResolvedRuntime {
    let mut layout = RuntimeLayout::new("22.11.0", "/cache/node-v22.11.0/bin");
    for (name, path) in executables {
        layout = layout.with_executable(*name, *path);
    }
    ResolvedRuntime::from_layout(Language::nodejs(), RuntimeSource::Managed, layout)
}

#[test]
fn a_resolution_becomes_the_paths_callers_name() {
    let adapted = ResolvedNode::from_module(&resolution(&[
        ("node", "/cache/node-v22.11.0/bin/node"),
        ("npm", "/cache/node-v22.11.0/bin/npm"),
    ]))
    .expect("a toolchain reporting node adapts");

    assert_eq!(
        adapted.node_bin,
        std::path::Path::new("/cache/node-v22.11.0/bin/node")
    );
    assert_eq!(
        adapted.npm_bin,
        std::path::Path::new("/cache/node-v22.11.0/bin/npm")
    );
    assert_eq!(
        adapted.bin_dir,
        std::path::Path::new("/cache/node-v22.11.0/bin")
    );
    assert_eq!(adapted.version, "22.11.0");
    assert_eq!(adapted.source, NodeSource::Managed);
}

#[test]
fn a_toolchain_without_npm_still_resolves() {
    // Refusing here would take `node_exec` down along with `npm_exec`, for a
    // toolchain that runs `node` perfectly well.
    let adapted = ResolvedNode::from_module(&resolution(&[("node", "/usr/bin/node")]))
        .expect("a toolchain without npm is still usable");
    assert_eq!(adapted.node_bin, std::path::Path::new("/usr/bin/node"));
    assert!(
        adapted
            .npm_bin
            .ends_with(if cfg!(windows) { "npm.cmd" } else { "npm" }),
        "npm was not derived: {}",
        adapted.npm_bin.display()
    );
}

#[test]
fn a_toolchain_without_node_is_refused() {
    // Deriving `node` too would turn a broken install into a spawn failure much
    // later, with nothing pointing at the cause.
    let error = ResolvedNode::from_module(&resolution(&[("npm", "/usr/bin/npm")]))
        .expect_err("a toolchain with no node is not a toolchain");
    assert!(error.to_string().contains("`node`"), "got `{error}`");
}

#[test]
fn the_version_never_keeps_a_leading_v() {
    // Callers render this and compare it; `v22.11.0` and `22.11.0` reaching
    // different call sites is exactly the drift the normalisation prevents.
    let mut resolved = resolution(&[("node", "/usr/bin/node")]);
    resolved.version = "v22.11.0".to_string();
    let adapted = ResolvedNode::from_module(&resolved).expect("adapts");
    assert_eq!(adapted.version, "22.11.0");
}

#[test]
fn a_system_toolchain_is_reported_as_one() {
    let mut resolved = resolution(&[("node", "/usr/bin/node")]);
    resolved.source = RuntimeSource::System;
    assert_eq!(
        ResolvedNode::from_module(&resolved).expect("adapts").source,
        NodeSource::System
    );
}

#[tokio::test]
async fn a_disabled_runtime_refuses_before_reaching_the_bus() {
    let mut config = Config::default();
    config.node.enabled = false;
    let bootstrap = NodeBootstrap::new(Arc::new(config));

    let error = bootstrap.resolve().await.expect_err("node is off");
    assert!(error.to_string().contains("disabled"), "got `{error}`");
    assert!(
        bootstrap.probe_installed().await.is_none(),
        "a disabled runtime has nothing provisioned"
    );
}

#[test]
fn nothing_is_cached_before_the_first_resolution() {
    // The shell consults this on every command; answering with a stale or
    // invented toolchain would put the wrong directory on PATH.
    let bootstrap = NodeBootstrap::new(Arc::new(Config::default()));
    assert!(bootstrap.try_cached().is_none());
}
