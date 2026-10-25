use super::*;
use pegoles_protocol::{ActionRequest, ComputerAction, ComputerId};
use std::fs;
fn store() -> (tempfile::TempDir, HistoryStore) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().canonicalize().unwrap().join("history");
    (dir, HistoryStore::new(path))
}
fn task(title: &str) -> AgentTask {
    AgentTask::new(title)
}
fn summary(id: TaskId, text: &str) -> AgentEvent {
    AgentEvent::AgentMessage {
        task_id: id,
        kind: AgentMessageKind::Summary,
        text: text.into(),
        at: Utc::now(),
    }
}
#[test]
fn restart_restores_metadata_and_output_without_replaying_events_or_running() {
    let (_dir, store) = store();
    let mut history = History::default();
    let mut a = task("Make a table");
    a.status = TaskStatus::Running;
    let b = task("Other task");
    history.update_tasks(&[a.clone(), b.clone()]);
    history.record(&summary(a.id, "Name|Count\nApple|2"));
    store.write(&history, Utc::now()).unwrap();
    let recovered = store.read(Utc::now()).unwrap();
    assert_eq!(recovered.records.len(), 2);
    let bus = crate::EventBus::new();
    let mut rx = bus.subscribe();
    let mut manager = crate::TaskManager::new(bus);
    manager.restore_history(&recovered.tasks()).unwrap();
    assert_eq!(manager.get(&a.id).unwrap().status, TaskStatus::Cancelled);
    assert!(rx.try_recv().is_err());
    assert_eq!(
        recovered
            .detail(a.id)
            .unwrap()
            .artifact
            .as_ref()
            .unwrap()
            .task_id,
        a.id
    );
    assert!(recovered.detail(b.id).unwrap().artifact.is_none());
    assert!(matches!(
        recovered
            .detail(a.id)
            .unwrap()
            .artifact
            .as_ref()
            .unwrap()
            .data,
        ArtifactData::Table { .. }
    ));
}
#[test]
fn audit_excludes_typed_keys_diagnostics_and_sensitive_output() {
    let (_dir, store) = store();
    let mut history = History::default();
    let a = task("Fill the form");
    history.update_tasks(std::slice::from_ref(&a));
    for (text, sensitive) in [
        ("ordinary typed payload", false),
        ("private typed payload", true),
    ] {
        let request = ActionRequest::new(
            a.id,
            ComputerId::new(),
            ComputerAction::TypeText {
                text: text.into(),
                sensitive,
            },
        );
        history.record(&AgentEvent::ActionRequested { request });
    }
    history.record(&summary(a.id, "private typed payload"));
    history.record(&AgentEvent::AgentMessage {
        task_id: a.id,
        kind: AgentMessageKind::Error,
        text: "diagnostic payload".into(),
        at: Utc::now(),
    });
    store.write(&history, Utc::now()).unwrap();
    let bytes = fs::read_to_string(store.path()).unwrap();
    for excluded in [
        "ordinary typed payload",
        "private typed payload",
        "diagnostic payload",
        "ActionRequest",
        "type_text",
    ] {
        assert!(!bytes.contains(excluded), "{excluded}");
    }
    assert!(store
        .read(Utc::now())
        .unwrap()
        .detail(a.id)
        .unwrap()
        .artifact
        .is_none());
    let mut history = History::default();
    history.update_tasks(std::slice::from_ref(&a));
    history.record(&summary(a.id, "password=hunter2"));
    assert!(history.detail(a.id).unwrap().artifact.is_none());
}
#[test]
fn count_age_and_serialized_byte_retention_have_exact_boundaries() {
    let now = Utc::now();
    let mut h = History::default();
    let tasks: Vec<_> = (0..MAX_TASKS + 1)
        .map(|i| task(&format!("task {i}")))
        .collect();
    h.update_tasks(&tasks);
    h.retain(now).unwrap();
    assert_eq!(h.records.len(), MAX_TASKS);
    let mut a = task("At age boundary");
    a.created_at = now - chrono::Duration::days(MAX_AGE_DAYS);
    a.updated_at = a.created_at;
    let mut b = a.clone();
    b.id = TaskId::new();
    b.created_at -= chrono::Duration::milliseconds(1);
    b.updated_at = b.created_at;
    let mut h = History::default();
    h.update_tasks(&[a.clone(), b]);
    h.retain(now).unwrap();
    assert_eq!(h.tasks(), vec![a]);
    let mut h = History::default();
    let tasks: Vec<_> = (0..MAX_TASKS).map(|_| task(&"🪩".repeat(500))).collect();
    h.update_tasks(&tasks);
    for a in tasks {
        h.record(&summary(a.id, &"🪩".repeat(MAX_RESULT_CHARS)));
    }
    assert!(serde_json::to_vec(&h).unwrap().len() > MAX_HISTORY_BYTES);
    h.retain(now).unwrap();
    assert!(serde_json::to_vec(&h).unwrap().len() <= MAX_HISTORY_BYTES);
    assert!(h.records.len() < MAX_TASKS);
}
#[test]
fn corrupt_oversize_unknown_fields_and_disk_failures_preserve_original() {
    let (_dir, store) = store();
    fs::create_dir_all(store.path().parent().unwrap()).unwrap();
    for bytes in [
        b"corrupt".to_vec(),
        vec![b' '; MAX_HISTORY_BYTES + 1],
        br#"{"schema_version":1,"records":[],"path":"/etc/passwd"}"#.to_vec(),
    ] {
        fs::write(store.path(), &bytes).unwrap();
        assert!(store.read(Utc::now()).is_err());
        assert!(store.write(&History::default(), Utc::now()).is_err());
        assert_eq!(fs::read(store.path()).unwrap(), bytes);
    }
    fs::remove_file(store.path()).unwrap();
    fs::create_dir(store.path()).unwrap();
    assert!(store.write(&History::default(), Utc::now()).is_err());
}
#[cfg(unix)]
#[test]
fn symlinks_are_refused_and_modes_are_private() {
    use std::os::unix::fs::{symlink, PermissionsExt};
    let (dir, store) = store();
    store.write(&History::default(), Utc::now()).unwrap();
    assert_eq!(
        fs::metadata(store.path()).unwrap().permissions().mode() & 0o777,
        0o600
    );
    assert_eq!(
        fs::metadata(store.path().parent().unwrap())
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o700
    );
    assert_eq!(
        fs::metadata(store.path().parent().unwrap().join(".tasks.lock"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    let outside = dir.path().join("outside");
    fs::write(&outside, "untouched").unwrap();
    fs::remove_file(store.path()).unwrap();
    symlink(&outside, store.path()).unwrap();
    assert!(store.read(Utc::now()).is_err());
    assert!(store.write(&History::default(), Utc::now()).is_err());
    assert_eq!(fs::read_to_string(outside).unwrap(), "untouched");
    fs::remove_file(store.path()).unwrap();
    fs::remove_file(store.path().parent().unwrap().join(".tasks.lock")).unwrap();
    fs::remove_dir(store.path().parent().unwrap()).unwrap();
    symlink(dir.path(), store.path().parent().unwrap()).unwrap();
    assert!(store.read(Utc::now()).is_err());
}
#[test]
fn sensitive_objectives_suppress_even_an_unlabelled_echo() {
    let mut h = History::default();
    let a = task("The password is sentinel_value_725");
    h.update_tasks(std::slice::from_ref(&a));
    h.record(&summary(a.id, "sentinel_value_725"));
    let r = h.detail(a.id).unwrap();
    assert_eq!(r.task.title, "Private task");
    assert!(r.artifact.is_none());
}
#[test]
fn strict_nested_schema_and_artifact_association_are_validated() {
    let (_dir, store) = store();
    let mut h = History::default();
    let a = task("Report");
    h.update_tasks(std::slice::from_ref(&a));
    h.record(&summary(a.id, "Result"));
    store.write(&h, Utc::now()).unwrap();
    let original = serde_json::to_value(&h).unwrap();
    for kind in [
        "task_extra",
        "audit_extra",
        "wrong_task",
        "oversized_text",
        "unsupported_version",
    ] {
        let mut v = original.clone();
        match kind {
            "task_extra" => v["records"][0]["task"]["path"] = "/etc/passwd".into(),
            "audit_extra" => {
                v["records"][0]["audit"] =
                    serde_json::json!([{"status":"running","at":Utc::now(),"text":"typed"}])
            }
            "wrong_task" => {
                v["records"][0]["artifact"]["task_id"] = TaskId::new().to_string().into()
            }
            "oversized_text" => {
                v["records"][0]["artifact"]["data"]["text"] =
                    "x".repeat(MAX_RESULT_CHARS + 1).into()
            }
            _ => v["schema_version"] = 2.into(),
        }
        fs::write(store.path(), serde_json::to_vec(&v).unwrap()).unwrap();
        assert!(store.read(Utc::now()).is_err(), "{kind}");
    }
}
#[test]
fn byte_read_cap_accepts_exact_limit_and_rejects_one_more() {
    let (_dir, store) = store();
    store.write(&History::default(), Utc::now()).unwrap();
    let mut bytes = fs::read(store.path()).unwrap();
    bytes.resize(MAX_HISTORY_BYTES, b' ');
    fs::write(store.path(), &bytes).unwrap();
    assert!(store.read(Utc::now()).is_ok());
    bytes.push(b' ');
    fs::write(store.path(), bytes).unwrap();
    assert!(store.read(Utc::now()).is_err());
}
#[test]
fn restore_validates_entire_batch_before_mutating_and_all_open_states_are_inert() {
    let bus = crate::EventBus::new();
    let mut rx = bus.subscribe();
    let mut manager = crate::TaskManager::new(bus);
    let a = task("Valid");
    let mut duplicate = a.clone();
    duplicate.title = "Different".into();
    assert!(manager.restore_history(&[a.clone(), duplicate]).is_err());
    assert!(manager.list().is_empty());
    let mut invalid = a.clone();
    invalid.updated_at = invalid.created_at - chrono::Duration::seconds(1);
    assert!(manager.restore_history(&[a.clone(), invalid]).is_err());
    assert!(manager.list().is_empty());
    for status in [
        TaskStatus::Pending,
        TaskStatus::Running,
        TaskStatus::WaitingForApproval,
    ] {
        let mut a = task("Interrupted");
        a.status = status;
        manager.restore_history(std::slice::from_ref(&a)).unwrap();
        assert_eq!(manager.get(&a.id).unwrap().status, TaskStatus::Cancelled);
    }
    assert!(rx.try_recv().is_err());
}
#[test]
fn results_after_a_secret_pattern_summary_remain_suppressed() {
    let mut h = History::default();
    let a = task("Form");
    h.update_tasks(std::slice::from_ref(&a));
    h.record(&summary(a.id, "password: sentinel_99"));
    h.record(&summary(a.id, "sentinel_99"));
    assert!(h.detail(a.id).unwrap().artifact.is_none());
}
#[test]
fn taint_survives_retention_eviction_of_an_active_task() {
    let mut h = History::default();
    let mut a = task("Running form");
    a.status = TaskStatus::Running;
    a.updated_at -= chrono::Duration::seconds(1);
    a.created_at = a.updated_at;
    h.update_tasks(std::slice::from_ref(&a));
    h.record(&AgentEvent::ActionRequested {
        request: ActionRequest::new(
            a.id,
            ComputerId::new(),
            ComputerAction::TypeText {
                text: "sensitive sentinel".into(),
                sensitive: true,
            },
        ),
    });
    h.update_tasks(
        &(0..MAX_TASKS)
            .map(|_| task("New pending task"))
            .collect::<Vec<_>>(),
    );
    h.retain(Utc::now()).unwrap();
    assert!(h.detail(a.id).is_none());
    a.updated_at = Utc::now();
    h.update_tasks(std::slice::from_ref(&a));
    h.record(&summary(a.id, "sensitive sentinel"));
    assert!(h.detail(a.id).unwrap().artifact.is_none());
}

#[test]
fn reopening_discards_crash_left_snapshots_with_bounded_repeated_replacements() {
    let (_dir, store) = store();
    let now = Utc::now();
    let mut h = History::default();
    let a = task("Current objective");
    h.update_tasks(std::slice::from_ref(&a));
    store.write(&h, now).unwrap();
    let canonical = fs::read(store.path()).unwrap();
    let scratch = store.path().parent().unwrap().join(".tasks.tmp");
    let mut orphan = History::default();
    let mut expired = task("Expired private objective");
    expired.created_at = now - chrono::Duration::days(31);
    expired.updated_at = expired.created_at;
    expired.status = TaskStatus::Completed;
    orphan.update_tasks(std::slice::from_ref(&expired));
    orphan.record(&summary(expired.id, "Expired private result"));
    let mut orphan_bytes = serde_json::to_vec(&orphan).unwrap();
    orphan_bytes.resize(MAX_HISTORY_BYTES, b' ');
    for _ in 0..10 {
        // A complete private snapshot left between write and rename by a crash.
        fs::write(&scratch, &orphan_bytes).unwrap();
        assert!(
            fs::read_dir(store.path().parent().unwrap())
                .unwrap()
                .map(|e| e.unwrap().metadata().unwrap().len())
                .sum::<u64>()
                <= (2 * MAX_HISTORY_BYTES) as u64
        );
        let reopened = HistoryStore::new(store.path().parent().unwrap().to_path_buf());
        assert_eq!(reopened.read(now).unwrap().tasks().len(), 1);
        assert!(
            !scratch.exists(),
            "uncommitted private bytes must leave disk on reopen"
        );
        assert_eq!(fs::read(store.path()).unwrap(), canonical);
        let entries: Vec<_> = fs::read_dir(store.path().parent().unwrap())
            .unwrap()
            .map(Result::unwrap)
            .collect();
        assert!(
            entries.len() <= 2,
            "only canonical snapshot and empty lock remain"
        );
        let bytes: u64 = entries.iter().map(|e| e.metadata().unwrap().len()).sum();
        assert!(bytes <= MAX_HISTORY_BYTES as u64);
    }
}

#[test]
fn recovery_removes_only_recognized_legacy_stages_and_preserves_corrupt_canonical() {
    let (_dir, store) = store();
    fs::create_dir_all(store.path().parent().unwrap()).unwrap();
    let parent = store.path().parent().unwrap().to_path_buf();
    let legacy = parent.join(format!(".tasks-{}.tmp", uuid::Uuid::new_v4()));
    let unrelated = [
        parent.join("notes.txt"),
        parent.join(".tasks-not-a-uuid.tmp"),
        parent.join(".tasks.tmp.backup"),
    ];
    for path in &unrelated {
        fs::write(path, "unrelated").unwrap();
    }
    fs::write(&legacy, "expired private snapshot").unwrap();
    fs::write(store.path(), "corrupt canonical preserved").unwrap();
    assert!(store.read(Utc::now()).is_err());
    assert!(
        !legacy.exists(),
        "legacy crash snapshots must not escape retention"
    );
    assert_eq!(
        fs::read_to_string(store.path()).unwrap(),
        "corrupt canonical preserved"
    );
    assert!(store.write(&History::default(), Utc::now()).is_err());
    assert_eq!(
        fs::read_to_string(store.path()).unwrap(),
        "corrupt canonical preserved"
    );
    for path in unrelated {
        assert_eq!(fs::read_to_string(path).unwrap(), "unrelated");
    }
}

#[test]
fn a_live_store_lock_prevents_cleanup_or_another_writer_from_touching_its_stage() {
    let (_dir, store) = store();
    store.write(&History::default(), Utc::now()).unwrap();
    let parent = store.path().parent().unwrap().to_path_buf();
    let lock = private_options()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(parent.join(".tasks.lock"))
        .unwrap();
    lock.try_lock().unwrap();
    let scratch = parent.join(".tasks.tmp");
    fs::write(&scratch, "active writer private bytes").unwrap();
    let original = fs::read(store.path()).unwrap();
    assert!(
        store.read(Utc::now()).is_err(),
        "read must not clean an active writer's stage"
    );
    assert!(store.write(&History::default(), Utc::now()).is_err());
    assert_eq!(
        fs::read_to_string(&scratch).unwrap(),
        "active writer private bytes"
    );
    assert_eq!(fs::read(store.path()).unwrap(), original);
    assert_eq!(
        fs::read_dir(&parent).unwrap().count(),
        3,
        "a competing write cannot create a random stage"
    );
    drop(lock);
    assert!(store.read(Utc::now()).is_ok());
    assert!(!scratch.exists());
}

#[cfg(unix)]
#[test]
fn recovery_never_follows_staging_or_lock_symlinks_or_removes_nonregular_entries() {
    use std::os::unix::fs::symlink;
    let (dir, store) = store();
    store.write(&History::default(), Utc::now()).unwrap();
    let outside = dir.path().join("outside");
    fs::write(&outside, "untouched outside bytes").unwrap();
    let parent = store.path().parent().unwrap().to_path_buf();
    for name in [
        ".tasks.tmp".to_string(),
        format!(".tasks-{}.tmp", uuid::Uuid::new_v4()),
        ".tasks.lock".to_string(),
    ] {
        let path = parent.join(name);
        if path.exists() {
            fs::remove_file(&path).unwrap();
        }
        symlink(&outside, &path).unwrap();
        assert!(store.read(Utc::now()).is_err());
        assert!(store.write(&History::default(), Utc::now()).is_err());
        assert_eq!(
            fs::read_to_string(&outside).unwrap(),
            "untouched outside bytes"
        );
        assert!(fs::symlink_metadata(&path)
            .unwrap()
            .file_type()
            .is_symlink());
        fs::remove_file(path).unwrap();
    }
    fs::create_dir(parent.join(".tasks.tmp")).unwrap();
    assert!(store.read(Utc::now()).is_err());
    assert!(parent.join(".tasks.tmp").is_dir());
}

#[test]
fn recovery_directory_scan_has_an_exact_finite_limit_and_leaves_unrelated_files() {
    let (_dir, store) = store();
    store.write(&History::default(), Utc::now()).unwrap();
    let parent = store.path().parent().unwrap().to_path_buf();
    // Canonical snapshot + permanent empty lock + 254 unrelated files = 256.
    for i in 0..254 {
        fs::write(parent.join(format!("unrelated-{i}")), "x").unwrap();
    }
    assert!(store.read(Utc::now()).is_ok());
    fs::write(parent.join("one-too-many"), "x").unwrap();
    let canonical = fs::read(store.path()).unwrap();
    assert!(store.read(Utc::now()).is_err());
    assert!(store.write(&History::default(), Utc::now()).is_err());
    assert_eq!(fs::read(store.path()).unwrap(), canonical);
    assert_eq!(fs::read_dir(parent).unwrap().count(), 257);
}

#[test]
fn human_takeover_taints_active_task_even_when_latest_task_snapshot_is_terminal() {
    let mut history = History::default();
    let mut a = task("manual assistance");
    a.status = TaskStatus::Completed;
    history.update_tasks(&[a.clone()]);
    history.record(&AgentEvent::TaskStatusChanged {
        task_id: a.id,
        from: TaskStatus::Pending,
        to: TaskStatus::Running,
        at: Utc::now(),
    });
    history.record(&AgentEvent::ControlOwnershipChanged {
        computer_id: ComputerId::new(),
        from: pegoles_protocol::ControlOwner::None,
        to: pegoles_protocol::ControlOwner::User,
        at: Utc::now(),
    });
    history.record(&summary(a.id, "opaque manual sentinel"));
    assert!(history.detail(a.id).unwrap().artifact.is_none());
}
