// Copyright 2022 - 2025 Zed Industries, Inc.
// SPDX-License-Identifier: Apache-2.0
//
// Copied from Zed's `gpui_tokio` crate, `crates/gpui_tokio/src/gpui_tokio.rs`
// at zed-industries/zed@93d7456 (the snapshot gpui-pre 0.3.9 is taken from).
// It isn't published on crates.io. Licensed under the Apache License 2.0; see
// LICENSE-APACHE in this crate. Changes from the original:
// - `gpui_util::defer` is replaced by the local `Defer` guard below.
// - `spawn_result` is left out (it needs `anyhow`, which tt-git doesn't use).
// - Rustdoc on the items that had none, and `#[allow(dead_code)]` on what
//   tt-git doesn't call yet.

//! Runs tokio next to GPUI's executor: tokio work started through
//! [`Tokio::spawn`] comes back as a GPUI [`Task`].

use std::future::Future;

use gpui::{App, AppContext, Global, ReadGlobal, Task};

pub use tokio::task::JoinError;

/// Initializes the Tokio wrapper using a new Tokio runtime with 2 worker threads.
///
/// If you need more threads (or access to the runtime outside of GPUI), you can create the runtime
/// yourself and pass a Handle to `init_from_handle`.
pub fn init(cx: &mut App) {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        // Since we now have two executors, let's try to keep our footprint small
        .worker_threads(2)
        .enable_all()
        .build()
        .expect("Failed to initialize Tokio");

    let handle = runtime.handle().clone();
    cx.set_global(GlobalTokio {
        owned_runtime: Some(runtime),
        handle,
    });
}

/// Initializes the Tokio wrapper using a Tokio runtime handle.
pub fn init_from_handle(cx: &mut App, handle: tokio::runtime::Handle) {
    cx.set_global(GlobalTokio {
        owned_runtime: None,
        handle,
    });
}

struct GlobalTokio {
    owned_runtime: Option<tokio::runtime::Runtime>,
    handle: tokio::runtime::Handle,
}

impl Global for GlobalTokio {}

impl Drop for GlobalTokio {
    fn drop(&mut self) {
        if let Some(runtime) = self.owned_runtime.take() {
            runtime.shutdown_background();
        }
    }
}

/// Starts tokio work from GPUI.
pub struct Tokio {}

impl Tokio {
    /// Spawns the given future on Tokio's thread pool, and returns it via a GPUI task
    /// Note that the Tokio task will be cancelled if the GPUI task is dropped
    #[allow(dead_code)]
    pub fn spawn<C, Fut, R>(cx: &C, f: Fut) -> Task<Result<R, JoinError>>
    where
        C: AppContext,
        Fut: Future<Output = R> + Send + 'static,
        R: Send + 'static,
    {
        cx.read_global(|tokio: &GlobalTokio, cx| {
            let join_handle = tokio.handle.spawn(f);
            let abort_handle = join_handle.abort_handle();
            let cancel = Defer(Some(move || {
                abort_handle.abort();
            }));
            cx.background_spawn(async move {
                let result = join_handle.await;
                drop(cancel);
                result
            })
        })
    }

    /// The runtime's handle.
    pub fn handle(cx: &App) -> tokio::runtime::Handle {
        GlobalTokio::global(cx).handle.clone()
    }
}

/// Runs its function when dropped, unless it already has.
struct Defer<F: FnOnce()>(Option<F>);

impl<F: FnOnce()> Drop for Defer<F> {
    fn drop(&mut self) {
        if let Some(f) = self.0.take() {
            f();
        }
    }
}
