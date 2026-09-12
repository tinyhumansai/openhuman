use serde_json::json;
use tempfile::TempDir;
use tinyflows::graph::ids::NodeId;
use tinyflows::graph::Checkpointer;
use tinyflows_sqlite::checkpoint::SqliteCheckpointer;

/// The reason this file exists. The port is a retarget of the backend
/// `tinyagents` shipped, and the two must agree on the bytes on disk: an
/// existing `<workspace>/flows/checkpoints.db` is read and written by this
/// code after the upgrade, and a divergence would surface as an interrupted
/// flow that cannot resume — at the worst possible moment, on a user's
/// machine, with the run already half-done.
///
/// Comparing the DDL is the cheapest total statement of that: same tables,
/// same columns, same primary keys, same indexes.
#[test]
fn schema_is_identical_to_the_backend_it_replaced() {
    assert_eq!(
        SqliteCheckpointer::<serde_json::Value>::schema_sql(),
        tinyagents_graph::SqliteCheckpointer::<serde_json::Value>::schema_sql(),
        "the ported schema drifted from the tinyagents backend that wrote every \
     checkpoints.db in the field — an existing database would stop resuming"
    );
}

/// A database written by the backend this replaced must be readable here, and
/// the schema check above is a statement about DDL rather than about
/// behaviour. This writes through the old type and reads back through the new
/// one, against one file.
#[tokio::test]
async fn reads_a_database_written_by_the_previous_backend() {
    let tmp = TempDir::new().unwrap();
    let db = tmp.path().join("checkpoints.db");

    // Fully qualified: the two `Checkpointer` traits share a method set, so
    // importing both here would make every call in this file ambiguous.
    use tinyagents_graph::Checkpointer as LegacyCheckpointer;

    let old = tinyagents_graph::SqliteCheckpointer::<serde_json::Value>::open(&db).unwrap();
    let written = tinyagents_graph::Checkpoint {
        thread_id: "flow:f1:run-a".to_string(),
        checkpoint_id: "cp-1".to_string(),
        run_id: Some("run-1".to_string()),
        parent_checkpoint_id: None,
        namespace: Vec::new(),
        state: json!({ "counter": 7 }),
        next_nodes: vec![tinyagents_harness::ids::NodeId::new("next")],
        completed_tasks: vec![tinyagents_harness::ids::NodeId::new("done")],
        pending_writes: Vec::new(),
        interrupts: Vec::new(),
        pending_activations: None,
        barrier_arrivals: Vec::new(),
        metadata: json!({ "source": "loop", "step": 3 }),
    };
    LegacyCheckpointer::put(&old, written).await.unwrap();
    drop(old);

    let new = SqliteCheckpointer::<serde_json::Value>::open(&db).unwrap();
    let read = new
        .get("flow:f1:run-a", None)
        .await
        .unwrap()
        .expect("the pre-upgrade checkpoint must still load");
    assert_eq!(read.checkpoint_id, "cp-1");
    assert_eq!(read.state, json!({ "counter": 7 }));
    assert_eq!(read.next_nodes, vec![NodeId::new("next")]);
    assert_eq!(read.metadata["step"], 3);
}
