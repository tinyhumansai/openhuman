use super::*;

#[test]
fn windows_comparison_normalizes_dos_verbatim_and_unc_paths() {
    assert_eq!(
        normalize_windows_path_for_comparison(r"\\?\C:\workspace\uploads\file.png"),
        r"C:\workspace\uploads\file.png"
    );
    assert_eq!(
        normalize_windows_path_for_comparison(r"\\?\UNC\server\share\workspace\uploads\file.png"),
        r"\\server\share\workspace\uploads\file.png"
    );
    assert_eq!(
        normalize_windows_path_for_comparison(r"C:\workspace\uploads\file.png"),
        r"C:\workspace\uploads\file.png"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn secure_open_rejects_parent_symlink_replaced_after_path_validation() {
    use std::os::unix::fs::symlink;

    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("workspace");
    let nested = root.join("nested");
    let outside = temp.path().join("outside");
    tokio::fs::create_dir_all(&nested).await.unwrap();
    tokio::fs::create_dir_all(&outside).await.unwrap();
    tokio::fs::write(nested.join("file.png"), b"allowed")
        .await
        .unwrap();
    tokio::fs::write(outside.join("file.png"), b"outside")
        .await
        .unwrap();

    let mut config = crate::config::Config::default();
    config.action_dir = root.clone();
    config.workspace_dir = temp.path().join("internal");
    let scope = AttachmentAccessScope {
        external_channel: false,
        workspace: Some(root.clone()),
    };
    let validated = crate::attachments::resolve_path(&config, "nested/file.png", &scope)
        .await
        .unwrap();
    let canonical_root = tokio::fs::canonicalize(&root).await.unwrap();

    tokio::fs::rename(&nested, root.join("nested-original"))
        .await
        .unwrap();
    symlink(&outside, &nested).unwrap();

    let error = secure_open(&validated, &canonical_root).unwrap_err();
    assert!(
        matches!(
            error.kind(),
            std::io::ErrorKind::TooManyLinks | std::io::ErrorKind::NotADirectory
        ),
        "parent symlink replacement must be refused, got {error}"
    );
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn fifo_attachment_is_rejected_without_waiting_for_a_writer() {
    use std::{
        ffi::CString,
        os::unix::ffi::OsStrExt,
        sync::atomic::{AtomicBool, Ordering},
        thread,
        time::Duration,
    };

    #[derive(Default)]
    struct NeverCalled(AtomicBool);

    #[async_trait]
    impl ChatModel<()> for NeverCalled {
        fn supports_input(&self, _: InputModality, _: &str, _: InputSource) -> bool {
            false
        }

        fn profile(&self) -> Option<&ModelProfile> {
            None
        }

        async fn invoke(
            &self,
            _: &(),
            _: ModelRequest,
        ) -> tinyinference_llm::Result<ModelResponse> {
            self.0.store(true, Ordering::SeqCst);
            Err(tinyinference_llm::Error::Model(
                "unexpected model invocation".into(),
            ))
        }
    }

    let temp = tempfile::tempdir().unwrap();
    let fifo = temp.path().join("attachment.png");
    let fifo_c = CString::new(fifo.as_os_str().as_bytes()).unwrap();
    let result = unsafe { libc::mkfifo(fifo_c.as_ptr(), 0o600) };
    assert_eq!(
        result,
        0,
        "mkfifo failed: {}",
        std::io::Error::last_os_error()
    );

    let mut config = Config::default();
    config.action_dir = temp.path().to_path_buf();
    config.workspace_dir = temp.path().join("internal");

    // The first path validation of a process loads the security module, which
    // can outlast the 2 s window below and has nothing to do with whether the
    // FIFO open blocks. Warm it on a regular file first, retrying while the
    // module is still loading, exactly as a module-backed test arms it.
    let warm = temp.path().join("warm.png");
    std::fs::write(&warm, b"warm").unwrap();
    let warm_scope = AttachmentAccessScope {
        external_channel: false,
        workspace: Some(temp.path().to_path_buf()),
    };
    let deadline = std::time::Instant::now() + Duration::from_secs(120);
    loop {
        match crate::attachments::resolve_path(&config, "warm.png", &warm_scope).await {
            Ok(_) => break,
            Err(error) if std::time::Instant::now() < deadline => {
                log::debug!("[attachments][test] module still loading: {error}");
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
            Err(error) => panic!("the security module must answer a path validation: {error}"),
        }
    }

    let model = Arc::new(NeverCalled::default());
    let wrapped = wrap_injected(
        &crate::core::runtime::DispatchCtx::host_default(),
        model.clone(),
        Arc::new(config),
    );
    let request = ModelRequest::new(vec![Message::User(
        tinyinference_llm::message::UserMessage {
            content: vec![ContentBlock::Image(ImageRef {
                url: "attachment.png".into(),
                mime_type: Some("image/png".into()),
            })],
        },
    )]);

    let fifo_for_unblock = fifo.clone();
    let mut handle = tokio::spawn(async move { wrapped.invoke(&(), request).await });
    let timely = tokio::time::timeout(Duration::from_secs(2), &mut handle).await;
    if timely.is_err() {
        // If a regression blocks in `openat(O_RDONLY)`, pair it with a writer
        // so the test process can still shut down cleanly after reporting it.
        let writer = thread::spawn(move || {
            let path = CString::new(fifo_for_unblock.as_os_str().as_bytes()).unwrap();
            for _ in 0..100 {
                let fd = unsafe {
                    libc::open(
                        path.as_ptr(),
                        libc::O_WRONLY | libc::O_NONBLOCK | libc::O_CLOEXEC,
                    )
                };
                if fd >= 0 {
                    unsafe { libc::close(fd) };
                    return;
                }
                thread::sleep(Duration::from_millis(10));
            }
        });
        let _ = handle.await;
        writer.join().unwrap();
        panic!("opening a FIFO attachment must not block");
    }
    let error = timely.unwrap().unwrap().unwrap_err();
    assert!(
        error
            .to_string()
            .contains("attachment exceeds configured file limit"),
        "non-regular attachment must be rejected before inference: {error}"
    );
    assert!(!model.0.load(Ordering::SeqCst));
}

#[tokio::test]
async fn oversized_local_file_marker_fails_before_primary_or_materialization() {
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[derive(Default)]
    struct NeverCalled(AtomicUsize);

    #[async_trait]
    impl ChatModel<()> for NeverCalled {
        fn supports_input(&self, _: InputModality, _: &str, _: InputSource) -> bool {
            false
        }

        fn profile(&self) -> Option<&ModelProfile> {
            None
        }

        async fn invoke(
            &self,
            _: &(),
            _: ModelRequest,
        ) -> tinyinference_llm::Result<ModelResponse> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Err(tinyinference_llm::Error::Model(
                "unexpected primary model invocation".into(),
            ))
        }
    }

    let temp = tempfile::tempdir().unwrap();
    let uploads = temp.path().join("uploads");
    tokio::fs::create_dir_all(&uploads).await.unwrap();
    tokio::fs::write(uploads.join("oversized.txt"), vec![b'x'; 1024 * 1024 + 1])
        .await
        .unwrap();

    let mut config = Config::default();
    config.action_dir = temp.path().to_path_buf();
    config.workspace_dir = temp.path().join("internal");
    config.multimodal_files.max_file_size_mb = 1;
    let primary = Arc::new(NeverCalled::default());
    let wrapped = wrap_injected(
        &crate::core::runtime::DispatchCtx::host_default(),
        primary.clone(),
        Arc::new(config),
    );
    let request = ModelRequest::new(vec![Message::User(
        tinyinference_llm::message::UserMessage {
            content: vec![ContentBlock::Text(
                "Please inspect [FILE:uploads/oversized.txt]".into(),
            )],
        },
    )]);

    let error = wrapped.invoke(&(), request).await.unwrap_err();
    assert!(error
        .to_string()
        .contains("attachment exceeds configured file limit"));
    assert_eq!(primary.0.load(Ordering::SeqCst), 0);
    assert!(!uploads.join("legacy-media").exists());
}
